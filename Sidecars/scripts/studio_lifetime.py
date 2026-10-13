"""Keep one studio per lock path without trusting recycled process IDs."""
import os
from pathlib import Path

_locks = []


def claim_window_lock(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    lock = path.open("a+b")
    if lock.tell() == 0:
        lock.write(b"\0")
        lock.flush()
    lock.seek(0)
    try:
        if os.name == "nt":
            import msvcrt
            msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        lock.close()
        raise SystemExit(0)  # The existing studio remains the owner.
    _locks.append(lock)  # The kernel releases ownership even after a crash.
