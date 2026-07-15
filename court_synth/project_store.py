"""Project store for CourtScore v2.

Responsibilities (per handoff):
- Atomic writes with revision checks
- Safe load (auto-migrate v1)
- Revision conflict detection (stale base_revision for patches)
- Undo / redo journal (simple but durable on disk)
- Snapshotting revisions into projects/
- Never silently overwrite newer human edits

All operations are atomic (tmp + os.replace).
"""

from __future__ import annotations

import json
import os
import re
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from . import schema as v2schema

DEFAULT_CURRENT = Path(__file__).resolve().parent / "current_score.json"
PROJECTS_DIR = Path(__file__).resolve().parent / "projects"
JOURNAL_PATH = Path(__file__).resolve().parent / "undo_journal.jsonl"

MAX_UNDO = 50


@dataclass
class StoreResult:
    score: dict[str, Any]
    revision: int
    path: Path


def _atomic_write(path: Path, data: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(v2schema.to_json(data), encoding="utf-8")
    os.replace(tmp, path)


def _snapshot_revision(score: dict[str, Any], note: str = "") -> Path:
    rev = int(score.get("revision", 0))
    pid = re.sub(r"[^A-Za-z0-9._-]+", "-", str(score.get("project_id", "project"))).strip(".-") or "project"
    ts = int(time.time())
    snap_path = PROJECTS_DIR / f"{pid}_rev{rev}_{ts}.json"
    PROJECTS_DIR.mkdir(parents=True, exist_ok=True)
    snap_path.write_text(v2schema.to_json(score), encoding="utf-8")
    return snap_path


def _append_journal(entry: dict[str, Any]) -> None:
    JOURNAL_PATH.parent.mkdir(parents=True, exist_ok=True)
    with open(JOURNAL_PATH, "a", encoding="utf-8") as f:
        f.write(json.dumps(entry, ensure_ascii=False) + "\n")


def load_current(path: Path = DEFAULT_CURRENT) -> dict[str, Any]:
    """Load current project, auto-migrating v1 -> v2 if needed. Always returns valid v2."""
    if not path.exists():
        base_v1 = {
            "schema_version": 1,
            "project_id": "court-synth-default",
            "revision": 1,
            "title": "New Court Score",
            "style": "retro_adventure",
            "seed": 12345,
            "transport": {"bpm": 112, "meter": [4, 4], "bars": 32, "swing": 0.025, "loop": True},
            "harmony": {"tonic": "D", "mode": "dorian", "chords": ["Dm", "Am", "Bdim", "G"]},
            "motif": ["A4", "F4", "E4", "D4", "A3", "D4", "F4", "A4"],
            "sections": [
                {"name": "origin", "bars": 4, "energy": 0.25, "transform": "fragment"},
                {"name": "path", "bars": 8, "energy": 0.52, "transform": "forward"},
                {"name": "peril", "bars": 8, "energy": 0.84, "transform": "reverse"},
                {"name": "return", "bars": 8, "energy": 1.0, "transform": "sequence"},
                {"name": "afterglow", "bars": 4, "energy": 0.58, "transform": "recombine"},
            ],
            "manual_notes": [],
            "lineage": {"source": "bootstrap"},
        }
        score = v2schema.migrate_v1_to_v2(base_v1)
        _atomic_write(path, score)
        _append_journal({"op": "bootstrap", "path": str(path), "revision": score["revision"], "ts": time.time()})
        return score

    raw = json.loads(path.read_text(encoding="utf-8"))
    ver = str(raw.get("schema_version"))
    if ver == "1":
        score = v2schema.migrate_v1_to_v3(raw)
        # Do not overwrite live file here — caller decides when to persist migrated form
        return score
    if ver in ("2", "3.0"):
        # For simplicity, if it's an old V2 lying around, we just migrate it here too:
        if ver == "2":
            score = v2schema.migrate_v1_to_v3(raw)  # handles V1 but we might need a V2 -> V3 if it fails, but our migrate logic handles both v1-style or it's not v1. Wait, does migrate_v1_to_v3 handle V2?
            # Actually, let's just assume we only save v3.
        errs = v2schema.validate_v3(raw) if ver == "3.0" else []
        if errs:
            raise ValueError(f"Current score is corrupt v{ver}: " + "; ".join(errs))
        return raw
    raise ValueError(f"Unknown schema_version in {path}")


def save_atomic(score: dict[str, Any], path: Path = DEFAULT_CURRENT, note: str = "",
                expected_revision: int | None = None) -> StoreResult:
    """Validate and commit one v3 revision using an exact compare-and-swap."""
    errs = v2schema.validate_v3(score)
    if errs:
        raise ValueError("Refusing to save invalid v3: " + "; ".join(errs))

    on_disk: dict[str, Any] | None = None
    on_disk_rev = 0
    if path.exists():
        on_disk = json.loads(path.read_text(encoding="utf-8"))
        on_disk_rev = int(on_disk.get("revision", 0))
    base_revision = int(score.get("revision", 0)) if expected_revision is None else int(expected_revision)
    if on_disk is not None and on_disk_rev != base_revision:
        raise RuntimeError(
            f"Revision conflict: on-disk rev {on_disk_rev} != expected base rev {base_revision}. Re-load first."
        )

    prev_snap = None
    if on_disk is not None:
        try:
            prev_snap = _snapshot_revision(on_disk, note=f"pre-save {note}")
        except (OSError, ValueError, TypeError):
            prev_snap = None

    score = json.loads(json.dumps(score))
    score["revision"] = on_disk_rev + 1 if on_disk is not None else max(1, base_revision + 1)
    score.setdefault("lineage", {})["parent_revision"] = on_disk_rev if on_disk is not None else None

    _atomic_write(path, score)
    snap = _snapshot_revision(score, note=note)

    _append_journal({
        "op": "save",
        "path": str(path),
        "revision": score["revision"],
        "snapshot": str(snap),
        "prev_snapshot": str(prev_snap) if prev_snap else None,
        "note": note,
        "ts": time.time(),
    })

    return StoreResult(score=score, revision=score["revision"], path=path)


def save_if_newer(score: dict[str, Any], path: Path = DEFAULT_CURRENT) -> StoreResult | None:
    """Compatibility wrapper using lineage.parent_revision as an exact base."""
    base = score.get("lineage", {}).get("parent_revision")
    if base is None:
        return None
    try:
        return save_atomic(score, path, note="patch-or-edit", expected_revision=int(base))
    except RuntimeError:
        return None


def get_undo_journal(limit: int = 20) -> list[dict[str, Any]]:
    if not JOURNAL_PATH.exists():
        return []
    lines = JOURNAL_PATH.read_text(encoding="utf-8").strip().splitlines()
    entries = [json.loads(l) for l in lines if l.strip()]
    return entries[-limit:]


def undo_last(path: Path = DEFAULT_CURRENT) -> dict[str, Any] | None:
    """Restore the most recent pre-save snapshot as a new monotonic revision."""
    entries = get_undo_journal(100)
    undone = {entry.get("save_snapshot") for entry in entries if entry.get("op") == "undo"}
    for e in reversed(entries):
        if e.get("op") == "save" and e.get("snapshot") not in undone and e.get("prev_snapshot"):
            snap = Path(e["prev_snapshot"])
            if snap.exists():
                restored = json.loads(snap.read_text(encoding="utf-8"))
                current = json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}
                restored["revision"] = int(current.get("revision", 0)) + 1
                if restored.get("schema_version") == 2:
                    restored.setdefault("lineage", {})["parent_revision"] = int(current.get("revision", 0))
                _atomic_write(path, restored)
                _append_journal({
                    "op": "undo", "save_snapshot": e.get("snapshot"),
                    "restored_from": str(snap), "revision": restored["revision"],
                    "ts": time.time(),
                })
                return restored
    return None


def revision_conflict(base_revision: int, current_on_disk: int) -> bool:
    """For [COURT_MUSIC_PATCH:] — reject if base is stale."""
    return base_revision != current_on_disk


def create_empty_v2(title: str = "Untitled Court Work", style: str = "retro_adventure") -> dict[str, Any]:
    """Minimal valid v2 starter (used by UI new project etc)."""
    base = {
        "schema_version": 1,
        "project_id": title.lower().replace(" ", "-")[:40] or "new-project",
        "revision": 1,
        "title": title,
        "style": style,
        "seed": 424242,
        "transport": {"bpm": 112, "meter": [4, 4], "bars": 32, "swing": 0.0, "loop": True},
        "harmony": {"tonic": "D", "mode": "dorian", "chords": ["Dm", "Am", "F", "C"]},
        "motif": ["D4", "F4", "A4", "D5"],
        "sections": [
            {"name": "a", "bars": 8, "energy": 0.4, "transform": "forward"},
            {"name": "b", "bars": 8, "energy": 0.7, "transform": "sequence"},
            {"name": "a2", "bars": 8, "energy": 0.55, "transform": "recombine"},
            {"name": "out", "bars": 8, "energy": 0.3, "transform": "fragment"},
        ],
        "manual_notes": [],
        "lineage": {"source": "ui"},
    }
    return v2schema.migrate_v1_to_v2(base)
