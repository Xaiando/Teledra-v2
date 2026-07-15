"""Artifact-bound human feedback for Court Synth compositions.

The feedback store is deliberately separate from ``current_score.json``.
Rating a composition is an observation about an exact audible artifact, not a
musical edit, and therefore must not advance the score revision or wake the
renderer.

Each event is published as one immutable JSON file under
``court_synth/feedback/events``.  Event names are deterministic over the
decision and the exact score/WAV hashes, which makes repeated button presses
idempotent.  Positive decisions also preserve an immutable keeper directory
containing the score, render, render state, and a hash manifest.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import time
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterator, Mapping


PROJECT_DIR = Path(__file__).resolve().parent
DEFAULT_SCORE_PATH = PROJECT_DIR / "current_score.json"
DEFAULT_STATE_PATH = PROJECT_DIR / "state.json"
DEFAULT_FEEDBACK_DIR = PROJECT_DIR / "feedback"

EVENT_SCHEMA_VERSION = 1
EVENT_KIND = "court_music_feedback"
HASH_RE = re.compile(r"^[0-9a-f]{64}$")

TRACK_ORDER = (
    "drums",
    "percussion",
    "bass",
    "harmony",
    "pluck",
    "lead",
    "atmos",
    "fx",
)

V1_PATCHES = {
    "drums": "kit.mechanical_court",
    "percussion": "kit.velvet_lofi",
    "bass": "bass.substructure",
    "harmony": "keys.nocturne_felt",
    "pluck": "pluck.glass_current",
    "lead": "lead.ember_superwave",
    "atmos": "pad.aurora_choir",
    "fx": "fx.riser",
}

# These action fields are intentionally explicit and stable: the native UI can
# present the labels while Rust can apply policy without guessing at sentiment.
DECISION_SEMANTICS: dict[str, dict[str, Any]] = {
    "like_as_is": {
        "label": "Like (as is)",
        "valence": "positive",
        "learning_weight": 1.0,
        "continue_work": False,
        "preserve_identity": True,
        "replace_identity": False,
        "autonomous_lease_seconds": 600,
        "freeze_autonomous": True,
        "keeper": True,
    },
    "like_work_on_it": {
        "label": "Like but work on it",
        "valence": "positive",
        "learning_weight": 0.65,
        "continue_work": True,
        "preserve_identity": True,
        "replace_identity": False,
        "freeze_autonomous": False,
        "keeper": True,
    },
    "dislike": {
        "label": "Dislike",
        "valence": "negative",
        "learning_weight": -1.0,
        "continue_work": False,
        "preserve_identity": False,
        "replace_identity": True,
        "freeze_autonomous": False,
        "keeper": False,
    },
    "dislike_work_on_it": {
        "label": "Dislike but work on it",
        "valence": "negative",
        "learning_weight": -0.65,
        "continue_work": True,
        "preserve_identity": True,
        "replace_identity": False,
        "freeze_autonomous": False,
        "keeper": False,
    },
}


class FeedbackError(RuntimeError):
    """Base class for feedback persistence failures."""


class FeedbackBindingError(FeedbackError):
    """The score, state, and render do not describe one exact artifact."""


class FeedbackStoreError(FeedbackError):
    """The immutable event store is missing or corrupt."""


@dataclass(frozen=True)
class FeedbackResult:
    event: dict[str, Any]
    event_path: Path
    created: bool
    keeper_path: Path | None

    @property
    def decision(self) -> str:
        return str(self.event["decision"])


def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _json_bytes(value: Any) -> bytes:
    return (json.dumps(value, indent=2, ensure_ascii=False) + "\n").encode("utf-8")


def _json_copy(value: Any) -> Any:
    return json.loads(json.dumps(value, ensure_ascii=False))


def renderer_score_sha256(score: Mapping[str, Any]) -> str:
    """Match the hash input used by ``court_synthesizer.render_score``."""
    return _sha256(json.dumps(score, sort_keys=True).encode("utf-8"))


def decision_semantics(decision: str) -> dict[str, Any]:
    """Return a detached copy of the stable policy for one UI decision."""
    try:
        return _json_copy(DECISION_SEMANTICS[decision])
    except KeyError as exc:
        choices = ", ".join(DECISION_SEMANTICS)
        raise ValueError(f"unknown Court Synth feedback decision {decision!r}; expected {choices}") from exc


def _read_json_bytes(path: Path, label: str) -> tuple[bytes, dict[str, Any]]:
    try:
        raw = path.read_bytes()
    except OSError as exc:
        raise FeedbackBindingError(f"could not read {label} at {path}: {exc}") from exc
    try:
        value = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise FeedbackBindingError(f"{label} is not valid UTF-8 JSON: {exc}") from exc
    if not isinstance(value, dict):
        raise FeedbackBindingError(f"{label} root must be a JSON object")
    return raw, value


def _resolve_render_path(state_path: Path, state: Mapping[str, Any], render_path: Path | None) -> Path:
    declared = state.get("render_path")
    if not isinstance(declared, str) or not declared.strip():
        raise FeedbackBindingError("render state does not declare render_path")
    declared_path = Path(declared)
    if not declared_path.is_absolute():
        declared_path = state_path.parent / declared_path
    declared_path = declared_path.resolve()
    if render_path is not None and Path(render_path).resolve() != declared_path:
        raise FeedbackBindingError(
            f"requested WAV {Path(render_path).resolve()} does not match render state {declared_path}"
        )
    return declared_path


def _instrument_roster(score: Mapping[str, Any]) -> list[dict[str, Any]]:
    if score.get("schema_version") == 2:
        roster: list[dict[str, Any]] = []
        for track in score.get("tracks", []):
            if not isinstance(track, dict):
                continue
            instrument = track.get("instrument", {})
            mixer = track.get("mixer", {})
            roster.append({
                "track_id": track.get("id"),
                "role": track.get("role"),
                "patch_id": instrument.get("patch_id") if isinstance(instrument, dict) else None,
                "instrument": _json_copy(instrument) if isinstance(instrument, dict) else {},
                "mixer": _json_copy(mixer) if isinstance(mixer, dict) else {},
            })
        return roster

    track_mix = score.get("track_mix", {})
    if not isinstance(track_mix, dict):
        track_mix = {}
    instrumentation = score.get("instrumentation", {})
    if not isinstance(instrumentation, dict):
        instrumentation = {}
    return [
        {
            "track_id": track_id,
            "patch_id": instrumentation.get(track_id, {}).get("patch_id", V1_PATCHES[track_id])
            if isinstance(instrumentation.get(track_id, {}), dict) else V1_PATCHES[track_id],
            "instrument": _json_copy(instrumentation.get(track_id, {}))
            if isinstance(instrumentation.get(track_id, {}), dict) else {},
            "mixer": _json_copy(track_mix.get(track_id, {}))
            if isinstance(track_mix.get(track_id, {}), dict)
            else {},
        }
        for track_id in TRACK_ORDER
    ]


def _composition_features(score: Mapping[str, Any]) -> dict[str, Any]:
    manual_notes = score.get("manual_notes", [])
    return {
        "schema_version": score.get("schema_version"),
        "title": score.get("title"),
        "style": score.get("style"),
        "transport": _json_copy(score.get("transport", {})),
        "harmony": _json_copy(score.get("harmony", {})),
        "motif": _json_copy(score.get("motif", score.get("motifs", []))),
        "sections": _json_copy(score.get("sections", [])),
        "mix": _json_copy(score.get("mix", {})),
        "master": _json_copy(score.get("master", {})),
        "instrument_roster": _instrument_roster(score),
        "manual_note_count": len(manual_notes) if isinstance(manual_notes, list) else 0,
        "lineage_source": score.get("lineage", {}).get("source")
        if isinstance(score.get("lineage"), dict)
        else None,
        "changed_axis": score.get("lineage", {}).get("changed_axis")
        if isinstance(score.get("lineage"), dict)
        else None,
    }


def _verification_features(state: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "render": {
            key: _json_copy(state.get(key))
            for key in ("title", "style", "revision", "bpm", "bars", "seconds", "events", "tracks", "peak", "rendered_at")
        },
        "harmony_grade": _json_copy(state.get("harmony_grade", {})),
        "compiled_harmony_grade": _json_copy(state.get("compiled_harmony_grade", {})),
        "arrangement_grade": _json_copy(state.get("arrangement_grade", {})),
    }


def _write_temp_file(path: Path, data: bytes) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.parent / f".{path.name}.{uuid.uuid4().hex}.tmp"
    with open(temporary, "xb") as handle:
        handle.write(data)
        handle.flush()
        os.fsync(handle.fileno())
    return temporary


def _write_fsynced(path: Path, data: bytes) -> None:
    with open(path, "xb") as handle:
        handle.write(data)
        handle.flush()
        os.fsync(handle.fileno())


def _publish_immutable(path: Path, data: bytes) -> bool:
    """Atomically publish a complete file without replacing an existing event."""
    temporary: Path | None = _write_temp_file(path, data)
    try:
        try:
            # A hard-link publishes the already-fsynced inode in one operation
            # and, unlike os.replace, fails if the immutable destination exists.
            os.link(temporary, path)
            return True
        except FileExistsError:
            return False
        except OSError as link_error:
            if path.exists():
                return False
            if os.name != "nt":
                raise FeedbackStoreError(f"could not atomically publish {path}: {link_error}") from link_error
            # Windows os.rename is no-replace.  This is the supported fallback
            # for filesystems that decline hard-link creation.
            try:
                os.rename(temporary, path)
                temporary = None
                return True
            except FileExistsError:
                return False
    finally:
        if temporary is not None and temporary.exists():
            temporary.unlink()


def _remove_flat_directory(path: Path) -> None:
    if not path.exists():
        return
    for child in path.iterdir():
        if child.is_dir():
            raise FeedbackStoreError(f"unexpected nested temporary keeper directory: {child}")
        child.unlink()
    path.rmdir()


def _read_event(path: Path) -> dict[str, Any]:
    try:
        event = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise FeedbackStoreError(f"invalid feedback event {path}: {exc}") from exc
    if not isinstance(event, dict):
        raise FeedbackStoreError(f"feedback event {path} is not an object")
    required = ("event_id", "decision", "score_revision", "score_sha256", "project_id", "wav_sha256")
    missing = [key for key in required if key not in event]
    if missing:
        raise FeedbackStoreError(f"feedback event {path} lacks: {', '.join(missing)}")
    if event["decision"] not in DECISION_SEMANTICS:
        raise FeedbackStoreError(f"feedback event {path} has unknown decision {event['decision']!r}")
    if path.stem != event["event_id"]:
        raise FeedbackStoreError(f"feedback event filename does not match event_id: {path}")
    for key in ("score_sha256", "wav_sha256"):
        if not HASH_RE.fullmatch(str(event[key])):
            raise FeedbackStoreError(f"feedback event {path} has invalid {key}")
    return event


def _publish_keeper(
    feedback_dir: Path,
    event_id: str,
    score_bytes: bytes,
    state_bytes: bytes,
    wav_bytes: bytes,
    manifest: Mapping[str, Any],
) -> Path:
    keepers_dir = feedback_dir / "keepers"
    keepers_dir.mkdir(parents=True, exist_ok=True)
    destination = keepers_dir / event_id
    if destination.exists():
        existing = destination / "manifest.json"
        try:
            existing_manifest = json.loads(existing.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            raise FeedbackStoreError(f"invalid keeper manifest at {existing}: {exc}") from exc
        if existing_manifest != manifest:
            raise FeedbackStoreError(f"keeper collision or incomplete keeper at {destination}")
        return destination

    temporary = keepers_dir / f".{event_id}.{uuid.uuid4().hex}.tmp"
    temporary.mkdir()
    try:
        _write_fsynced(temporary / "score.json", score_bytes)
        _write_fsynced(temporary / "state.json", state_bytes)
        _write_fsynced(temporary / "render.wav", wav_bytes)
        _write_fsynced(temporary / "manifest.json", _json_bytes(manifest))
        try:
            os.rename(temporary, destination)
        except OSError:
            if not destination.exists():
                raise
            _remove_flat_directory(temporary)
        existing = destination / "manifest.json"
        if not existing.exists() or json.loads(existing.read_text(encoding="utf-8")) != manifest:
            raise FeedbackStoreError(f"keeper publication did not preserve the expected manifest at {destination}")
        return destination
    finally:
        if temporary.exists():
            _remove_flat_directory(temporary)


def record_feedback(
    decision: str,
    *,
    score_path: Path = DEFAULT_SCORE_PATH,
    state_path: Path = DEFAULT_STATE_PATH,
    render_path: Path | None = None,
    feedback_dir: Path = DEFAULT_FEEDBACK_DIR,
    expected_revision: int | None = None,
) -> FeedbackResult:
    """Record one decision against the exact score and audible WAV.

    ``expected_revision`` should be the native UI's loaded revision.  A stale
    editor, stale render state, mismatched renderer score hash, or mismatched
    render path is rejected rather than attaching feedback to the wrong song.
    """
    action = decision_semantics(decision)
    score_path = Path(score_path).resolve()
    state_path = Path(state_path).resolve()
    feedback_dir = Path(feedback_dir).resolve()

    score_bytes, score = _read_json_bytes(score_path, "CourtScore")
    state_bytes, state = _read_json_bytes(state_path, "render state")
    project_id = score.get("project_id")
    score_revision = score.get("revision")
    if not isinstance(project_id, str) or not project_id.strip():
        raise FeedbackBindingError("CourtScore project_id is missing")
    if not isinstance(score_revision, int) or isinstance(score_revision, bool) or score_revision < 1:
        raise FeedbackBindingError("CourtScore revision must be a positive integer")
    if expected_revision is not None and score_revision != int(expected_revision):
        raise FeedbackBindingError(
            f"editor loaded revision {expected_revision}, but disk contains revision {score_revision}"
        )
    if state.get("revision") != score_revision:
        raise FeedbackBindingError(
            f"render state revision {state.get('revision')!r} does not match score revision {score_revision}"
        )

    score_sha256 = _sha256(score_bytes)
    renderer_hash = renderer_score_sha256(score)
    state_score_hash = str(state.get("score_hash", "")).strip().lower()
    if (
        len(state_score_hash) not in {16, 64}
        or not all(character in "0123456789abcdef" for character in state_score_hash)
        or not renderer_hash.startswith(state_score_hash)
    ):
        raise FeedbackBindingError(
            "render state score_hash does not match the current CourtScore; render before rating"
        )

    bound_render_path = _resolve_render_path(state_path, state, render_path)
    try:
        wav_bytes = bound_render_path.read_bytes()
    except OSError as exc:
        raise FeedbackBindingError(f"could not read rendered WAV at {bound_render_path}: {exc}") from exc
    if not wav_bytes:
        raise FeedbackBindingError("rendered WAV is empty")
    wav_sha256 = _sha256(wav_bytes)
    state_sha256 = _sha256(state_bytes)

    # Detect an external atomic score/state replacement during capture.  The
    # keeper itself is made from the already captured bytes.
    if score_path.read_bytes() != score_bytes:
        raise FeedbackBindingError("CourtScore changed while feedback was being captured; try again")
    if state_path.read_bytes() != state_bytes:
        raise FeedbackBindingError("render state changed while feedback was being captured; try again")
    if _sha256(bound_render_path.read_bytes()) != wav_sha256:
        raise FeedbackBindingError("rendered WAV changed while feedback was being captured; try again")

    previous = latest_feedback(
        feedback_dir=feedback_dir,
        project_id=project_id,
        score_revision=score_revision,
        score_sha256=score_sha256,
    )
    # A repeated click on the current decision is idempotent.  A changed
    # decision starts a new immutable transition, and changing back later is a
    # new transition too.  Including the previous event ID keeps concurrent
    # identical clicks deterministic without making an old verdict permanent.
    if previous is not None and previous.get("decision") == decision:
        previous_path = feedback_dir / "events" / f"{previous['event_id']}.json"
        relative = previous.get("artifacts", {}).get("keeper_snapshot")
        previous_keeper = feedback_dir / relative if isinstance(relative, str) and relative else None
        return FeedbackResult(
            event=previous,
            event_path=previous_path,
            created=False,
            keeper_path=previous_keeper,
        )
    supersedes_event_id = str(previous["event_id"]) if previous is not None else None
    dedupe_material = json.dumps(
        {
            "decision": decision,
            "project_id": project_id,
            "score_revision": score_revision,
            "score_sha256": score_sha256,
            "wav_sha256": wav_sha256,
            "supersedes_event_id": supersedes_event_id,
        },
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    dedupe_key = _sha256(dedupe_material)
    event_id = f"music-feedback-{dedupe_key}"
    created_at_ns = time.time_ns()
    created_at = datetime.fromtimestamp(created_at_ns / 1_000_000_000, tz=timezone.utc).isoformat()

    keeper_path: Path | None = None
    keeper_relative: str | None = None
    if action["keeper"]:
        keeper_manifest = {
            "schema_version": 1,
            "kind": "court_music_keeper",
            "event_id": event_id,
            "decision": decision,
            "project_id": project_id,
            "score_revision": score_revision,
            "score_sha256": score_sha256,
            "renderer_score_sha256": renderer_hash,
            "state_sha256": state_sha256,
            "wav_sha256": wav_sha256,
            "files": {
                "score": "score.json",
                "state": "state.json",
                "render": "render.wav",
            },
        }
        keeper_path = _publish_keeper(
            feedback_dir,
            event_id,
            score_bytes,
            state_bytes,
            wav_bytes,
            keeper_manifest,
        )
        keeper_relative = keeper_path.relative_to(feedback_dir).as_posix()

    event = {
        "schema_version": EVENT_SCHEMA_VERSION,
        "kind": EVENT_KIND,
        "event_id": event_id,
        "dedupe_key": dedupe_key,
        "decision": decision,
        "supersedes_event_id": supersedes_event_id,
        "project_id": project_id,
        "score_revision": score_revision,
        # score_sha256 and wav_sha256 are hashes of exact raw file bytes, so a
        # Rust consumer can validate them without reproducing Python JSON rules.
        "score_sha256": score_sha256,
        "renderer_score_sha256": renderer_hash,
        "wav_sha256": wav_sha256,
        "state_sha256": state_sha256,
        "created_at": created_at,
        "created_at_unix_ns": created_at_ns,
        "action": action,
        "features": _composition_features(score),
        "verification": _verification_features(state),
        "artifacts": {
            "score_path": str(score_path),
            "state_path": str(state_path),
            "render_path": str(bound_render_path),
            "keeper_snapshot": keeper_relative,
        },
    }

    event_path = feedback_dir / "events" / f"{event_id}.json"
    created = _publish_immutable(event_path, _json_bytes(event))
    if not created:
        existing = _read_event(event_path)
        if existing.get("dedupe_key") != dedupe_key:
            raise FeedbackStoreError(f"feedback event collision at {event_path}")
        event = existing
        relative = event.get("artifacts", {}).get("keeper_snapshot")
        keeper_path = feedback_dir / relative if isinstance(relative, str) and relative else None
        if action["keeper"] and (keeper_path is None or not keeper_path.exists()):
            raise FeedbackStoreError(f"positive feedback event lacks its keeper snapshot: {event_path}")

    return FeedbackResult(
        event=event,
        event_path=event_path,
        created=created,
        keeper_path=keeper_path,
    )


def iter_feedback_events(
    feedback_dir: Path = DEFAULT_FEEDBACK_DIR,
    *,
    strict: bool = True,
) -> Iterator[dict[str, Any]]:
    """Yield immutable events in creation order.

    Strict mode is the safe default for learning: corrupt evidence must not be
    silently interpreted as preference data.
    """
    events_dir = Path(feedback_dir) / "events"
    if not events_dir.exists():
        return
    events: list[dict[str, Any]] = []
    for path in events_dir.glob("*.json"):
        try:
            events.append(_read_event(path))
        except FeedbackStoreError:
            if strict:
                raise
    events.sort(key=lambda item: (int(item.get("created_at_unix_ns", 0)), str(item["event_id"])))
    yield from events


def latest_feedback(
    *,
    feedback_dir: Path = DEFAULT_FEEDBACK_DIR,
    project_id: str | None = None,
    score_revision: int | None = None,
    score_sha256: str | None = None,
) -> dict[str, Any] | None:
    """Return the latest event matching the supplied exact binding filters."""
    latest: dict[str, Any] | None = None
    for event in iter_feedback_events(feedback_dir):
        if project_id is not None and event.get("project_id") != project_id:
            continue
        if score_revision is not None and event.get("score_revision") != score_revision:
            continue
        if score_sha256 is not None and event.get("score_sha256") != score_sha256:
            continue
        latest = event
    return latest


def current_feedback(
    score_path: Path = DEFAULT_SCORE_PATH,
    *,
    feedback_dir: Path = DEFAULT_FEEDBACK_DIR,
) -> dict[str, Any] | None:
    """Return the newest decision for the exact score bytes currently on disk."""
    score_path = Path(score_path)
    raw, score = _read_json_bytes(score_path, "CourtScore")
    project_id = score.get("project_id")
    revision = score.get("revision")
    if not isinstance(project_id, str) or not isinstance(revision, int):
        raise FeedbackBindingError("CourtScore lacks a valid project_id/revision binding")
    return latest_feedback(
        feedback_dir=feedback_dir,
        project_id=project_id,
        score_revision=revision,
        score_sha256=_sha256(raw),
    )


def current_decision(
    score_path: Path = DEFAULT_SCORE_PATH,
    *,
    feedback_dir: Path = DEFAULT_FEEDBACK_DIR,
) -> str | None:
    """Convenience API for native UI button highlighting."""
    event = current_feedback(score_path, feedback_dir=feedback_dir)
    return str(event["decision"]) if event is not None else None


__all__ = [
    "DECISION_SEMANTICS",
    "DEFAULT_FEEDBACK_DIR",
    "FeedbackBindingError",
    "FeedbackError",
    "FeedbackResult",
    "FeedbackStoreError",
    "current_decision",
    "current_feedback",
    "decision_semantics",
    "iter_feedback_events",
    "latest_feedback",
    "record_feedback",
    "renderer_score_sha256",
]
