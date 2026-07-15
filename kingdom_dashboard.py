#!/usr/bin/env python3
"""Read-only native observability dashboard for the Teledra kingdom.

The collector is deliberately independent from Tkinter so ``--snapshot-json``
works in headless checks and on machines where a GUI cannot be opened.  All
append-only journals are read from a bounded tail; malformed or concurrently
written records become diagnostics instead of crashing a refresh.
"""

from __future__ import annotations

import argparse
import csv
from dataclasses import dataclass
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import platform
import queue
import socket
import subprocess
import sys
import threading
import time
from typing import Any, Mapping, Sequence
import wave


APP_VERSION = "1.0"
DEFAULT_REFRESH_SECONDS = 5.0
MAX_JSON_BYTES = 1_048_576
MAX_JOURNAL_BYTES = 524_288
MAX_JOURNAL_RECORDS = 24
MAX_DIRECTORY_ENTRIES = 2_000
MAX_DISPLAY_TEXT = 2_000


@dataclass(frozen=True)
class JsonlTail:
    """Result of a byte-bounded JSONL tail read."""

    records: list[dict[str, Any]]
    bytes_read: int
    size_bytes: int
    truncated: bool
    malformed_lines: int
    error: str | None = None

    def metadata(self) -> dict[str, Any]:
        return {
            "bytes_read": self.bytes_read,
            "size_bytes": self.size_bytes,
            "truncated": self.truncated,
            "malformed_lines": self.malformed_lines,
            "error": self.error,
        }


def _clip(value: Any, limit: int = MAX_DISPLAY_TEXT) -> str:
    text = "" if value is None else str(value)
    text = " ".join(text.replace("\x00", "").split())
    if len(text) <= limit:
        return text
    return text[: max(0, limit - 1)] + "…"


def _as_dict(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def _as_list(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []


def _as_float(value: Any, default: float = 0.0) -> float:
    try:
        number = float(value)
    except (TypeError, ValueError):
        return default
    return number if math.isfinite(number) else default


def _confidence(value: Any) -> float:
    number = _as_float(value)
    if 1.0 < number <= 100.0:
        number /= 100.0
    return max(0.0, min(1.0, number))


def _reject_json_constant(value: str) -> None:
    raise ValueError(f"non-finite JSON number '{value}' is not permitted")


def _exact_schema_version(
    raw: Mapping[str, Any], expected: int, *, default: int | None = None
) -> str | None:
    value = raw.get("schema_version", default)
    if isinstance(value, bool) or not isinstance(value, int):
        return "schema_version must be an integer"
    if value != expected:
        return f"unsupported schema_version {value}; expected {expected}"
    return None


def _nonempty_text(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _finite_json_scalar(value: Any) -> Any:
    """Keep display-safe JSON scalars without ever propagating NaN/Infinity."""

    if isinstance(value, float):
        return value if math.isfinite(value) else None
    if isinstance(value, (str, int, bool)) or value is None:
        return value
    return None


def _relative(path: Path, root: Path) -> str:
    try:
        return path.resolve(strict=False).relative_to(root.resolve(strict=False)).as_posix()
    except (OSError, ValueError):
        return str(path)


def _mtime_iso(path: Path) -> str | None:
    try:
        return datetime.fromtimestamp(path.stat().st_mtime, timezone.utc).isoformat()
    except (OSError, OverflowError, ValueError):
        return None


def display_timestamp(value: Any) -> str:
    """Convert Unix seconds/milliseconds or ISO text into concise UTC text."""

    if value is None or value == "":
        return "—"
    number: float | None = None
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        number = float(value)
    elif isinstance(value, str):
        stripped = value.strip()
        try:
            number = float(stripped)
        except ValueError:
            try:
                parsed = datetime.fromisoformat(stripped.replace("Z", "+00:00"))
                if parsed.tzinfo is None:
                    parsed = parsed.replace(tzinfo=timezone.utc)
                return parsed.astimezone(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")
            except ValueError:
                return _clip(stripped, 48) or "—"
    if number is None or not math.isfinite(number):
        return _clip(value, 48) or "—"
    if abs(number) > 10_000_000_000:
        number /= 1_000.0
    try:
        return datetime.fromtimestamp(number, timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")
    except (OverflowError, OSError, ValueError):
        return _clip(value, 48) or "—"


def read_jsonl_tail(
    path: Path,
    *,
    max_bytes: int = MAX_JOURNAL_BYTES,
    max_records: int = MAX_JOURNAL_RECORDS,
) -> JsonlTail:
    """Read only the final ``max_bytes`` of a JSONL file.

    At most one extra byte is inspected to determine whether the bounded read
    begins on a line boundary.  A partially appended final record is ignored as
    malformed, allowing atomic and non-atomic writers to coexist safely.
    """

    max_bytes = max(1, int(max_bytes))
    max_records = max(1, int(max_records))
    try:
        size = path.stat().st_size
        start = max(0, size - max_bytes)
        with path.open("rb") as handle:
            previous = b"\n"
            if start:
                handle.seek(start - 1)
                previous = handle.read(1)
            handle.seek(start)
            raw = handle.read(max_bytes)
    except FileNotFoundError:
        return JsonlTail([], 0, 0, False, 0)
    except OSError as exc:
        return JsonlTail([], 0, 0, False, 0, _clip(exc, 300))

    truncated = start > 0
    if truncated and previous not in {b"\n", b"\r"}:
        newline = raw.find(b"\n")
        raw = b"" if newline < 0 else raw[newline + 1 :]

    records: list[dict[str, Any]] = []
    malformed = 0
    for encoded_line in raw.splitlines():
        if not encoded_line.strip():
            continue
        try:
            value = json.loads(
                encoded_line.decode("utf-8"), parse_constant=_reject_json_constant
            )
        except (UnicodeDecodeError, json.JSONDecodeError, ValueError):
            malformed += 1
            continue
        if isinstance(value, dict):
            records.append(value)
        else:
            malformed += 1
    return JsonlTail(
        records[-max_records:],
        len(raw),
        size,
        truncated,
        malformed,
    )


def read_bounded_json(
    path: Path, *, max_bytes: int = MAX_JSON_BYTES
) -> tuple[dict[str, Any] | None, dict[str, Any]]:
    """Read a bounded JSON object and return data plus non-throwing metadata."""

    metadata: dict[str, Any] = {
        "exists": False,
        "bytes_read": 0,
        "size_bytes": 0,
        "error": None,
    }
    try:
        size = path.stat().st_size
        metadata.update({"exists": True, "size_bytes": size})
        if size > max_bytes:
            metadata["error"] = f"JSON object is {size} bytes; safety limit is {max_bytes}"
            return None, metadata
        with path.open("rb") as handle:
            raw = handle.read(max_bytes + 1)
        metadata["bytes_read"] = len(raw)
        if len(raw) > max_bytes:
            metadata["error"] = f"JSON object grew beyond the {max_bytes}-byte safety limit"
            return None, metadata
        value = json.loads(raw.decode("utf-8"), parse_constant=_reject_json_constant)
        if not isinstance(value, dict):
            metadata["error"] = "JSON root is not an object"
            return None, metadata
        return value, metadata
    except FileNotFoundError:
        return None, metadata
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, ValueError) as exc:
        metadata["error"] = _clip(exc, 300)
        return None, metadata


def _add_diagnostic(
    diagnostics: list[dict[str, str]],
    area: str,
    source: str,
    message: str,
    *,
    severity: str = "warning",
) -> None:
    diagnostics.append(
        {
            "severity": severity,
            "area": area,
            "source": source,
            "message": _clip(message, 500),
        }
    )


def _journal_diagnostics(
    diagnostics: list[dict[str, str]], area: str, source: str, tail: JsonlTail
) -> None:
    if tail.error:
        _add_diagnostic(diagnostics, area, source, tail.error)
    if tail.malformed_lines:
        _add_diagnostic(
            diagnostics,
            area,
            source,
            f"Ignored {tail.malformed_lines} malformed or partially-written JSONL record(s)",
        )


def _task_summary(raw: Mapping[str, Any]) -> dict[str, Any]:
    evidence = _as_dict(raw.get("evidence"))
    positive_evidence = sum(
        1
        for item in _as_list(evidence.get("artifacts"))
        if isinstance(item, dict)
        and item.get("verified") is True
        and _nonempty_text(item.get("reference"))
    ) + sum(
        1
        for item in _as_list(evidence.get("checks"))
        if isinstance(item, dict)
        and item.get("passed") is True
        and _nonempty_text(item.get("name"))
    ) + sum(
        1
        for item in _as_list(evidence.get("sources"))
        if isinstance(item, dict) and _nonempty_text(item.get("url"))
    )
    failure = _as_dict(raw.get("last_failure"))
    return {
        "id": _clip(raw.get("id"), 160),
        "objective": _clip(raw.get("objective"), 600),
        "owner": _clip(raw.get("owner") or "unassigned", 100),
        "role": _clip(raw.get("role") or "unspecified", 100),
        "status": _clip(raw.get("status") or "unknown", 40).lower(),
        "attempt": max(0, int(_as_float(raw.get("attempt")))),
        "max_attempts": max(0, int(_as_float(raw.get("max_attempts")))),
        "priority": max(0, int(_as_float(raw.get("priority")))),
        "dependencies": [_clip(item, 160) for item in _as_list(raw.get("dependencies"))[:24]],
        "updated_at": display_timestamp(raw.get("updated_at_ms")),
        "positive_evidence_items": positive_evidence,
        "last_failure": {
            "code": _clip(failure.get("code"), 120),
            "message": _clip(failure.get("message"), 500),
            "disposition": _clip(failure.get("disposition"), 40),
        }
        if failure
        else None,
    }


def collect_mission(root: Path, diagnostics: list[dict[str, str]]) -> dict[str, Any]:
    snapshot_path = root / "knowledge" / "active_mission.json"
    source = _relative(snapshot_path, root)
    raw, metadata = read_bounded_json(snapshot_path)
    if metadata.get("error"):
        _add_diagnostic(diagnostics, "mission", source, str(metadata["error"]))

    contract_error: str | None = None
    if raw is not None:
        contract_error = _exact_schema_version(raw, 1)
        if contract_error is None and not _nonempty_text(raw.get("id")):
            contract_error = "mission id must be non-empty text"
        if contract_error is None and not _nonempty_text(raw.get("objective")):
            contract_error = "mission objective must be non-empty text"
        if contract_error is None and not _nonempty_text(raw.get("status")):
            contract_error = "mission status must be non-empty text"
        if contract_error is None and not isinstance(raw.get("tasks"), list):
            contract_error = "mission tasks must be an array"
        if contract_error is not None:
            _add_diagnostic(diagnostics, "mission", source, contract_error)
            raw = None

    tasks_raw = _as_list(raw.get("tasks")) if raw else []
    tasks = [_task_summary(item) for item in tasks_raw[:250] if isinstance(item, dict)]
    counts: dict[str, int] = {}
    for item in tasks_raw:
        if not isinstance(item, dict):
            continue
        status = _clip(item.get("status") or "unknown", 40).lower()
        counts[status] = counts.get(status, 0) + 1
    completed = counts.get("completed", 0)
    total = len(tasks_raw)
    mission = {
        "state": "present"
        if raw
        else ("unreadable" if metadata.get("error") or contract_error else "idle"),
        "source": source,
        "source_modified_at": _mtime_iso(snapshot_path),
        "read": metadata,
        "id": _clip(raw.get("id"), 160) if raw else "",
        "objective": _clip(raw.get("objective"), 1_200) if raw else "",
        "owner": _clip(raw.get("owner") or "unassigned", 100) if raw else "",
        "owner_role": _clip(raw.get("owner_role") or "unspecified", 100) if raw else "",
        "status": _clip(raw.get("status") or "idle", 40).lower() if raw else "idle",
        "revision": max(0, int(_as_float(raw.get("revision")))) if raw else 0,
        "updated_at": display_timestamp(raw.get("updated_at_ms")) if raw else "—",
        "compact_synopsis": _clip(raw.get("compact_synopsis"), 1_200) if raw else "",
        "task_counts": counts,
        "task_total": total,
        "tasks_visible": len(tasks),
        "progress_fraction": (completed / total) if total else 0.0,
        "tasks": tasks,
    }

    lifecycle_path = root / "knowledge" / "mission_lifecycle.jsonl"
    lifecycle_tail = read_jsonl_tail(lifecycle_path, max_records=32)
    lifecycle_source = _relative(lifecycle_path, root)
    _journal_diagnostics(diagnostics, "mission", lifecycle_source, lifecycle_tail)
    events: list[dict[str, Any]] = []
    for item in reversed(lifecycle_tail.records[-20:]):
        events.append(
            {
                "event_id": _clip(item.get("event_id"), 160),
                "mission_id": _clip(item.get("mission_id"), 160),
                "task_id": _clip(item.get("task_id"), 160),
                "kind": _clip(item.get("kind") or "unknown", 80),
                "timestamp": display_timestamp(item.get("at_ms") or item.get("timestamp")),
                "attempt": max(0, int(_as_float(item.get("attempt")))),
                "from_status": _clip(item.get("from_status"), 40),
                "to_status": _clip(item.get("to_status"), 40),
                "summary": _clip(item.get("summary"), 800),
                "mission_revision": max(0, int(_as_float(item.get("mission_revision")))),
            }
        )
    mission["lifecycle"] = {
        "source": lifecycle_source,
        "read": lifecycle_tail.metadata(),
        "events": events,
    }
    return mission


def _statement_list(value: Any, limit: int = 6) -> list[str]:
    output: list[str] = []
    for item in _as_list(value)[:limit]:
        if isinstance(item, dict):
            text = item.get("statement") or item.get("summary") or item.get("text")
        else:
            text = item
        if text is not None and _clip(text, 800):
            output.append(_clip(text, 800))
    return output


def _research_summary(raw: Mapping[str, Any]) -> dict[str, Any]:
    sources: list[dict[str, Any]] = []
    for item in _as_list(raw.get("sources"))[:12]:
        if not isinstance(item, dict):
            continue
        sources.append(
            {
                "id": _clip(item.get("id"), 40),
                "title": _clip(item.get("title"), 300),
                "domain": _clip(item.get("domain"), 160),
                "url": _clip(item.get("url"), 1_000),
                "source_kind": _clip(item.get("source_kind") or "unknown", 50),
                "quality": _confidence(item.get("quality")),
            }
        )
    claims: list[dict[str, Any]] = []
    for item in _as_list(raw.get("claims"))[:8]:
        if not isinstance(item, dict):
            continue
        claims.append(
            {
                "statement": _clip(item.get("statement"), 900),
                "source_ids": [_clip(value, 40) for value in _as_list(item.get("source_ids"))[:12]],
                "confidence": _confidence(item.get("confidence")),
            }
        )
    primary_kinds = {"primary", "official", "first_party", "first-party"}
    confidence_value = (
        raw.get("overall_confidence")
        if "overall_confidence" in raw
        else raw.get("confidence")
    )
    return {
        "timestamp": display_timestamp(raw.get("timestamp") or raw.get("created_at")),
        "query": _clip(raw.get("query") or raw.get("topic"), 600),
        "usable": bool(raw.get("usable", bool(claims))),
        "confidence": _confidence(confidence_value),
        "source_count": len(_as_list(raw.get("sources"))),
        "primary_source_count": sum(
            1 for source in sources if source["source_kind"].lower() in primary_kinds
        ),
        "sources": sources,
        "claim_count": len(_as_list(raw.get("claims"))),
        "claims": claims,
        "contradictions": _statement_list(raw.get("contradictions"), 6),
        "unknowns": _statement_list(raw.get("unknowns"), 8),
        "failure": _clip(raw.get("failure"), 800),
    }


def collect_research(root: Path, diagnostics: list[dict[str, str]]) -> dict[str, Any]:
    path = root / "knowledge" / "research_briefs.jsonl"
    source = _relative(path, root)
    tail = read_jsonl_tail(path, max_records=16)
    _journal_diagnostics(diagnostics, "research", source, tail)
    valid_records: list[dict[str, Any]] = []
    for index, item in enumerate(tail.records[-10:]):
        contract_error = _exact_schema_version(item, 1)
        required_lists = ("sources", "claims", "contradictions", "unknowns")
        if contract_error is None and not _nonempty_text(item.get("query")):
            contract_error = "research query must be non-empty text"
        if contract_error is None and not _nonempty_text(item.get("timestamp")):
            contract_error = "research timestamp must be non-empty text"
        if contract_error is None and not isinstance(item.get("usable"), bool):
            contract_error = "research usable must be boolean"
        if contract_error is None:
            invalid_list = next(
                (name for name in required_lists if not isinstance(item.get(name), list)),
                None,
            )
            if invalid_list:
                contract_error = f"research {invalid_list} must be an array"
        if contract_error is not None:
            _add_diagnostic(
                diagnostics,
                "research",
                source,
                f"Ignored record {index + 1}: {contract_error}",
            )
            continue
        valid_records.append(item)
    briefs = [_research_summary(item) for item in reversed(valid_records)]
    latest = briefs[0] if briefs else None
    return {
        "state": "available"
        if briefs
        else ("unreadable" if tail.error or tail.records else "no_data"),
        "source": source,
        "read": tail.metadata(),
        "brief_count_visible": len(briefs),
        "latest_confidence": latest["confidence"] if latest else None,
        "latest_unknown_count": len(latest["unknowns"]) if latest else 0,
        "briefs": briefs,
    }


def collect_failures(root: Path, diagnostics: list[dict[str, str]]) -> dict[str, Any]:
    path = root / "knowledge" / "recursive_failure_reflections.jsonl"
    source = _relative(path, root)
    tail = read_jsonl_tail(path, max_records=40)
    _journal_diagnostics(diagnostics, "failures", source, tail)
    items: list[dict[str, Any]] = []
    recurrence: dict[str, int] = {}
    for raw in reversed(tail.records[-24:]):
        kind = _clip(raw.get("kind") or raw.get("code") or "unknown", 160)
        recurrence[kind] = recurrence.get(kind, 0) + 1
        items.append(
            {
                "timestamp": display_timestamp(
                    raw.get("timestamp") or raw.get("at_ms") or raw.get("created_at")
                ),
                "kind": kind,
                "detail": _clip(raw.get("detail") or raw.get("message"), 1_200),
                "next_reflection": _clip(
                    raw.get("next_reflection") or raw.get("next_action"), 800
                ),
            }
        )
    repeated = [
        {"kind": kind, "count": count}
        for kind, count in sorted(recurrence.items(), key=lambda pair: (-pair[1], pair[0]))
        if count > 1
    ]
    return {
        "state": "available" if items else ("unreadable" if tail.error else "no_data"),
        "source": source,
        "read": tail.metadata(),
        "failure_count_visible": len(items),
        "repeated_kinds": repeated,
        "items": items,
    }


def _newest_file(
    directory: Path,
    predicate: Any,
    *,
    entry_limit: int = MAX_DIRECTORY_ENTRIES,
) -> tuple[Path | None, bool]:
    newest: tuple[int, Path] | None = None
    scanned = 0
    limited = False
    try:
        with os.scandir(directory) as entries:
            for entry in entries:
                scanned += 1
                if scanned > entry_limit:
                    limited = True
                    break
                try:
                    if not entry.is_file(follow_symlinks=False) or not predicate(entry.name):
                        continue
                    modified = entry.stat().st_mtime_ns
                except OSError:
                    continue
                candidate = (modified, Path(entry.path))
                if newest is None or candidate[0] > newest[0]:
                    newest = candidate
    except (FileNotFoundError, NotADirectoryError, OSError):
        return None, False
    return (newest[1] if newest else None), limited


def _safe_render_path(raw_path: Any, root: Path, metadata_path: Path | None) -> Path | None:
    output_root = (root / "Fractus" / "output").resolve(strict=False)
    candidates: list[Path] = []
    if raw_path:
        candidate = Path(str(raw_path))
        if not candidate.is_absolute():
            candidate = root / candidate
        candidates.append(candidate)
    if metadata_path and metadata_path.name.lower().endswith(".json"):
        candidates.append(metadata_path.with_suffix(""))
    for candidate in candidates:
        try:
            resolved = candidate.resolve(strict=False)
            resolved.relative_to(output_root)
            if resolved.is_file() and resolved.suffix.lower() in {".png", ".gif", ".jpg", ".jpeg", ".webp"}:
                return resolved
        except (OSError, ValueError):
            continue
    return None


def collect_fractus(root: Path, diagnostics: list[dict[str, str]]) -> dict[str, Any]:
    fractus_root = root / "Fractus"
    status_path = fractus_root / "fractus_status.json"
    status, status_read = read_bounded_json(status_path, max_bytes=256_000)
    selected_status_path: Path | None = status_path if status else None
    if status_read.get("error"):
        _add_diagnostic(
            diagnostics, "fractus", _relative(status_path, root), str(status_read["error"])
        )
    status_contract_error: str | None = None
    if status is not None:
        status_contract_error = _exact_schema_version(status, 2)
        allowed_states = {
            "accepted",
            "rendering",
            "completed",
            "rejected",
            "cancelled",
            "superseded",
            "closing",
            "pong",
        }
        if status_contract_error is None and not _nonempty_text(status.get("command_id")):
            status_contract_error = "status command_id must be non-empty text"
        if status_contract_error is None and status.get("state") not in allowed_states:
            status_contract_error = "status state is missing or unsupported"
        if status_contract_error is not None:
            _add_diagnostic(
                diagnostics,
                "fractus",
                _relative(status_path, root),
                status_contract_error,
            )
            status = None
            selected_status_path = None

    if status is None and status_contract_error is None:
        archived, limited = _newest_file(
            fractus_root / "status", lambda name: name.lower().endswith(".json"), entry_limit=1_000
        )
        if limited:
            _add_diagnostic(
                diagnostics,
                "fractus",
                "Fractus/status",
                "Status discovery stopped at the 1,000-entry safety limit",
                severity="info",
            )
        if archived:
            archived_status, archived_read = read_bounded_json(archived, max_bytes=256_000)
            if archived_status:
                archived_error = _exact_schema_version(archived_status, 2)
                if archived_error is None and not _nonempty_text(
                    archived_status.get("command_id")
                ):
                    archived_error = "status command_id must be non-empty text"
                if archived_error is None and archived_status.get("state") not in {
                    "accepted",
                    "rendering",
                    "completed",
                    "rejected",
                    "cancelled",
                    "superseded",
                    "closing",
                    "pong",
                }:
                    archived_error = "status state is missing or unsupported"
                if archived_error:
                    status_contract_error = archived_error
                    _add_diagnostic(
                        diagnostics,
                        "fractus",
                        _relative(archived, root),
                        archived_error,
                    )
                else:
                    status, status_read, selected_status_path = (
                        archived_status,
                        archived_read,
                        archived,
                    )
            elif archived_read.get("error"):
                _add_diagnostic(
                    diagnostics, "fractus", _relative(archived, root), str(archived_read["error"])
                )

    metadata_path, output_limited = _newest_file(
        fractus_root / "output",
        lambda name: name.lower().endswith(
            (".png.json", ".jpg.json", ".jpeg.json", ".webp.json")
        ),
    )
    if output_limited:
        _add_diagnostic(
            diagnostics,
            "fractus",
            "Fractus/output",
            f"Render discovery stopped at the {MAX_DIRECTORY_ENTRIES:,}-entry safety limit",
            severity="info",
        )
    render_metadata: dict[str, Any] | None = None
    render_read: dict[str, Any] = {"exists": False, "error": None}
    if metadata_path:
        render_metadata, render_read = read_bounded_json(metadata_path, max_bytes=512_000)
        if render_read.get("error"):
            _add_diagnostic(
                diagnostics,
                "fractus",
                _relative(metadata_path, root),
                str(render_read["error"]),
            )

    metadata_contract_error: str | None = None
    if render_metadata is not None:
        metadata_contract_error = _exact_schema_version(render_metadata, 2)
        if metadata_contract_error is None and render_metadata.get("state") != "completed":
            metadata_contract_error = "render metadata state must be completed"
        if metadata_contract_error is None and not _nonempty_text(
            render_metadata.get("output_path")
        ):
            metadata_contract_error = "render metadata output_path must be non-empty text"
        for dimension in ("width", "height"):
            value = render_metadata.get(dimension)
            if metadata_contract_error is None and (
                isinstance(value, bool) or not isinstance(value, int) or value <= 0
            ):
                metadata_contract_error = f"render metadata {dimension} must be a positive integer"
        if metadata_contract_error is None and not isinstance(
            render_metadata.get("metrics"), dict
        ):
            metadata_contract_error = "render metadata metrics must be an object"
        if metadata_contract_error is not None:
            _add_diagnostic(
                diagnostics,
                "fractus",
                _relative(metadata_path, root) if metadata_path else "Fractus/output",
                metadata_contract_error,
            )
            render_metadata = None

    render_basis = render_metadata or status or {}
    render_path = _safe_render_path(render_basis.get("output_path"), root, metadata_path)
    engine_files = [
        fractus_root / "fractus_gui.py",
        fractus_root / "fractus_registry.py",
        fractus_root / "fractus_render.py",
        fractus_root / "fractus_dsl.py",
        fractus_root / "fractus_protocol.py",
    ]
    missing_engine_files = [_relative(path, root) for path in engine_files if not path.is_file()]
    state = _clip((status or render_metadata or {}).get("state") or "no_status", 60).lower()
    if missing_engine_files or status_contract_error or metadata_contract_error:
        health = "degraded"
    elif state in {"rejected"}:
        health = "attention"
    elif fractus_root.is_dir():
        health = "ready"
    else:
        health = "unavailable"
    return {
        "health": health,
        "engine_present": fractus_root.is_dir(),
        "missing_engine_files": missing_engine_files,
        "schema_version": int(_as_float((status or render_metadata or {}).get("schema_version"))),
        "state": state,
        "command_id": _clip((status or {}).get("command_id"), 160),
        "sequence": max(0, int(_as_float((status or {}).get("sequence")))),
        "source": _clip((status or {}).get("source"), 100),
        "detail": _clip((status or {}).get("detail"), 1_000),
        "updated_at": display_timestamp((status or {}).get("updated_at_unix_ms")),
        "status_source": _relative(selected_status_path, root) if selected_status_path else None,
        "status_read": status_read,
        "last_render": {
            "metadata_source": _relative(metadata_path, root) if metadata_path else None,
            "metadata_modified_at": _mtime_iso(metadata_path) if metadata_path else None,
            "path": _relative(render_path, root) if render_path else None,
            "preview_path": str(render_path) if render_path else None,
            "exists": bool(render_path),
            "width": max(0, int(_as_float(render_basis.get("width")))),
            "height": max(0, int(_as_float(render_basis.get("height")))),
            "duration_ms": max(0, int(_as_float(render_basis.get("duration_ms")))),
            "frame_index": max(0, int(_as_float(render_basis.get("frame_index")))),
            "recipe_hash": _clip(render_basis.get("recipe_hash"), 128),
            "render_hash": _clip(render_basis.get("render_hash"), 128),
            "metrics": {
                _clip(key, 80): _finite_json_scalar(value)
                for key, value in list(_as_dict(render_basis.get("metrics")).items())[:20]
                if isinstance(value, (str, int, float, bool)) or value is None
            },
            "read": render_read,
        },
    }


def _wav_signal(path: Path, root: Path) -> dict[str, Any]:
    signal: dict[str, Any] = {
        "path": _relative(path, root),
        "exists": path.is_file(),
        "valid": False,
        "duration_seconds": 0.0,
        "sample_rate": 0,
        "channels": 0,
        "modified_at": _mtime_iso(path),
        "error": None,
    }
    if not signal["exists"]:
        return signal
    try:
        with wave.open(str(path), "rb") as audio:
            frames = audio.getnframes()
            rate = audio.getframerate()
            signal.update(
                {
                    "valid": frames > 0 and rate > 0 and audio.getnchannels() > 0,
                    "duration_seconds": round(frames / rate, 3) if rate else 0.0,
                    "sample_rate": rate,
                    "channels": audio.getnchannels(),
                }
            )
    except (OSError, EOFError, wave.Error) as exc:
        signal["error"] = _clip(exc, 300)
    return signal


def _count_windows_process(image_name: str) -> tuple[int | None, str | None]:
    flags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    try:
        result = subprocess.run(
            ["tasklist", "/FO", "CSV", "/NH", "/FI", f"IMAGENAME eq {image_name}"],
            capture_output=True,
            text=True,
            timeout=1.5,
            check=False,
            creationflags=flags,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        return None, _clip(exc, 300)
    if result.returncode != 0:
        return None, _clip(result.stderr or f"tasklist exited {result.returncode}", 300)
    count = 0
    try:
        for row in csv.reader(result.stdout.splitlines()):
            if row and row[0].strip().lower() == image_name.lower():
                count += 1
    except csv.Error as exc:
        return None, _clip(exc, 300)
    return count, None


def _probe_local_port(port: int, timeout: float = 0.2) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=timeout):
            return True
    except OSError:
        return False


def collect_runtime(
    root: Path,
    diagnostics: list[dict[str, str]],
    *,
    probe_processes: bool,
) -> dict[str, Any]:
    binaries = [root / "target" / "debug" / "teledra.exe", root / "target" / "release" / "teledra.exe"]
    existing = [path for path in binaries if path.is_file()]
    latest_binary = max(existing, key=lambda path: path.stat().st_mtime_ns) if existing else None
    process_counts: dict[str, int | None] = {"teledra": None, "python": None, "ollama": None}
    process_error: str | None = None
    if probe_processes and platform.system() == "Windows":
        for key, image in (("teledra", "teledra.exe"), ("python", "python.exe"), ("ollama", "ollama.exe")):
            count, error = _count_windows_process(image)
            process_counts[key] = count
            if error and process_error is None:
                process_error = error
    if process_error:
        _add_diagnostic(
            diagnostics,
            "runtime",
            "Windows process table",
            process_error,
            severity="info",
        )
    teledra_count = process_counts.get("teledra")
    if isinstance(teledra_count, int) and teledra_count > 0:
        health = "running"
    elif latest_binary:
        health = "ready"
    else:
        health = "not_built"
    return {
        "health": health,
        "platform": platform.platform(),
        "dashboard_pid": os.getpid(),
        "binary": _relative(latest_binary, root) if latest_binary else None,
        "binary_modified_at": _mtime_iso(latest_binary) if latest_binary else None,
        "process_probe_enabled": probe_processes,
        "process_counts": process_counts,
        "ollama_reachable": _probe_local_port(11434) if probe_processes else None,
        "process_probe_error": process_error,
    }


def collect_tts(
    root: Path,
    runtime: Mapping[str, Any],
    diagnostics: list[dict[str, str]],
) -> dict[str, Any]:
    python_candidates = [root / ".venv" / "Scripts" / "python.exe", root / ".venv" / "bin" / "python"]
    python_executable = next((path for path in python_candidates if path.is_file()), None)
    script = root / "generate_voice.py"
    backend = root / "LuxTTS" / "zipvoice" / "luxvoice.py"
    assets = root / "assets"
    reference_paths: list[Path] = []
    limited = False
    try:
        with os.scandir(assets) as entries:
            for index, entry in enumerate(entries):
                if index >= 128:
                    limited = True
                    break
                if entry.is_file(follow_symlinks=False) and entry.name.lower().endswith("_ref_clean.wav"):
                    reference_paths.append(Path(entry.path))
    except (FileNotFoundError, NotADirectoryError, OSError):
        pass
    references = [_wav_signal(path, root) for path in sorted(reference_paths, key=lambda item: item.name.lower())]
    valid_references = sum(1 for item in references if item["valid"])
    queen_reference = _wav_signal(assets / "queen_ref_clean.wav", root)
    required = {
        "python_executable": bool(python_executable),
        "generator_script": script.is_file(),
        "lux_backend": backend.is_file(),
        "queen_fallback_voice": bool(queen_reference["valid"]),
    }
    missing = [name for name, present in required.items() if not present]
    invalid_references = [item for item in references if not item["valid"]]
    if missing:
        _add_diagnostic(
            diagnostics,
            "tts",
            "local TTS runtime",
            "Missing or invalid: " + ", ".join(missing),
        )
    for item in invalid_references[:5]:
        _add_diagnostic(
            diagnostics,
            "tts",
            item["path"],
            item["error"] or "Reference WAV has no usable audio frames",
        )
    if limited:
        _add_diagnostic(
            diagnostics,
            "tts",
            "assets",
            "Reference discovery stopped at the 128-entry safety limit",
            severity="info",
        )
    timeout_defaults = {
        "startup_seconds": ("TELEDRA_TTS_STARTUP_TIMEOUT_SECS", 120),
        "frame_seconds": ("TELEDRA_TTS_FRAME_TIMEOUT_SECS", 180),
        "total_seconds": ("TELEDRA_TTS_TOTAL_TIMEOUT_SECS", 900),
        "child_exit_seconds": ("TELEDRA_TTS_EXIT_TIMEOUT_SECS", 15),
    }
    configured_timeouts: dict[str, float] = {}
    for name, (environment_name, default) in timeout_defaults.items():
        configured_timeouts[name] = max(0.0, _as_float(os.environ.get(environment_name), default))
    return {
        "health": "ready" if not missing and not invalid_references else "degraded",
        "python_executable": _relative(python_executable, root) if python_executable else None,
        "generator_script": _relative(script, root) if script.is_file() else None,
        "lux_backend": _relative(backend, root) if backend.is_file() else None,
        "required_signals": required,
        "reference_count": len(references),
        "valid_reference_count": valid_references,
        "queen_reference": queen_reference,
        "references": references,
        "last_local_wav": _wav_signal(root / "output.wav", root),
        "configured_timeouts": configured_timeouts,
        "python_process_count": _as_dict(runtime.get("process_counts")).get("python"),
    }


def collect_snapshot(root: Path | str, *, probe_processes: bool = True) -> dict[str, Any]:
    """Collect one complete, JSON-serializable dashboard snapshot."""

    root = Path(root).expanduser().resolve(strict=False)
    diagnostics: list[dict[str, str]] = []
    if not root.is_dir():
        _add_diagnostic(diagnostics, "root", str(root), "Project root does not exist")

    def safe_section(name: str, collector: Any, fallback: dict[str, Any]) -> dict[str, Any]:
        try:
            return collector()
        except Exception as exc:  # Last-resort isolation: one panel cannot kill refresh.
            _add_diagnostic(diagnostics, name, name, f"Collector failed safely: {exc}")
            return fallback

    mission = safe_section("mission", lambda: collect_mission(root, diagnostics), {"state": "unavailable", "tasks": [], "lifecycle": {"events": []}})
    research = safe_section("research", lambda: collect_research(root, diagnostics), {"state": "unavailable", "briefs": []})
    failures = safe_section("failures", lambda: collect_failures(root, diagnostics), {"state": "unavailable", "items": []})
    fractus = safe_section("fractus", lambda: collect_fractus(root, diagnostics), {"health": "unavailable", "state": "unavailable", "last_render": {}})
    runtime = safe_section("runtime", lambda: collect_runtime(root, diagnostics, probe_processes=probe_processes), {"health": "unavailable", "process_counts": {}})
    tts = safe_section("tts", lambda: collect_tts(root, runtime, diagnostics), {"health": "unavailable", "references": []})

    attention = sum(1 for item in diagnostics if item.get("severity") == "warning")
    subsystem_attention = (
        fractus.get("health") in {"attention", "degraded", "unavailable"}
        or tts.get("health") in {"degraded", "unavailable"}
        or runtime.get("health") in {"not_built", "unavailable"}
        or mission.get("status") in {"failed"}
    )
    overall = "attention" if attention or subsystem_attention else "observable"
    return {
        "schema_version": 1,
        "dashboard_version": APP_VERSION,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "project_root": str(root),
        "read_only": True,
        "health": {
            "overall": overall,
            "attention_count": attention,
            "mission_status": mission.get("status", mission.get("state", "unknown")),
            "research_state": research.get("state", "unknown"),
            "visible_failure_count": failures.get("failure_count_visible", 0),
            "fractus": fractus.get("health", "unknown"),
            "tts": tts.get("health", "unknown"),
            "runtime": runtime.get("health", "unknown"),
        },
        "mission": mission,
        "research": research,
        "recursive_failures": failures,
        "fractus": fractus,
        "tts": tts,
        "runtime": runtime,
        "diagnostics": diagnostics,
    }


class KingdomDashboard:
    """Tkinter presentation layer; collection always occurs off the UI thread."""

    COLORS = {
        "background": "#0b0d17",
        "panel": "#15182a",
        "panel_alt": "#1c2036",
        "text": "#eef0ff",
        "muted": "#9da5c9",
        "accent": "#bd8cff",
        "cyan": "#55d6d0",
        "good": "#77d69b",
        "warn": "#ffbf69",
        "bad": "#ff7b88",
        "border": "#343a5d",
    }

    def __init__(
        self,
        tk_root: Any,
        project_root: Path,
        *,
        refresh_seconds: float,
        probe_processes: bool,
    ) -> None:
        import tkinter as tk
        from tkinter import ttk

        self.tk = tk
        self.ttk = ttk
        self.root = tk_root
        self.project_root = project_root
        self.refresh_seconds = max(1.0, refresh_seconds)
        self.probe_processes = probe_processes
        self.results: queue.Queue[tuple[str, Any]] = queue.Queue(maxsize=1)
        self.refreshing = False
        self.closed = False
        self.next_refresh_at = time.monotonic()
        self.latest_snapshot: dict[str, Any] | None = None
        self.research_by_iid: dict[str, dict[str, Any]] = {}
        self.preview_image: Any = None
        self.preview_token: tuple[str, int] | None = None

        self.root.title("Teledra · Kingdom Observatory")
        self.root.geometry("1280x820")
        self.root.minsize(980, 660)
        self.root.configure(background=self.COLORS["background"])
        self.root.protocol("WM_DELETE_WINDOW", self.close)
        self._configure_styles()
        self._build()
        self.root.after(100, self._tick)

    def _configure_styles(self) -> None:
        ttk = self.ttk
        colors = self.COLORS
        style = ttk.Style(self.root)
        if "clam" in style.theme_names():
            style.theme_use("clam")
        style.configure("Root.TFrame", background=colors["background"])
        style.configure("Panel.TFrame", background=colors["panel"])
        style.configure("Card.TFrame", background=colors["panel_alt"], relief="flat")
        style.configure(
            "Title.TLabel",
            background=colors["background"],
            foreground=colors["text"],
            font=("Segoe UI Semibold", 21),
        )
        style.configure(
            "Subtitle.TLabel",
            background=colors["background"],
            foreground=colors["muted"],
            font=("Segoe UI", 9),
        )
        style.configure(
            "CardTitle.TLabel",
            background=colors["panel_alt"],
            foreground=colors["muted"],
            font=("Segoe UI Semibold", 9),
        )
        style.configure(
            "CardValue.TLabel",
            background=colors["panel_alt"],
            foreground=colors["text"],
            font=("Segoe UI Semibold", 15),
        )
        style.configure(
            "CardDetail.TLabel",
            background=colors["panel_alt"],
            foreground=colors["muted"],
            font=("Segoe UI", 8),
        )
        style.configure(
            "Section.TLabel",
            background=colors["panel"],
            foreground=colors["accent"],
            font=("Segoe UI Semibold", 11),
        )
        style.configure(
            "Body.TLabel",
            background=colors["panel"],
            foreground=colors["text"],
            font=("Segoe UI", 9),
        )
        style.configure(
            "Muted.TLabel",
            background=colors["panel"],
            foreground=colors["muted"],
            font=("Segoe UI", 8),
        )
        style.configure(
            "TNotebook",
            background=colors["background"],
            borderwidth=0,
            tabmargins=(0, 6, 0, 0),
        )
        style.configure(
            "TNotebook.Tab",
            background=colors["panel"],
            foreground=colors["muted"],
            padding=(16, 8),
            borderwidth=0,
            font=("Segoe UI Semibold", 9),
        )
        style.map(
            "TNotebook.Tab",
            background=[("selected", colors["panel_alt"])],
            foreground=[("selected", colors["text"])],
        )
        style.configure(
            "Treeview",
            background=colors["panel"],
            fieldbackground=colors["panel"],
            foreground=colors["text"],
            rowheight=27,
            bordercolor=colors["border"],
            borderwidth=0,
            font=("Segoe UI", 9),
        )
        style.configure(
            "Treeview.Heading",
            background=colors["panel_alt"],
            foreground=colors["muted"],
            relief="flat",
            font=("Segoe UI Semibold", 8),
        )
        style.map("Treeview", background=[("selected", "#4a3e70")])
        style.configure(
            "Accent.TButton",
            background=colors["accent"],
            foreground="#171222",
            borderwidth=0,
            padding=(13, 7),
            font=("Segoe UI Semibold", 9),
        )
        style.map("Accent.TButton", background=[("active", "#d0adff")])
        style.configure(
            "Dashboard.TCheckbutton",
            background=colors["background"],
            foreground=colors["muted"],
            font=("Segoe UI", 8),
        )

    def _build(self) -> None:
        tk, ttk = self.tk, self.ttk
        shell = ttk.Frame(self.root, style="Root.TFrame", padding=(18, 14, 18, 16))
        shell.pack(fill="both", expand=True)

        header = ttk.Frame(shell, style="Root.TFrame")
        header.pack(fill="x", pady=(0, 9))
        title_block = ttk.Frame(header, style="Root.TFrame")
        title_block.pack(side="left", fill="x", expand=True)
        ttk.Label(title_block, text="KINGDOM OBSERVATORY", style="Title.TLabel").pack(anchor="w")
        ttk.Label(
            title_block,
            text=f"Read-only telemetry · {self.project_root}",
            style="Subtitle.TLabel",
        ).pack(anchor="w")
        controls = ttk.Frame(header, style="Root.TFrame")
        controls.pack(side="right", anchor="e")
        self.auto_refresh = tk.BooleanVar(value=True)
        ttk.Checkbutton(
            controls,
            text=f"Auto {self.refresh_seconds:g}s",
            variable=self.auto_refresh,
            style="Dashboard.TCheckbutton",
        ).pack(side="left", padx=(0, 10))
        self.refresh_button = ttk.Button(
            controls, text="Refresh now", style="Accent.TButton", command=self.request_refresh
        )
        self.refresh_button.pack(side="left")
        self.refresh_status = tk.StringVar(value="Preparing first observation…")
        ttk.Label(shell, textvariable=self.refresh_status, style="Subtitle.TLabel").pack(
            fill="x", pady=(0, 4)
        )

        cards = ttk.Frame(shell, style="Root.TFrame")
        cards.pack(fill="x", pady=(2, 10))
        for column in range(6):
            cards.columnconfigure(column, weight=1, uniform="cards")
        self.card_values: dict[str, Any] = {}
        self.card_details: dict[str, Any] = {}
        for index, (key, title) in enumerate(
            (
                ("mission", "MISSION"),
                ("research", "RESEARCH"),
                ("failures", "FAILURES"),
                ("fractus", "FRACTUS V2"),
                ("tts", "VOICE / TTS"),
                ("runtime", "RUNTIME"),
            )
        ):
            frame = ttk.Frame(cards, style="Card.TFrame", padding=(12, 10))
            frame.grid(row=0, column=index, sticky="nsew", padx=(0 if index == 0 else 4, 0))
            ttk.Label(frame, text=title, style="CardTitle.TLabel").pack(anchor="w")
            value = ttk.Label(frame, text="—", style="CardValue.TLabel")
            value.pack(anchor="w", pady=(4, 0))
            detail = ttk.Label(frame, text="Waiting", style="CardDetail.TLabel")
            detail.pack(anchor="w")
            self.card_values[key] = value
            self.card_details[key] = detail

        notebook = ttk.Notebook(shell)
        notebook.pack(fill="both", expand=True)
        self._build_overview_tab(notebook)
        self._build_mission_tab(notebook)
        self._build_research_tab(notebook)
        self._build_failure_tab(notebook)
        self._build_systems_tab(notebook)

    def _panel(self, parent: Any, *, padding: tuple[int, int] = (12, 10)) -> Any:
        return self.ttk.Frame(parent, style="Panel.TFrame", padding=padding)

    def _tree(self, parent: Any, columns: Sequence[tuple[str, str, int, str]]) -> tuple[Any, Any]:
        ttk = self.ttk
        frame = ttk.Frame(parent, style="Panel.TFrame")
        names = [item[0] for item in columns]
        tree = ttk.Treeview(frame, columns=names, show="headings", selectmode="browse")
        for name, heading, width, anchor in columns:
            tree.heading(name, text=heading)
            tree.column(name, width=width, minwidth=55, anchor=anchor, stretch=True)
        scroll = ttk.Scrollbar(frame, orient="vertical", command=tree.yview)
        tree.configure(yscrollcommand=scroll.set)
        tree.pack(side="left", fill="both", expand=True)
        scroll.pack(side="right", fill="y")
        return frame, tree

    def _readonly_text(self, parent: Any, *, height: int = 10) -> Any:
        return self.tk.Text(
            parent,
            height=height,
            state="disabled",
            background=self.COLORS["panel"],
            foreground=self.COLORS["text"],
            insertbackground=self.COLORS["text"],
            selectbackground="#4a3e70",
            relief="flat",
            borderwidth=0,
            font=("Consolas", 9),
            wrap="word",
            padx=10,
            pady=8,
        )

    def _build_overview_tab(self, notebook: Any) -> None:
        tab = self._panel(notebook)
        notebook.add(tab, text="Overview")
        tab.columnconfigure(0, weight=3)
        tab.columnconfigure(1, weight=2)
        tab.rowconfigure(1, weight=1)
        self.ttk.Label(tab, text="Mission pulse", style="Section.TLabel").grid(
            row=0, column=0, sticky="w", pady=(0, 7)
        )
        self.ttk.Label(tab, text="Latest signals", style="Section.TLabel").grid(
            row=0, column=1, sticky="w", padx=(12, 0), pady=(0, 7)
        )
        self.overview_mission = self._readonly_text(tab, height=15)
        self.overview_mission.grid(row=1, column=0, sticky="nsew")
        self.overview_signals = self._readonly_text(tab, height=15)
        self.overview_signals.grid(row=1, column=1, sticky="nsew", padx=(12, 0))

    def _build_mission_tab(self, notebook: Any) -> None:
        tab = self._panel(notebook)
        notebook.add(tab, text="Mission & tasks")
        tab.columnconfigure(0, weight=1)
        tab.rowconfigure(2, weight=2)
        tab.rowconfigure(4, weight=1)
        self.mission_header = self.tk.StringVar(value="No active mission snapshot yet.")
        self.ttk.Label(tab, textvariable=self.mission_header, style="Body.TLabel").grid(
            row=0, column=0, sticky="ew", pady=(0, 8)
        )
        self.ttk.Label(tab, text="Task states", style="Section.TLabel").grid(
            row=1, column=0, sticky="w", pady=(0, 5)
        )
        task_frame, self.task_tree = self._tree(
            tab,
            (
                ("status", "STATE", 95, "center"),
                ("task", "TASK", 140, "w"),
                ("owner", "OWNER / ROLE", 145, "w"),
                ("attempt", "ATTEMPT", 75, "center"),
                ("evidence", "EVIDENCE", 75, "center"),
                ("objective", "OBJECTIVE", 430, "w"),
            ),
        )
        task_frame.grid(row=2, column=0, sticky="nsew")
        self.ttk.Label(tab, text="Recent lifecycle", style="Section.TLabel").grid(
            row=3, column=0, sticky="w", pady=(11, 5)
        )
        event_frame, self.event_tree = self._tree(
            tab,
            (
                ("time", "TIME", 165, "w"),
                ("kind", "EVENT", 170, "w"),
                ("task", "TASK", 135, "w"),
                ("transition", "TRANSITION", 130, "center"),
                ("summary", "SUMMARY", 420, "w"),
            ),
        )
        event_frame.grid(row=4, column=0, sticky="nsew")

    def _build_research_tab(self, notebook: Any) -> None:
        tab = self._panel(notebook)
        notebook.add(tab, text="Research")
        tab.columnconfigure(0, weight=2)
        tab.columnconfigure(1, weight=3)
        tab.rowconfigure(1, weight=1)
        self.ttk.Label(tab, text="Structured briefs", style="Section.TLabel").grid(
            row=0, column=0, sticky="w", pady=(0, 5)
        )
        self.ttk.Label(tab, text="Evidence, disagreements & unknowns", style="Section.TLabel").grid(
            row=0, column=1, sticky="w", padx=(12, 0), pady=(0, 5)
        )
        brief_frame, self.research_tree = self._tree(
            tab,
            (
                ("time", "TIME", 150, "w"),
                ("confidence", "CONF.", 70, "center"),
                ("sources", "SOURCES", 75, "center"),
                ("unknowns", "OPEN", 55, "center"),
                ("query", "QUERY", 250, "w"),
            ),
        )
        brief_frame.grid(row=1, column=0, sticky="nsew")
        self.research_tree.bind("<<TreeviewSelect>>", self._research_selected)
        self.research_detail = self._readonly_text(tab, height=20)
        self.research_detail.grid(row=1, column=1, sticky="nsew", padx=(12, 0))

    def _build_failure_tab(self, notebook: Any) -> None:
        tab = self._panel(notebook)
        notebook.add(tab, text="Recovery ledger")
        tab.columnconfigure(0, weight=1)
        tab.rowconfigure(1, weight=1)
        self.failure_header = self.tk.StringVar(value="No recursive failure data observed.")
        self.ttk.Label(tab, textvariable=self.failure_header, style="Body.TLabel").grid(
            row=0, column=0, sticky="ew", pady=(0, 8)
        )
        failure_frame, self.failure_tree = self._tree(
            tab,
            (
                ("time", "TIME", 170, "w"),
                ("kind", "FAILURE KIND", 210, "w"),
                ("detail", "DETAIL", 500, "w"),
                ("next", "REPAIR / NEXT REFLECTION", 330, "w"),
            ),
        )
        failure_frame.grid(row=1, column=0, sticky="nsew")

    def _build_systems_tab(self, notebook: Any) -> None:
        tab = self._panel(notebook)
        notebook.add(tab, text="Systems")
        tab.columnconfigure(0, weight=3)
        tab.columnconfigure(1, weight=2)
        tab.rowconfigure(1, weight=1)
        self.ttk.Label(tab, text="Fractus v2 & last render", style="Section.TLabel").grid(
            row=0, column=0, sticky="w", pady=(0, 5)
        )
        self.ttk.Label(tab, text="TTS & runtime health", style="Section.TLabel").grid(
            row=0, column=1, sticky="w", padx=(12, 0), pady=(0, 5)
        )
        left = self.ttk.Frame(tab, style="Panel.TFrame")
        left.grid(row=1, column=0, sticky="nsew")
        left.rowconfigure(1, weight=1)
        left.columnconfigure(0, weight=1)
        self.fractus_preview = self.ttk.Label(
            left,
            text="No safe local render preview available",
            style="Body.TLabel",
            anchor="center",
        )
        self.fractus_preview.grid(row=0, column=0, sticky="ew", pady=(2, 8))
        self.fractus_detail = self._readonly_text(left, height=12)
        self.fractus_detail.grid(row=1, column=0, sticky="nsew")
        right = self.ttk.Frame(tab, style="Panel.TFrame")
        right.grid(row=1, column=1, sticky="nsew", padx=(12, 0))
        right.rowconfigure(0, weight=2)
        right.rowconfigure(2, weight=1)
        right.columnconfigure(0, weight=1)
        self.system_detail = self._readonly_text(right, height=14)
        self.system_detail.grid(row=0, column=0, sticky="nsew")
        self.ttk.Label(right, text="Collector diagnostics", style="Section.TLabel").grid(
            row=1, column=0, sticky="w", pady=(10, 4)
        )
        self.diagnostic_detail = self._readonly_text(right, height=8)
        self.diagnostic_detail.grid(row=2, column=0, sticky="nsew")

    @staticmethod
    def _clear_tree(tree: Any) -> None:
        children = tree.get_children()
        if children:
            tree.delete(*children)

    @staticmethod
    def _set_text(widget: Any, text: str) -> None:
        widget.configure(state="normal")
        widget.delete("1.0", "end")
        widget.insert("1.0", text)
        widget.configure(state="disabled")

    def request_refresh(self) -> None:
        self.next_refresh_at = time.monotonic()
        self._start_refresh_if_due(force=True)

    def _start_refresh_if_due(self, *, force: bool = False) -> None:
        if self.closed or self.refreshing:
            return
        if not force and (not self.auto_refresh.get() or time.monotonic() < self.next_refresh_at):
            return
        self.refreshing = True
        self.refresh_button.configure(state="disabled")
        self.refresh_status.set("Observing bounded local artifacts…")
        worker = threading.Thread(target=self._collect_worker, name="kingdom-observer", daemon=True)
        worker.start()

    def _collect_worker(self) -> None:
        if self.closed:
            return
        try:
            result = collect_snapshot(self.project_root, probe_processes=self.probe_processes)
            message: tuple[str, Any] = ("ok", result)
        except Exception as exc:
            message = ("error", _clip(exc, 500))
        if self.closed:
            return
        try:
            self.results.put_nowait(message)
        except queue.Full:
            pass

    def _tick(self) -> None:
        if self.closed:
            return
        try:
            kind, payload = self.results.get_nowait()
        except queue.Empty:
            pass
        else:
            self.refreshing = False
            self.refresh_button.configure(state="normal")
            self.next_refresh_at = time.monotonic() + self.refresh_seconds
            if kind == "ok":
                self.latest_snapshot = payload
                self._apply_snapshot(payload)
            else:
                self.refresh_status.set(f"Refresh failed safely: {payload}")
        self._start_refresh_if_due()
        self.root.after(125, self._tick)

    def _card(self, key: str, value: str, detail: str, state: str) -> None:
        color = self.COLORS["text"]
        normalized = state.lower()
        if normalized in {"ready", "running", "observable", "completed", "available", "active"}:
            color = self.COLORS["good"]
        elif normalized in {"attention", "degraded", "failed", "rejected", "unreadable"}:
            color = self.COLORS["warn"]
        elif normalized in {"unavailable", "not_built"}:
            color = self.COLORS["bad"]
        self.card_values[key].configure(text=value, foreground=color)
        self.card_details[key].configure(text=detail)

    def _apply_snapshot(self, snapshot: Mapping[str, Any]) -> None:
        mission = _as_dict(snapshot.get("mission"))
        research = _as_dict(snapshot.get("research"))
        failures = _as_dict(snapshot.get("recursive_failures"))
        fractus = _as_dict(snapshot.get("fractus"))
        tts = _as_dict(snapshot.get("tts"))
        runtime = _as_dict(snapshot.get("runtime"))
        diagnostics = _as_list(snapshot.get("diagnostics"))

        total = int(_as_float(mission.get("task_total")))
        completed = _as_dict(mission.get("task_counts")).get("completed", 0)
        mission_value = str(mission.get("status") or mission.get("state") or "idle").upper()
        self._card("mission", mission_value, f"{completed}/{total} tasks complete", mission_value.lower())
        confidence = research.get("latest_confidence")
        research_value = f"{_as_float(confidence) * 100:.0f}%" if confidence is not None else "NO DATA"
        self._card(
            "research",
            research_value,
            f"{research.get('brief_count_visible', 0)} briefs · {research.get('latest_unknown_count', 0)} unknowns",
            str(research.get("state", "no_data")),
        )
        failure_count = int(_as_float(failures.get("failure_count_visible")))
        self._card(
            "failures",
            str(failure_count),
            f"{len(_as_list(failures.get('repeated_kinds')))} recurring kinds",
            "attention" if failure_count else "ready",
        )
        self._card(
            "fractus",
            str(fractus.get("state", "no_status")).upper(),
            "v2 engine " + str(fractus.get("health", "unknown")),
            str(fractus.get("health", "unknown")),
        )
        self._card(
            "tts",
            str(tts.get("health", "unknown")).upper(),
            f"{tts.get('valid_reference_count', 0)}/{tts.get('reference_count', 0)} references valid",
            str(tts.get("health", "unknown")),
        )
        self._card(
            "runtime",
            str(runtime.get("health", "unknown")).upper(),
            f"Teledra processes: {_as_dict(runtime.get('process_counts')).get('teledra', '—')}",
            str(runtime.get("health", "unknown")),
        )

        self._apply_overview(mission, research, failures, fractus, tts, runtime)
        self._apply_mission(mission)
        self._apply_research(research)
        self._apply_failures(failures)
        self._apply_systems(fractus, tts, runtime, diagnostics)
        generated = display_timestamp(snapshot.get("generated_at"))
        self.refresh_status.set(
            f"Observed {generated} · {len(diagnostics)} diagnostic(s) · bounded, read-only refresh"
        )

    def _apply_overview(
        self,
        mission: Mapping[str, Any],
        research: Mapping[str, Any],
        failures: Mapping[str, Any],
        fractus: Mapping[str, Any],
        tts: Mapping[str, Any],
        runtime: Mapping[str, Any],
    ) -> None:
        task_counts = _as_dict(mission.get("task_counts"))
        count_line = " · ".join(f"{name}: {count}" for name, count in sorted(task_counts.items())) or "No task state recorded"
        mission_text = "\n".join(
            (
                f"{mission.get('id') or 'No active mission'}  [{mission.get('status', 'idle')}]",
                str(mission.get("objective") or "Teledra is currently between durable mission snapshots."),
                "",
                f"Owner: {mission.get('owner') or '—'} / {mission.get('owner_role') or '—'}",
                f"Revision: {mission.get('revision', 0)}  ·  Updated: {mission.get('updated_at', '—')}",
                f"Tasks: {count_line}",
                "",
                str(mission.get("compact_synopsis") or "No compact mission synopsis recorded."),
            )
        )
        latest_brief = (_as_list(research.get("briefs")) or [{}])[0]
        latest_failure = (_as_list(failures.get("items")) or [{}])[0]
        signals = "\n".join(
            (
                "RESEARCH",
                f"{_as_float(_as_dict(latest_brief).get('confidence')) * 100:.0f}% confidence · {_as_dict(latest_brief).get('source_count', 0)} sources",
                _clip(_as_dict(latest_brief).get("query") or "No structured brief yet.", 260),
                "",
                "LATEST FAILURE",
                f"{_as_dict(latest_failure).get('kind', 'None observed')} · {_as_dict(latest_failure).get('timestamp', '—')}",
                _clip(_as_dict(latest_failure).get("detail"), 320),
                "",
                "SYSTEMS",
                f"Fractus {fractus.get('state', 'no_status')} · TTS {tts.get('health', 'unknown')} · Runtime {runtime.get('health', 'unknown')}",
            )
        )
        self._set_text(self.overview_mission, mission_text)
        self._set_text(self.overview_signals, signals)

    def _apply_mission(self, mission: Mapping[str, Any]) -> None:
        self.mission_header.set(
            f"{mission.get('id') or 'No active mission'} · {mission.get('status', 'idle')} · "
            f"revision {mission.get('revision', 0)} · {mission.get('updated_at', '—')}"
        )
        self._clear_tree(self.task_tree)
        for index, task in enumerate(_as_list(mission.get("tasks"))):
            if not isinstance(task, dict):
                continue
            attempt = f"{task.get('attempt', 0)}/{task.get('max_attempts', 0)}"
            self.task_tree.insert(
                "",
                "end",
                iid=f"task-{index}",
                values=(
                    str(task.get("status", "unknown")).upper(),
                    task.get("id", ""),
                    f"{task.get('owner', '')} / {task.get('role', '')}",
                    attempt,
                    task.get("positive_evidence_items", 0),
                    task.get("objective", ""),
                ),
            )
        self._clear_tree(self.event_tree)
        lifecycle = _as_dict(mission.get("lifecycle"))
        for index, event in enumerate(_as_list(lifecycle.get("events"))):
            if not isinstance(event, dict):
                continue
            transition = " → ".join(
                value for value in (str(event.get("from_status") or ""), str(event.get("to_status") or "")) if value
            )
            self.event_tree.insert(
                "",
                "end",
                iid=f"event-{index}",
                values=(
                    event.get("timestamp", "—"),
                    event.get("kind", "unknown"),
                    event.get("task_id", ""),
                    transition,
                    event.get("summary", ""),
                ),
            )

    def _apply_research(self, research: Mapping[str, Any]) -> None:
        self._clear_tree(self.research_tree)
        self.research_by_iid.clear()
        briefs = _as_list(research.get("briefs"))
        for index, brief in enumerate(briefs):
            if not isinstance(brief, dict):
                continue
            iid = f"brief-{index}"
            self.research_by_iid[iid] = brief
            self.research_tree.insert(
                "",
                "end",
                iid=iid,
                values=(
                    brief.get("timestamp", "—"),
                    f"{_as_float(brief.get('confidence')) * 100:.0f}%",
                    f"{brief.get('primary_source_count', 0)}/{brief.get('source_count', 0)}",
                    len(_as_list(brief.get("unknowns"))),
                    brief.get("query", ""),
                ),
            )
        if briefs:
            self.research_tree.selection_set("brief-0")
            self.research_tree.focus("brief-0")
            self._show_research_detail(_as_dict(briefs[0]))
        else:
            self._set_text(
                self.research_detail,
                "No structured research briefs observed.\n\nExpected source: knowledge/research_briefs.jsonl",
            )

    def _research_selected(self, _event: Any) -> None:
        selected = self.research_tree.selection()
        if selected and selected[0] in self.research_by_iid:
            self._show_research_detail(self.research_by_iid[selected[0]])

    def _show_research_detail(self, brief: Mapping[str, Any]) -> None:
        lines = [
            str(brief.get("query") or "Untitled research brief"),
            f"Confidence {_as_float(brief.get('confidence')) * 100:.0f}% · "
            f"usable={brief.get('usable')} · {brief.get('source_count', 0)} source(s)",
        ]
        failure = brief.get("failure")
        if failure:
            lines.extend(("", "SYNTHESIS FAILURE", str(failure)))
        lines.extend(("", "CLAIMS"))
        claims = _as_list(brief.get("claims"))
        if claims:
            for claim in claims:
                claim = _as_dict(claim)
                lines.append(
                    f"• [{_as_float(claim.get('confidence')) * 100:.0f}%] "
                    f"{claim.get('statement', '')}  ({', '.join(_as_list(claim.get('source_ids')))})"
                )
        else:
            lines.append("• No grounded claims recorded.")
        lines.extend(("", "CONTRADICTIONS"))
        contradictions = _as_list(brief.get("contradictions"))
        lines.extend(f"• {item}" for item in contradictions)
        if not contradictions:
            lines.append("• None recorded.")
        lines.extend(("", "WHAT REMAINS UNKNOWN"))
        unknowns = _as_list(brief.get("unknowns"))
        lines.extend(f"• {item}" for item in unknowns)
        if not unknowns:
            lines.append("• No explicit unknowns recorded.")
        lines.extend(("", "SOURCES"))
        for source in _as_list(brief.get("sources")):
            source = _as_dict(source)
            lines.append(
                f"• [{source.get('source_kind', 'unknown')}; {_as_float(source.get('quality')) * 100:.0f}%] "
                f"{source.get('title', '')} — {source.get('domain') or source.get('url', '')}"
            )
        self._set_text(self.research_detail, "\n".join(lines))

    def _apply_failures(self, failures: Mapping[str, Any]) -> None:
        repeated = _as_list(failures.get("repeated_kinds"))
        repeat_text = ", ".join(
            f"{_as_dict(item).get('kind')} ×{_as_dict(item).get('count')}" for item in repeated[:5]
        )
        self.failure_header.set(
            f"{failures.get('failure_count_visible', 0)} recent reflection(s) in bounded tail"
            + (f" · recurring: {repeat_text}" if repeat_text else "")
        )
        self._clear_tree(self.failure_tree)
        for index, item in enumerate(_as_list(failures.get("items"))):
            if not isinstance(item, dict):
                continue
            self.failure_tree.insert(
                "",
                "end",
                iid=f"failure-{index}",
                values=(
                    item.get("timestamp", "—"),
                    item.get("kind", "unknown"),
                    item.get("detail", ""),
                    item.get("next_reflection", ""),
                ),
            )

    def _apply_systems(
        self,
        fractus: Mapping[str, Any],
        tts: Mapping[str, Any],
        runtime: Mapping[str, Any],
        diagnostics: Sequence[Any],
    ) -> None:
        render = _as_dict(fractus.get("last_render"))
        metrics = _as_dict(render.get("metrics"))
        fractus_lines = [
            f"Health: {fractus.get('health', 'unknown')} · State: {fractus.get('state', 'no_status')} · Schema: {fractus.get('schema_version', 0)}",
            f"Command: {fractus.get('command_id') or '—'} · Sequence: {fractus.get('sequence', 0)} · Source: {fractus.get('source') or '—'}",
            f"Updated: {fractus.get('updated_at', '—')}",
            f"Status artifact: {fractus.get('status_source') or 'not present'}",
            "",
            "LAST RENDER",
            f"Artifact: {render.get('path') or 'not present'}",
            f"Metadata: {render.get('metadata_source') or 'not present'}",
            f"Canvas: {render.get('width', 0)} × {render.get('height', 0)} · {render.get('duration_ms', 0)} ms · frame {render.get('frame_index', 0)}",
            f"Recipe: {render.get('recipe_hash') or '—'}",
            f"Render: {render.get('render_hash') or '—'}",
        ]
        if metrics:
            fractus_lines.extend(("", "METRICS"))
            fractus_lines.extend(f"{name}: {value}" for name, value in metrics.items())
        if fractus.get("detail"):
            fractus_lines.extend(("", "DETAIL", str(fractus.get("detail"))))
        if _as_list(fractus.get("missing_engine_files")):
            fractus_lines.extend(("", "MISSING ENGINE FILES"))
            fractus_lines.extend(str(item) for item in _as_list(fractus.get("missing_engine_files")))
        self._set_text(self.fractus_detail, "\n".join(fractus_lines))
        self._update_render_preview(render)

        process_counts = _as_dict(runtime.get("process_counts"))
        system_lines = [
            "VOICE / TTS",
            f"Health: {tts.get('health', 'unknown')}",
            f"Python: {tts.get('python_executable') or 'missing'}",
            f"Generator: {tts.get('generator_script') or 'missing'}",
            f"Backend: {tts.get('lux_backend') or 'missing'}",
            f"Reference WAVs: {tts.get('valid_reference_count', 0)}/{tts.get('reference_count', 0)} valid",
            f"Python processes: {tts.get('python_process_count') if tts.get('python_process_count') is not None else 'not probed'}",
            "",
            "RUNTIME",
            f"Health: {runtime.get('health', 'unknown')}",
            f"Binary: {runtime.get('binary') or 'not built'}",
            f"Binary modified: {runtime.get('binary_modified_at') or '—'}",
            f"Teledra processes: {process_counts.get('teledra', 'not probed')}",
            f"Ollama processes: {process_counts.get('ollama', 'not probed')}",
            f"Ollama localhost:11434: {runtime.get('ollama_reachable') if runtime.get('ollama_reachable') is not None else 'not probed'}",
        ]
        last_wav = _as_dict(tts.get("last_local_wav"))
        if last_wav.get("exists"):
            system_lines.extend(
                (
                    "",
                    "LAST LOCAL WAV",
                    f"{last_wav.get('path')} · valid={last_wav.get('valid')} · {last_wav.get('duration_seconds', 0):.2f}s · {last_wav.get('sample_rate', 0)} Hz",
                    f"Modified: {last_wav.get('modified_at') or '—'}",
                )
            )
        self._set_text(self.system_detail, "\n".join(system_lines))

        diagnostic_lines: list[str] = []
        for item in diagnostics:
            item = _as_dict(item)
            diagnostic_lines.append(
                f"[{str(item.get('severity', 'info')).upper()}] {item.get('area', 'collector')} · "
                f"{item.get('source', '')}\n{item.get('message', '')}"
            )
        self._set_text(
            self.diagnostic_detail,
            "\n\n".join(diagnostic_lines) if diagnostic_lines else "No collector diagnostics. All observed contracts parsed cleanly.",
        )

    def _update_render_preview(self, render: Mapping[str, Any]) -> None:
        """Never decode untrusted render files in the dashboard process.

        The render path and verified metadata remain visible in the Fractus
        panel. Decoding belongs in Fractus itself, where rendering is bounded
        and isolated from this observability UI.
        """

        path_text = render.get("preview_path")
        self.preview_image = None
        self.preview_token = None
        text = (
            f"Render recorded: {_clip(path_text, 180)} · preview disabled for safety"
            if path_text
            else "No safe local render preview available"
        )
        self.fractus_preview.configure(image="", text=text)

    def close(self) -> None:
        self.closed = True
        self.root.destroy()


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Read-only native observability dashboard for Teledra."
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent,
        help="Teledra project root (default: directory containing this script)",
    )
    parser.add_argument(
        "--snapshot-json",
        action="store_true",
        help="Print one headless JSON snapshot and exit",
    )
    parser.add_argument(
        "--refresh-seconds",
        type=float,
        default=DEFAULT_REFRESH_SECONDS,
        help=f"Native UI auto-refresh interval (default: {DEFAULT_REFRESH_SECONDS:g})",
    )
    parser.add_argument(
        "--no-process-probe",
        action="store_true",
        help="Skip tasklist and localhost runtime probes",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    project_root = args.root.expanduser().resolve(strict=False)
    probe_processes = not args.no_process_probe
    if args.snapshot_json:
        snapshot = collect_snapshot(project_root, probe_processes=probe_processes)
        json.dump(
            snapshot,
            sys.stdout,
            ensure_ascii=False,
            indent=2,
            sort_keys=True,
            allow_nan=False,
        )
        sys.stdout.write("\n")
        return 0

    if not math.isfinite(args.refresh_seconds) or args.refresh_seconds < 1.0:
        print("--refresh-seconds must be a finite number of at least 1", file=sys.stderr)
        return 2
    try:
        import tkinter as tk

        root = tk.Tk()
    except Exception as exc:
        print(f"Could not open the native dashboard: {exc}", file=sys.stderr)
        print("Use --snapshot-json for headless verification.", file=sys.stderr)
        return 1
    KingdomDashboard(
        root,
        project_root,
        refresh_seconds=args.refresh_seconds,
        probe_processes=probe_processes,
    )
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
