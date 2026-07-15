"""Durable, artifact-bound back workshop for Court Synth.

The front-stage score, renderer state, and WAV are never mutated here.  A
human ``*work_on_it`` feedback event creates one immutable base snapshot and a
four-pass refinement job.  Each completed pass is validated and rendered in a
private staging directory before that entire candidate directory is renamed
into place and referenced from the atomically replaced ``job.json``.

This module is both a Python API and a JSON-speaking CLI for the Rust court.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import sys
import time
import uuid
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterator, Mapping

if __package__ in {None, ""}:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from court_synth import feedback as court_feedback


PROJECT_DIR = Path(__file__).resolve().parent
DEFAULT_WORKSHOP_DIR = PROJECT_DIR / "feedback" / "workshops"
JOB_SCHEMA_VERSION = 1
JOB_KIND = "court_synth_workshop_job"
PASS_COUNT = 4
ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,159}$")
HASH_RE = re.compile(r"^[0-9a-f]{64}$")

PASS_PLAN: tuple[dict[str, str], ...] = (
    {
        "focus": "harmonic_coherence",
        "objective": (
            "Refine functional chord motion, consonance, and voice leading while "
            "preserving the keeper's recognizable tonal identity."
        ),
    },
    {
        "focus": "groove_and_pulse",
        "objective": (
            "Strengthen the steady beat, bass-and-kick lock, phrase rhythm, and "
            "controlled syncopation without erasing the keeper's motif."
        ),
    },
    {
        "focus": "arrangement_arc",
        "objective": (
            "Develop sections, contrast, transitions, dynamics, and motif returns "
            "into a coherent long-form progression."
        ),
    },
    {
        "focus": "mix_and_loop",
        "objective": (
            "Polish balance, space, width, headroom, and the seamless loop while "
            "keeping the musical decisions from the earlier passes intact."
        ),
    },
)


class WorkshopError(RuntimeError):
    """Base class for durable workshop failures."""


class WorkshopBindingError(WorkshopError):
    """Feedback or candidate bytes do not match their declared artifacts."""


class WorkshopStateError(WorkshopError):
    """A requested job transition is stale or invalid."""


def _now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _renderer_score_sha256(score: Mapping[str, Any]) -> str:
    return hashlib.sha256(json.dumps(score, sort_keys=True).encode("utf-8")).hexdigest()


def _protected_identity(score: Mapping[str, Any]) -> dict[str, Any]:
    transport = score.get("transport", {})
    harmony = score.get("harmony", {})
    return {
        "schema_version": score.get("schema_version"),
        "project_id": score.get("project_id"),
        "style": score.get("style"),
        "transport": {
            key: transport.get(key) for key in ("bpm", "meter", "bars", "swing", "loop")
        },
        "harmony": {
            "tonic": harmony.get("tonic"),
            "mode": harmony.get("mode"),
            "chords": harmony.get("chords"),
        },
        "motif": score.get("motif"),
        "manual_notes": score.get("manual_notes", []),
    }


def _audible_fingerprint(score: Mapping[str, Any]) -> str:
    audible = _json_copy(score)
    for key in ("revision", "title", "lineage", "editor_notes"):
        audible.pop(key, None)
    return _renderer_score_sha256(audible)


def _validate_pass_delta(
    candidate: Mapping[str, Any],
    parent: Mapping[str, Any],
    pass_index: int,
) -> None:
    seed_changed = candidate.get("seed") != parent.get("seed")
    sections_changed = candidate.get("sections") != parent.get("sections")
    mix_changed = (
        candidate.get("mix") != parent.get("mix")
        or candidate.get("track_mix") != parent.get("track_mix")
    )
    if pass_index <= 3:
        if not seed_changed:
            raise WorkshopBindingError(
                f"pass {pass_index} must generate a new random 'seed' to prove meaningful musical iteration"
            )
        if not sections_changed:
            raise WorkshopBindingError(
                f"pass {pass_index} must change 'sections' from the parent"
            )
        if mix_changed:
            raise WorkshopBindingError(
                f"pass {pass_index} cannot spend its focused arrangement pass on mix changes"
            )
    else:
        if not mix_changed:
            raise WorkshopBindingError("pass 4 must make a real mix or track_mix change")
        if seed_changed or sections_changed:
            raise WorkshopBindingError(
                "pass 4 must preserve seed/sections from the accepted arrangement passes"
            )


def _json_bytes(value: Any) -> bytes:
    return (json.dumps(value, indent=2, ensure_ascii=False) + "\n").encode("utf-8")


def _json_copy(value: Any) -> Any:
    return json.loads(json.dumps(value, ensure_ascii=False))


def _read_json(path: Path, label: str) -> tuple[bytes, dict[str, Any]]:
    try:
        raw = Path(path).read_bytes()
        value = json.loads(raw.decode("utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise WorkshopBindingError(f"could not read {label} at {path}: {exc}") from exc
    if not isinstance(value, dict):
        raise WorkshopBindingError(f"{label} root must be a JSON object")
    return raw, value


def _write_fsynced(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb") as handle:
        handle.write(data)
        handle.flush()
        os.fsync(handle.fileno())


def _atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.parent / f".{path.name}.{uuid.uuid4().hex}.tmp"
    try:
        _write_fsynced(temporary, _json_bytes(value))
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def _validate_event_id(event_id: str) -> str:
    event_id = str(event_id)
    if not ID_RE.fullmatch(event_id):
        raise WorkshopBindingError(f"unsafe workshop event_id {event_id!r}")
    return event_id


def _job_dir(root: Path, event_id: str) -> Path:
    return Path(root).resolve() / "jobs" / _validate_event_id(event_id)


def _job_path(root: Path, event_id: str) -> Path:
    return _job_dir(root, event_id) / "job.json"


@contextmanager
def _store_lock(root: Path, timeout: float = 10.0) -> Iterator[None]:
    """Cross-process exclusive lock for claims and job-state transitions."""
    root = Path(root).resolve()
    root.mkdir(parents=True, exist_ok=True)
    lock_path = root / ".workshop.lock"
    handle = lock_path.open("a+b")
    handle.seek(0, os.SEEK_END)
    if handle.tell() == 0:
        handle.write(b"0")
        handle.flush()
    deadline = time.monotonic() + timeout
    locked = False
    try:
        while not locked:
            try:
                handle.seek(0)
                if os.name == "nt":
                    import msvcrt

                    msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
                else:
                    import fcntl

                    fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                locked = True
            except (OSError, BlockingIOError):
                if time.monotonic() >= deadline:
                    raise WorkshopStateError("timed out waiting for the workshop store lock")
                time.sleep(0.025)
        yield
    finally:
        if locked:
            try:
                handle.seek(0)
                if os.name == "nt":
                    import msvcrt

                    msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
                else:
                    import fcntl

                    fcntl.flock(handle.fileno(), fcntl.LOCK_UN)
            except OSError:
                pass
        handle.close()


def _load_job_path(path: Path) -> dict[str, Any]:
    try:
        job = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise WorkshopStateError(f"invalid workshop job {path}: {exc}") from exc
    if not isinstance(job, dict) or job.get("kind") != JOB_KIND:
        raise WorkshopStateError(f"{path} is not a Court Synth workshop job")
    if job.get("schema_version") != JOB_SCHEMA_VERSION:
        raise WorkshopStateError(f"unsupported workshop schema in {path}")
    if job.get("event_id") != path.parent.name:
        raise WorkshopStateError(f"workshop job/event directory mismatch at {path}")
    passes = job.get("passes")
    if not isinstance(passes, list) or len(passes) != PASS_COUNT:
        raise WorkshopStateError(f"workshop job {path} does not contain {PASS_COUNT} passes")
    return job


def get_job(
    event_id: str,
    *,
    root: Path = DEFAULT_WORKSHOP_DIR,
) -> dict[str, Any] | None:
    """Return a detached job document, or ``None`` if it has not been queued."""
    path = _job_path(root, event_id)
    if not path.exists():
        return None
    return _json_copy(_load_job_path(path))


def _expected_hash(event: Mapping[str, Any], key: str, raw: bytes) -> str:
    declared = str(event.get(key, "")).lower()
    actual = _sha256(raw)
    if not HASH_RE.fullmatch(declared) or declared != actual:
        raise WorkshopBindingError(f"feedback {key} does not match the supplied artifact")
    return actual


def _build_job(
    event: Mapping[str, Any],
    *,
    base_manifest: Mapping[str, Any],
) -> dict[str, Any]:
    created_at = _now()
    passes: list[dict[str, Any]] = []
    for index, plan in enumerate(PASS_PLAN, start=1):
        parent_score = "base/score.json" if index == 1 else f"candidates/pass-{index - 1:02d}/score.json"
        passes.append(
            {
                "pass_index": index,
                "focus": plan["focus"],
                "objective": plan["objective"],
                "status": "queued" if index == 1 else "pending",
                "attempt": 0,
                "parent_score": parent_score,
                "candidate": None,
                "last_error": None,
                "started_at": None,
                "completed_at": None,
            }
        )
    return {
        "schema_version": JOB_SCHEMA_VERSION,
        "kind": JOB_KIND,
        "event_id": str(event["event_id"]),
        "decision": str(event["decision"]),
        "project_id": str(event["project_id"]),
        "base_revision": int(event["score_revision"]),
        "status": "queued",
        "created_at": created_at,
        "updated_at": created_at,
        "pass_count": PASS_COUNT,
        "base": {
            "event_path": "base/feedback_event.json",
            "score_path": "base/score.json",
            "state_path": "base/state.json",
            "wav_path": "base/render.wav",
            "manifest_path": "base/manifest.json",
            "event_sha256": base_manifest["event_sha256"],
            "score_sha256": base_manifest["score_sha256"],
            "state_sha256": base_manifest["state_sha256"],
            "wav_sha256": base_manifest["wav_sha256"],
        },
        "passes": passes,
        "review": None,
        "failures": [],
        "recoveries": [],
    }


def queue_job(
    event_path: Path,
    *,
    score_path: Path,
    state_path: Path,
    wav_path: Path,
    root: Path = DEFAULT_WORKSHOP_DIR,
    pass_count: int = PASS_COUNT,
) -> dict[str, Any]:
    """Idempotently queue one exact feedback artifact for four workshop passes."""
    if int(pass_count) != PASS_COUNT:
        raise WorkshopBindingError(f"Court Synth workshop requires exactly {PASS_COUNT} passes")
    event_path = Path(event_path).resolve()
    root = Path(root).resolve()
    event_raw, event = _read_json(event_path, "feedback event")
    event_id = _validate_event_id(str(event.get("event_id", "")))
    decision = str(event.get("decision", ""))
    try:
        semantics = court_feedback.decision_semantics(decision)
    except ValueError as exc:
        raise WorkshopBindingError(str(exc)) from exc
    if not semantics["continue_work"] or not bool(event.get("action", {}).get("continue_work")):
        raise WorkshopBindingError(f"feedback decision {decision!r} is not a workshop action")

    # A positive work verdict already owns an immutable keeper. Prefer those
    # exact captured bytes even when the live renderer has since rewritten its
    # timestamped state or the front stage has moved on. Negative repair jobs
    # have no keeper, so they remain strictly bound to the explicit live paths.
    source_kind = "live_capture"
    keeper_manifest: dict[str, Any] | None = None
    keeper_relative = event.get("artifacts", {}).get("keeper_snapshot")
    if semantics["keeper"]:
        if not isinstance(keeper_relative, str) or not keeper_relative.strip():
            raise WorkshopBindingError("positive work feedback lacks its immutable keeper snapshot")
        feedback_root = event_path.parent.parent.resolve()
        keeper_dir = (feedback_root / keeper_relative).resolve()
        try:
            keeper_dir.relative_to(feedback_root)
        except ValueError as exc:
            raise WorkshopBindingError("feedback keeper path escapes the feedback store") from exc
        _manifest_raw, keeper_manifest = _read_json(
            keeper_dir / "manifest.json", "keeper manifest"
        )
        if keeper_manifest.get("event_id") != event_id:
            raise WorkshopBindingError("keeper manifest event_id does not match feedback")
        score_path = keeper_dir / "score.json"
        state_path = keeper_dir / "state.json"
        wav_path = keeper_dir / "render.wav"
        source_kind = "immutable_keeper"
    else:
        score_path = Path(score_path).resolve()
        state_path = Path(state_path).resolve()
        wav_path = Path(wav_path).resolve()

    score_raw, score = _read_json(score_path, "base CourtScore")
    state_raw, state = _read_json(state_path, "base render state")
    try:
        wav_raw = wav_path.read_bytes()
    except OSError as exc:
        raise WorkshopBindingError(f"could not read base WAV at {wav_path}: {exc}") from exc
    if not wav_raw:
        raise WorkshopBindingError("base WAV is empty")
    if event.get("project_id") != score.get("project_id"):
        raise WorkshopBindingError("feedback project_id does not match the base CourtScore")
    if event.get("score_revision") != score.get("revision"):
        raise WorkshopBindingError("feedback revision does not match the base CourtScore")
    if state.get("revision") != score.get("revision"):
        raise WorkshopBindingError("base render state revision does not match the CourtScore")

    score_hash = _expected_hash(event, "score_sha256", score_raw)
    state_hash = _expected_hash(event, "state_sha256", state_raw)
    wav_hash = _expected_hash(event, "wav_sha256", wav_raw)
    renderer_hash = _renderer_score_sha256(score)
    state_score_hash = str(state.get("score_hash", "")).lower()
    if len(state_score_hash) not in {16, 64} or not renderer_hash.startswith(state_score_hash):
        raise WorkshopBindingError("base render state score_hash does not match the CourtScore")
    if keeper_manifest is not None:
        for key, actual in (
            ("score_sha256", score_hash),
            ("state_sha256", state_hash),
            ("wav_sha256", wav_hash),
        ):
            if keeper_manifest.get(key) != actual:
                raise WorkshopBindingError(f"immutable keeper manifest has stale {key}")
    else:
        declared_render = state.get("render_path")
        if not isinstance(declared_render, str) or not declared_render.strip():
            raise WorkshopBindingError("base render state does not declare render_path")
        declared_path = Path(declared_render)
        if not declared_path.is_absolute():
            declared_path = state_path.parent / declared_path
        if declared_path.resolve() != wav_path:
            raise WorkshopBindingError("supplied base WAV is not the render bound by state.json")

    # Re-read after capture to reject an atomic front-stage replacement during
    # snapshotting instead of binding a mixed generation of artifacts.
    if event_path.read_bytes() != event_raw or score_path.read_bytes() != score_raw:
        raise WorkshopBindingError("feedback or CourtScore changed during workshop capture")
    if state_path.read_bytes() != state_raw or wav_path.read_bytes() != wav_raw:
        raise WorkshopBindingError("render state or WAV changed during workshop capture")

    base_manifest = {
        "schema_version": 1,
        "kind": "court_synth_workshop_base",
        "event_id": event_id,
        "decision": decision,
        "project_id": event["project_id"],
        "score_revision": event["score_revision"],
        "source_kind": source_kind,
        "event_sha256": _sha256(event_raw),
        "score_sha256": score_hash,
        "renderer_score_sha256": renderer_hash,
        "state_sha256": state_hash,
        "wav_sha256": wav_hash,
        "files": {
            "event": "feedback_event.json",
            "score": "score.json",
            "state": "state.json",
            "render": "render.wav",
        },
    }
    destination = _job_dir(root, event_id)
    jobs_dir = destination.parent
    jobs_dir.mkdir(parents=True, exist_ok=True)

    with _store_lock(root):
        if destination.exists():
            existing = _load_job_path(destination / "job.json")
            base = existing.get("base", {})
            if (
                base.get("score_sha256") != score_hash
                or base.get("state_sha256") != state_hash
                or base.get("wav_sha256") != wav_hash
            ):
                raise WorkshopBindingError(f"event_id collision with a different base at {destination}")
            result = _json_copy(existing)
            result["created"] = False
            result["message"] = "workshop job already exists for this exact feedback event"
            return result

        temporary = jobs_dir / f".{event_id}.{uuid.uuid4().hex}.tmp"
        try:
            base_dir = temporary / "base"
            _write_fsynced(base_dir / "feedback_event.json", event_raw)
            _write_fsynced(base_dir / "score.json", score_raw)
            _write_fsynced(base_dir / "state.json", state_raw)
            _write_fsynced(base_dir / "render.wav", wav_raw)
            _write_fsynced(base_dir / "manifest.json", _json_bytes(base_manifest))
            (temporary / "candidates").mkdir()
            job = _build_job(event, base_manifest=base_manifest)
            _write_fsynced(temporary / "job.json", _json_bytes(job))
            os.rename(temporary, destination)
        finally:
            if temporary.exists():
                shutil.rmtree(temporary)
    result = _json_copy(job)
    result["created"] = True
    result["message"] = "immutable base captured; four-pass workshop job queued"
    return result


def _parse_time(value: Any) -> float | None:
    if not isinstance(value, str):
        return None
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()
    except ValueError:
        return None


def _claim_envelope(job: Mapping[str, Any], item: Mapping[str, Any], root: Path) -> dict[str, Any]:
    job_dir = _job_dir(root, str(job["event_id"]))
    base_path = (job_dir / str(job["base"]["score_path"])).resolve()
    parent_path = (job_dir / str(item["parent_score"])).resolve()
    base_hash = _sha256(base_path.read_bytes())
    parent_hash = _sha256(parent_path.read_bytes())
    if base_hash != job["base"]["score_sha256"]:
        raise WorkshopBindingError(f"immutable base score was modified for {job['event_id']}")
    return {
        "status": "claimed",
        "event_id": job["event_id"],
        "decision": job["decision"],
        "project_id": job["project_id"],
        "base_revision": job["base_revision"],
        "pass_index": item["pass_index"],
        "attempt": item["attempt"],
        "planned_passes": job["pass_count"],
        "pass_count": job["pass_count"],
        "focus": item["focus"],
        "objective": item["objective"],
        "base_score_path": str(base_path),
        "parent_score_path": str(parent_path),
        "base_score_sha256": base_hash,
        "parent_score_sha256": parent_hash,
        "job_path": str(job_dir / "job.json"),
    }


def claim_next_job(
    *,
    root: Path = DEFAULT_WORKSHOP_DIR,
    lease_seconds: int = 1800,
) -> dict[str, Any]:
    """Atomically claim the oldest actionable pass, or return ``status=idle``."""
    root = Path(root).resolve()
    now_epoch = time.time()
    with _store_lock(root):
        paths = sorted(
            (root / "jobs").glob("*/job.json") if (root / "jobs").exists() else [],
            key=lambda path: path.as_posix(),
        )
        jobs = sorted(
            (_load_job_path(path) for path in paths),
            key=lambda job: (str(job.get("created_at", "")), str(job["event_id"])),
        )

        # One workshop worker at a time.  Expired claims are safely requeued so
        # a crashed producer cannot strand the durable pipeline forever.
        for job in jobs:
            for item in job["passes"]:
                if item["status"] != "in_progress":
                    continue
                started = _parse_time(item.get("started_at"))
                if started is not None and now_epoch - started <= max(1, int(lease_seconds)):
                    return {
                        "status": "idle",
                        "reason": "workshop_busy",
                        "active_event_id": job["event_id"],
                        "active_pass_index": item["pass_index"],
                    }
                item["status"] = "queued"
                item["last_error"] = "claim lease expired; pass safely requeued"
                job["status"] = "queued"
                job["updated_at"] = _now()
                _atomic_json(_job_path(root, str(job["event_id"])), job)

        for job in jobs:
            if job["status"] not in {"queued", "in_progress"}:
                continue
            item = next((entry for entry in job["passes"] if entry["status"] == "queued"), None)
            if item is None:
                continue
            item["status"] = "in_progress"
            item["attempt"] = int(item.get("attempt", 0)) + 1
            item["started_at"] = _now()
            item["last_error"] = None
            job["status"] = "in_progress"
            job["updated_at"] = _now()
            _atomic_json(_job_path(root, str(job["event_id"])), job)
            return _claim_envelope(job, item, root)
    return {"status": "idle", "reason": "no_queued_work"}


def _find_pass(job: Mapping[str, Any], pass_index: int) -> dict[str, Any]:
    if pass_index < 1 or pass_index > PASS_COUNT:
        raise WorkshopStateError(f"pass_index must be between 1 and {PASS_COUNT}")
    return job["passes"][pass_index - 1]


def _guard_claim(
    job: Mapping[str, Any],
    item: Mapping[str, Any],
    *,
    root: Path,
    attempt: int | None,
    expected_base_sha256: str | None,
    expected_parent_sha256: str | None,
) -> tuple[Path, Path, str, str]:
    if item["status"] != "in_progress":
        raise WorkshopStateError(
            f"pass {item['pass_index']} for {job['event_id']} is {item['status']}, not in_progress"
        )
    if attempt is not None and int(item["attempt"]) != int(attempt):
        raise WorkshopStateError(
            f"stale workshop attempt {attempt}; current attempt is {item['attempt']}"
        )
    job_dir = _job_dir(root, str(job["event_id"]))
    base_path = (job_dir / str(job["base"]["score_path"])).resolve()
    parent_path = (job_dir / str(item["parent_score"])).resolve()
    try:
        base_hash = _sha256(base_path.read_bytes())
        parent_hash = _sha256(parent_path.read_bytes())
    except OSError as exc:
        raise WorkshopBindingError(f"workshop parent artifact is missing: {exc}") from exc
    if base_hash != job["base"]["score_sha256"]:
        raise WorkshopBindingError("immutable workshop base score hash changed")
    if expected_base_sha256 is not None and expected_base_sha256.lower() != base_hash:
        raise WorkshopBindingError("expected base score hash no longer matches")
    if expected_parent_sha256 is not None and expected_parent_sha256.lower() != parent_hash:
        raise WorkshopBindingError("expected parent score hash no longer matches")
    return base_path, parent_path, base_hash, parent_hash


def _render_candidate_files(score_path: Path, wav_path: Path, state_path: Path) -> dict[str, Any]:
    # Lazy import avoids making queue/status operations depend on NumPy, audio,
    # or Tk.  render_to_path validates every structural/harmonic/arrangement
    # gate before it emits isolated artifacts.
    import court_synthesizer

    _target, summary, _audio = court_synthesizer.render_to_path(
        score_path,
        wav_path,
        state_out_path=state_path,
    )
    return summary


def publish_candidate(
    event_id: str,
    pass_index: int,
    candidate_path: Path,
    *,
    root: Path = DEFAULT_WORKSHOP_DIR,
    attempt: int | None = None,
    expected_base_sha256: str | None = None,
    expected_parent_sha256: str | None = None,
) -> dict[str, Any]:
    """Validate/render a claimed pass and atomically publish its candidate."""
    event_id = _validate_event_id(event_id)
    root = Path(root).resolve()
    candidate_raw, candidate = _read_json(Path(candidate_path).resolve(), "workshop candidate")
    initial = get_job(event_id, root=root)
    if initial is None:
        raise WorkshopStateError(f"unknown workshop event {event_id}")
    item = _find_pass(initial, int(pass_index))
    _base_path, _parent_path, base_hash, parent_hash = _guard_claim(
        initial,
        item,
        root=root,
        attempt=attempt,
        expected_base_sha256=expected_base_sha256,
        expected_parent_sha256=expected_parent_sha256,
    )
    base_score = json.loads(_base_path.read_text(encoding="utf-8"))
    parent_score = json.loads(_parent_path.read_text(encoding="utf-8"))
    candidate_identity = _protected_identity(candidate)
    base_identity = _protected_identity(base_score)
    if candidate_identity != base_identity:
        changed = [
            key for key in base_identity if candidate_identity.get(key) != base_identity.get(key)
        ]
        raise WorkshopBindingError(
            "candidate drifted protected keeper identity: " + ", ".join(changed)
        )
    if _audible_fingerprint(candidate) == _audible_fingerprint(parent_score):
        raise WorkshopBindingError(
            "candidate is a no-op: it must contain a real audible change from its parent"
        )
    _validate_pass_delta(candidate, parent_score, int(pass_index))

    job_dir = _job_dir(root, event_id)
    candidates_dir = job_dir / "candidates"
    candidates_dir.mkdir(exist_ok=True)
    final_dir = candidates_dir / f"pass-{int(pass_index):02d}"
    staging = candidates_dir / f".pass-{int(pass_index):02d}.{uuid.uuid4().hex}.tmp"
    try:
        _write_fsynced(staging / "score.json", candidate_raw)
        _render_candidate_files(staging / "score.json", staging / "render.wav", staging / "state.json")
        state_raw, state = _read_json(staging / "state.json", "candidate render state")
        wav_raw = (staging / "render.wav").read_bytes()
        if not wav_raw:
            raise WorkshopBindingError("candidate renderer emitted an empty WAV")
        renderer_hash = _renderer_score_sha256(candidate)
        state_score_hash = str(state.get("score_hash", "")).lower()
        if len(state_score_hash) not in {16, 64} or not renderer_hash.startswith(state_score_hash):
            raise WorkshopBindingError("candidate render state does not bind the candidate score")
        if state.get("revision") != candidate.get("revision"):
            raise WorkshopBindingError("candidate render state revision does not match the score")

        # The staging directory will be renamed.  Publish the durable final WAV
        # path in state rather than leaving a dead temporary pathname behind.
        state["render_path"] = str((final_dir / "render.wav").resolve())
        _atomic_json(staging / "state.json", state)
        state_raw = (staging / "state.json").read_bytes()
        candidate_hash = _sha256(candidate_raw)
        state_hash = _sha256(state_raw)
        wav_hash = _sha256(wav_raw)
        manifest = {
            "schema_version": 1,
            "kind": "court_synth_workshop_candidate",
            "event_id": event_id,
            "decision": initial["decision"],
            "project_id": initial["project_id"],
            "pass_index": int(pass_index),
            "attempt": int(item["attempt"]),
            "focus": item["focus"],
            "base_score_sha256": base_hash,
            "parent_score_sha256": parent_hash,
            "score_sha256": candidate_hash,
            "renderer_score_sha256": renderer_hash,
            "state_sha256": state_hash,
            "wav_sha256": wav_hash,
            "published_at": _now(),
            "files": {"score": "score.json", "state": "state.json", "render": "render.wav"},
        }
        _write_fsynced(staging / "manifest.json", _json_bytes(manifest))

        with _store_lock(root):
            latest = _load_job_path(job_dir / "job.json")
            latest_item = _find_pass(latest, int(pass_index))
            if latest_item["status"] == "completed" and final_dir.exists():
                existing = json.loads((final_dir / "manifest.json").read_text(encoding="utf-8"))
                if existing.get("score_sha256") != candidate_hash:
                    raise WorkshopStateError("completed pass is immutable and has a different candidate")
                return {
                    "status": "published",
                    "idempotent": True,
                    "event_id": event_id,
                    "pass_index": int(pass_index),
                    "job_status": latest["status"],
                    "candidate_score_path": str((final_dir / "score.json").resolve()),
                    "candidate_state_path": str((final_dir / "state.json").resolve()),
                    "candidate_wav_path": str((final_dir / "render.wav").resolve()),
                    "manifest_path": str((final_dir / "manifest.json").resolve()),
                    "score_sha256": candidate_hash,
                    "state_sha256": state_hash,
                    "wav_sha256": wav_hash,
                }
            _guard_claim(
                latest,
                latest_item,
                root=root,
                attempt=attempt,
                expected_base_sha256=expected_base_sha256,
                expected_parent_sha256=expected_parent_sha256,
            )
            if final_dir.exists():
                raise WorkshopStateError(f"candidate directory already exists for pass {pass_index}")
            os.rename(staging, final_dir)

            relative = final_dir.relative_to(job_dir).as_posix()
            latest_item["status"] = "completed"
            latest_item["completed_at"] = manifest["published_at"]
            latest_item["candidate"] = {
                "directory": relative,
                "score_path": f"{relative}/score.json",
                "state_path": f"{relative}/state.json",
                "wav_path": f"{relative}/render.wav",
                "manifest_path": f"{relative}/manifest.json",
                "score_sha256": candidate_hash,
                "state_sha256": state_hash,
                "wav_sha256": wav_hash,
                "parent_score_sha256": parent_hash,
            }
            if int(pass_index) < PASS_COUNT:
                next_item = latest["passes"][int(pass_index)]
                if next_item["status"] != "pending":
                    raise WorkshopStateError("next workshop pass is not pending")
                next_item["status"] = "queued"
                latest["status"] = "queued"
            else:
                latest["status"] = "review_ready"
                latest["review"] = {
                    "status": "pending_human_review",
                    "pass_index": int(pass_index),
                    **latest_item["candidate"],
                }
            latest["updated_at"] = _now()
            _atomic_json(job_dir / "job.json", latest)
    finally:
        if staging.exists():
            shutil.rmtree(staging)

    return {
        "status": "published",
        "idempotent": False,
        "event_id": event_id,
        "pass_index": int(pass_index),
        "job_status": latest["status"],
        "candidate_score_path": str((final_dir / "score.json").resolve()),
        "candidate_state_path": str((final_dir / "state.json").resolve()),
        "candidate_wav_path": str((final_dir / "render.wav").resolve()),
        "manifest_path": str((final_dir / "manifest.json").resolve()),
        "score_sha256": candidate_hash,
        "state_sha256": state_hash,
        "wav_sha256": wav_hash,
    }


def fail_pass(
    event_id: str,
    pass_index: int,
    reason: str,
    *,
    root: Path = DEFAULT_WORKSHOP_DIR,
    attempt: int | None = None,
    terminal: bool = False,
) -> dict[str, Any]:
    """Record an exact pass failure and requeue it unless marked terminal."""
    event_id = _validate_event_id(event_id)
    reason = str(reason).strip()
    if not reason:
        raise WorkshopStateError("workshop failure reason cannot be empty")
    root = Path(root).resolve()
    with _store_lock(root):
        path = _job_path(root, event_id)
        if not path.exists():
            raise WorkshopStateError(f"unknown workshop event {event_id}")
        job = _load_job_path(path)
        item = _find_pass(job, int(pass_index))
        if item["status"] != "in_progress":
            raise WorkshopStateError(f"pass {pass_index} is {item['status']}, not in_progress")
        if attempt is not None and int(attempt) != int(item["attempt"]):
            raise WorkshopStateError(f"stale workshop failure attempt {attempt}")
        failed_at = _now()
        job["failures"].append(
            {
                "pass_index": int(pass_index),
                "attempt": int(item["attempt"]),
                "reason": reason,
                "failed_at": failed_at,
                "terminal": bool(terminal),
            }
        )
        item["last_error"] = reason
        item["status"] = "failed" if terminal else "queued"
        job["status"] = "failed" if terminal else "queued"
        job["updated_at"] = failed_at
        _atomic_json(path, job)
    return {
        "status": job["status"],
        "event_id": event_id,
        "pass_index": int(pass_index),
        "attempt": int(item["attempt"]),
        "reason": reason,
    }


def retry_failed_job(
    event_id: str,
    reason: str,
    *,
    root: Path = DEFAULT_WORKSHOP_DIR,
) -> dict[str, Any]:
    """Explicitly requeue one terminal-failed pass after a systemic repair.

    This recovery transition changes only the durable job document.  The
    immutable base, every published candidate, and the front-stage score,
    renderer state, and WAV remain untouched.
    """
    event_id = _validate_event_id(event_id)
    reason = str(reason).strip()
    if not reason:
        raise WorkshopStateError("workshop recovery reason cannot be empty")
    root = Path(root).resolve()
    with _store_lock(root):
        path = _job_path(root, event_id)
        if not path.exists():
            raise WorkshopStateError(f"unknown workshop event {event_id}")
        job = _load_job_path(path)
        if job["status"] != "failed":
            raise WorkshopStateError(
                f"workshop job {event_id} is {job['status']}, not terminal failed"
            )
        failed_passes = [item for item in job["passes"] if item["status"] == "failed"]
        if len(failed_passes) != 1:
            raise WorkshopStateError(
                "terminal recovery requires exactly one failed workshop pass"
            )
        item = failed_passes[0]
        terminal_failure = next(
            (
                failure
                for failure in reversed(job.get("failures", []))
                if bool(failure.get("terminal"))
                and int(failure.get("pass_index", -1)) == int(item["pass_index"])
                and int(failure.get("attempt", -1)) == int(item.get("attempt", 0))
            ),
            None,
        )
        if terminal_failure is None:
            raise WorkshopStateError(
                "failed workshop pass has no matching terminal failure record"
            )

        recovered_at = _now()
        previous_attempt = int(item.get("attempt", 0))
        previous_error = item.get("last_error")
        recovery = {
            "pass_index": int(item["pass_index"]),
            "reason": reason,
            "recovered_at": recovered_at,
            "previous_attempt": previous_attempt,
            "previous_error": previous_error,
            "terminal_failure": _json_copy(terminal_failure),
        }
        recoveries = job.setdefault("recoveries", [])
        if not isinstance(recoveries, list):
            raise WorkshopStateError("workshop recovery history is invalid")
        recoveries.append(recovery)
        item["status"] = "queued"
        item["attempt"] = 0
        item["started_at"] = None
        item["completed_at"] = None
        item["last_error"] = f"recovered for retry: {reason}"
        job["status"] = "queued"
        job["updated_at"] = recovered_at
        _atomic_json(path, job)
    return {
        "status": "queued",
        "event_id": event_id,
        "pass_index": int(item["pass_index"]),
        "attempt": 0,
        "reason": reason,
        "recovered_at": recovered_at,
    }


def review_candidate(
    event_id: str,
    *,
    root: Path = DEFAULT_WORKSHOP_DIR,
) -> dict[str, Any]:
    """Return absolute, hash-bound paths for a review-ready final candidate."""
    job = get_job(event_id, root=root)
    if job is None:
        raise WorkshopStateError(f"unknown workshop event {event_id}")
    if job["status"] != "review_ready" or not isinstance(job.get("review"), dict):
        raise WorkshopStateError(f"workshop job {event_id} is {job['status']}, not review_ready")
    job_dir = _job_dir(root, event_id)
    review = _json_copy(job["review"])
    for key in ("score_path", "state_path", "wav_path", "manifest_path"):
        review[key] = str((job_dir / review[key]).resolve())
    review_status = str(review.pop("status", "pending_human_review"))
    return {
        "status": "review_ready",
        "review_status": review_status,
        "event_id": event_id,
        "decision": job["decision"],
        **review,
    }


def workshop_status(
    *,
    root: Path = DEFAULT_WORKSHOP_DIR,
    event_id: str | None = None,
) -> dict[str, Any]:
    """Return a stable JSON status document usable by native UI and Rust."""
    root = Path(root).resolve()
    if event_id is not None:
        job = get_job(event_id, root=root)
        return {"status": "ok", "job": job}
    jobs_dir = root / "jobs"
    jobs = [] if not jobs_dir.exists() else [
        _load_job_path(path) for path in sorted(jobs_dir.glob("*/job.json"))
    ]
    counts: dict[str, int] = {}
    summaries: list[dict[str, Any]] = []
    for job in jobs:
        status = str(job["status"])
        counts[status] = counts.get(status, 0) + 1
        completed = sum(item["status"] == "completed" for item in job["passes"])
        current = next(
            (item for item in job["passes"] if item["status"] in {"queued", "in_progress", "failed"}),
            None,
        )
        summaries.append(
            {
                "event_id": job["event_id"],
                "decision": job["decision"],
                "project_id": job["project_id"],
                "status": status,
                "completed_passes": completed,
                "pass_count": job["pass_count"],
                "current_pass_index": current["pass_index"] if current else None,
                "current_focus": current["focus"] if current else None,
                "updated_at": job["updated_at"],
            }
        )
    return {
        "status": "ok",
        "root": str(root),
        "job_count": len(jobs),
        "counts": counts,
        "jobs": summaries,
    }


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Court Synth durable back workshop")
    sub = parser.add_subparsers(dest="command", required=True)

    queue = sub.add_parser("queue")
    queue.add_argument("--event", required=True, type=Path)
    queue.add_argument("--score", required=True, type=Path)
    queue.add_argument("--state", required=True, type=Path)
    queue.add_argument("--wav", required=True, type=Path)
    queue.add_argument("--passes", type=int, default=PASS_COUNT)
    queue.add_argument("--root", type=Path, default=DEFAULT_WORKSHOP_DIR)

    next_job = sub.add_parser("next")
    next_job.add_argument("--root", type=Path, default=DEFAULT_WORKSHOP_DIR)
    next_job.add_argument("--lease-seconds", type=int, default=1800)

    publish = sub.add_parser("publish")
    publish.add_argument("--event-id", required=True)
    publish.add_argument("--pass-index", required=True, type=int)
    publish.add_argument("--candidate", required=True, type=Path)
    publish.add_argument("--attempt", type=int)
    publish.add_argument("--expected-base-sha256")
    publish.add_argument("--expected-parent-sha256")
    publish.add_argument("--root", type=Path, default=DEFAULT_WORKSHOP_DIR)

    fail = sub.add_parser("fail")
    fail.add_argument("--event-id", required=True)
    fail.add_argument("--pass-index", required=True, type=int)
    fail.add_argument("--reason", required=True)
    fail.add_argument("--attempt", type=int)
    fail.add_argument("--terminal", action="store_true")
    fail.add_argument("--root", type=Path, default=DEFAULT_WORKSHOP_DIR)

    retry = sub.add_parser("retry")
    retry.add_argument("--event-id", required=True)
    retry.add_argument("--reason", required=True)
    retry.add_argument("--root", type=Path, default=DEFAULT_WORKSHOP_DIR)

    status = sub.add_parser("status")
    status.add_argument("--event-id")
    status.add_argument("--json", action="store_true", help="JSON is always emitted; retained for callers")
    status.add_argument("--root", type=Path, default=DEFAULT_WORKSHOP_DIR)

    review = sub.add_parser("review")
    review.add_argument("--event-id", required=True)
    review.add_argument("--root", type=Path, default=DEFAULT_WORKSHOP_DIR)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        if args.command == "queue":
            job = queue_job(
                args.event,
                score_path=args.score,
                state_path=args.state,
                wav_path=args.wav,
                root=args.root,
                pass_count=args.passes,
            )
            result: dict[str, Any] = {
                "status": job["status"],
                "event_id": job["event_id"],
                "created": bool(job.get("created", False)),
                "message": str(job.get("message", "workshop queue checked")),
                "job": job,
            }
        elif args.command == "next":
            result = claim_next_job(root=args.root, lease_seconds=args.lease_seconds)
        elif args.command == "publish":
            result = publish_candidate(
                args.event_id,
                args.pass_index,
                args.candidate,
                root=args.root,
                attempt=args.attempt,
                expected_base_sha256=args.expected_base_sha256,
                expected_parent_sha256=args.expected_parent_sha256,
            )
        elif args.command == "fail":
            result = fail_pass(
                args.event_id,
                args.pass_index,
                args.reason,
                root=args.root,
                attempt=args.attempt,
                terminal=args.terminal,
            )
        elif args.command == "retry":
            result = retry_failed_job(args.event_id, args.reason, root=args.root)
        elif args.command == "review":
            result = review_candidate(args.event_id, root=args.root)
        else:
            result = workshop_status(root=args.root, event_id=args.event_id)
    except WorkshopError as exc:
        print(json.dumps({"status": "error", "error": str(exc)}, ensure_ascii=False))
        return 1
    print(json.dumps(result, ensure_ascii=False))
    return 0


__all__ = [
    "DEFAULT_WORKSHOP_DIR",
    "JOB_KIND",
    "JOB_SCHEMA_VERSION",
    "PASS_COUNT",
    "PASS_PLAN",
    "WorkshopBindingError",
    "WorkshopError",
    "WorkshopStateError",
    "claim_next_job",
    "fail_pass",
    "get_job",
    "publish_candidate",
    "queue_job",
    "retry_failed_job",
    "review_candidate",
    "workshop_status",
]


if __name__ == "__main__":
    raise SystemExit(main())
