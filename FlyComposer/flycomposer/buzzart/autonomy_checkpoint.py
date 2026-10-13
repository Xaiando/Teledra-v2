"""Transactional checkpoints and an OS-held lease for local training.

A checkpoint becomes visible only when its complete bundle is committed and
the small pointer is atomically replaced. No process identity is trusted as a
substitute for the operating system's live file lock.
"""
from __future__ import annotations

from contextlib import contextmanager
import hashlib
import io
import json
import os
from pathlib import Path
import re
import time
import uuid

import numpy as np


_ID = re.compile(r"^[0-9]{20}-[0-9a-f]{32}$")
_FILES = frozenset(('state.json', 'arrays.npz', 'manifest.json'))


def _json_bytes(value):
    return (json.dumps(value, ensure_ascii=False, allow_nan=False,
                       sort_keys=True, separators=(',', ':')) + '\n').encode('utf-8')


def _sha(data):
    return hashlib.sha256(data).hexdigest()


def _write_new(path, data):
    with Path(path).open('xb') as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def atomic_json(path, value):
    """Atomically replace JSON, tolerating brief Windows reader sharing locks."""
    path = Path(path)
    data = _json_bytes(value)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + '.' + uuid.uuid4().hex + '.next')
    try:
        _write_new(temporary, data)
        deadline, delay = time.monotonic() + 2., .01
        while True:
            try:
                os.replace(temporary, path)
                break
            except PermissionError as error:
                if getattr(error, 'winerror', None) not in (5, 32, 33):
                    raise
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise
                # Keep the flushed temporary intact while readers release the
                # destination. The previous JSON remains valid until replacement.
                time.sleep(min(delay, remaining))
                delay = min(.2, delay * 2.)
    finally:
        if temporary.exists():
            temporary.unlink()


def _checked_arrays(arrays):
    if not isinstance(arrays, dict):
        raise TypeError('Checkpoint arrays must be a dictionary')
    copied = {}
    for name, value in arrays.items():
        if not isinstance(name, str) or not name or not re.fullmatch(r'[A-Za-z0-9_-]+', name):
            raise ValueError('Checkpoint array names must be simple nonempty identifiers')
        if not isinstance(value, np.ndarray):
            raise TypeError(f'Checkpoint array {name} must be an ndarray')
        if value.dtype.hasobject or value.dtype.kind not in 'biufc':
            raise ValueError(f'Checkpoint array {name} must have a numeric or boolean dtype')
        array = np.array(value, copy=True)
        if not np.isfinite(array).all():
            raise ValueError(f'Checkpoint array {name} contains nonfinite values')
        copied[name] = array
    return copied


def _child(parent, name):
    """Resolve a direct child before reading, replacing or removing it."""
    parent = Path(parent).resolve()
    child = parent / name
    if child.is_symlink() or child.resolve().parent != parent:
        raise ValueError('Checkpoint path escapes its parent directory')
    return child


def _prune(bundles, current):
    # Only recognizable, complete bundle directories are eligible for cleanup.
    # Unexpected files or links are left alone instead of recursively removed.
    candidates = []
    for entry in bundles.iterdir():
        if not _ID.fullmatch(entry.name) or entry.is_symlink() or not entry.is_dir():
            continue
        checked = _child(bundles, entry.name)
        contents = list(checked.iterdir())
        if {p.name for p in contents} != _FILES or any(p.is_symlink() or not p.is_file() for p in contents):
            continue
        candidates.append(checked)
    keep = {p.name for p in sorted(candidates, key=lambda p: p.name, reverse=True)[:4]}
    keep.add(current)
    for candidate in candidates:
        if candidate.name in keep:
            continue
        checked = _child(bundles, candidate.name)
        for filename in _FILES:
            _child(checked, filename).unlink()
        checked.rmdir()


def save_checkpoint(run_dir, state: dict, arrays: dict[str, np.ndarray]) -> str:
    """Commit an immutable state/array bundle, then retain the latest four.

    Callers own configuration and RNG contents and should hold ``run_lock``
    around their training session. A failed pointer update preserves the last
    committed checkpoint. Orphan bundles are harmless and never auto-selected.
    """
    if not isinstance(state, dict):
        raise TypeError('Checkpoint state must be a dictionary')
    state_data = _json_bytes(state)
    copied = _checked_arrays(arrays)
    buffer = io.BytesIO()
    np.savez(buffer, **copied)
    array_data = buffer.getvalue()
    root = Path(run_dir).resolve()
    root.mkdir(parents=True, exist_ok=True)
    bundles = _child(root, 'checkpoints')
    bundles.mkdir(exist_ok=True)
    checkpoint_id = f'{time.time_ns():020d}-{uuid.uuid4().hex}'
    pending = _child(bundles, '.pending-' + checkpoint_id)
    destination = _child(bundles, checkpoint_id)
    pending.mkdir()
    manifest = {'version': 1, 'checkpoint_id': checkpoint_id,
                'files': {'state.json': _sha(state_data), 'arrays.npz': _sha(array_data)},
                'arrays': {name: {'dtype': value.dtype.str, 'shape': list(value.shape)}
                           for name, value in copied.items()}}
    manifest_data = _json_bytes(manifest)
    _write_new(pending / 'state.json', state_data)
    _write_new(pending / 'arrays.npz', array_data)
    _write_new(pending / 'manifest.json', manifest_data)
    os.replace(pending, destination)
    atomic_json(root / 'checkpoint.json', {'version': 1, 'checkpoint_id': checkpoint_id,
                                          'manifest_sha256': _sha(manifest_data)})
    _prune(bundles, checkpoint_id)
    return checkpoint_id


def load_checkpoint(run_dir):
    """Load only the committed pointer; reject incomplete or altered bundles."""
    root = Path(run_dir).resolve()
    pointer = json.loads(_child(root, 'checkpoint.json').read_bytes())
    checkpoint_id = pointer.get('checkpoint_id')
    if pointer.get('version') != 1 or not isinstance(checkpoint_id, str) or not _ID.fullmatch(checkpoint_id):
        raise ValueError('Invalid checkpoint pointer')
    bundles = _child(root, 'checkpoints')
    bundle = _child(bundles, checkpoint_id)
    manifest_data = _child(bundle, 'manifest.json').read_bytes()
    if _sha(manifest_data) != pointer.get('manifest_sha256'):
        raise ValueError('Checkpoint manifest hash mismatch')
    manifest = json.loads(manifest_data)
    if manifest.get('version') != 1 or manifest.get('checkpoint_id') != checkpoint_id:
        raise ValueError('Invalid checkpoint manifest')
    files = manifest.get('files')
    if not isinstance(files, dict) or set(files) != {'state.json', 'arrays.npz'}:
        raise ValueError('Invalid checkpoint file inventory')
    data = {}
    for name, digest in files.items():
        data[name] = _child(bundle, name).read_bytes()
        if _sha(data[name]) != digest:
            raise ValueError(f'Checkpoint hash mismatch: {name}')
    state = json.loads(data['state.json'])
    if not isinstance(state, dict):
        raise ValueError('Invalid checkpoint state')
    with np.load(io.BytesIO(data['arrays.npz']), allow_pickle=False) as archive:
        arrays = _checked_arrays({name: archive[name] for name in archive.files})
    expected = {name: {'dtype': value.dtype.str, 'shape': list(value.shape)}
                for name, value in arrays.items()}
    if expected != manifest.get('arrays'):
        raise ValueError('Checkpoint array inventory mismatch')
    return state, arrays


@contextmanager
def run_lock(run_dir):
    """Hold a nonblocking OS lock until this context exits or process dies."""
    root = Path(run_dir).resolve()
    root.mkdir(parents=True, exist_ok=True)
    path = _child(root, 'run.lock')
    fd = os.open(path, os.O_RDWR | os.O_CREAT, 0o600)
    locked = False
    try:
        if os.fstat(fd).st_size == 0:
            os.write(fd, b'0')
        os.lseek(fd, 0, os.SEEK_SET)
        try:
            if os.name == 'nt':
                import msvcrt
                msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            locked = True
        except OSError as error:
            raise RuntimeError('Another training process holds this run lock') from error
        yield
    finally:
        if locked:
            os.lseek(fd, 0, os.SEEK_SET)
            if os.name == 'nt':
                import msvcrt
                msvcrt.locking(fd, msvcrt.LK_UNLCK, 1)
            else:
                import fcntl
                fcntl.flock(fd, fcntl.LOCK_UN)
        os.close(fd)
