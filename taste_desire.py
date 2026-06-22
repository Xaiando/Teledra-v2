"""Persistent Taste & Desire memory shared by Teledra's Rust and Python tools."""

from __future__ import annotations

import argparse
import json
import os
import tempfile
import time
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent
DEFAULT_PATH = ROOT / "knowledge" / "taste_desire.json"
PROMOTE_AFTER = 3
IMMEDIATE_TTL_SECS = 7 * 24 * 60 * 60


def empty_memory() -> dict[str, list[dict[str, Any]]]:
    return {"likes": [], "dislikes": [], "desires": [], "opinions": [], "curiosities": []}


def load_memory(path: os.PathLike[str] | str = DEFAULT_PATH) -> dict[str, Any]:
    target = Path(path)
    try:
        loaded = json.loads(target.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError, TypeError):
        loaded = {}
    memory = empty_memory()
    for key in memory:
        value = loaded.get(key, []) if isinstance(loaded, dict) else []
        memory[key] = value if isinstance(value, list) else []
    return memory


def save_memory(memory: dict[str, Any], path: os.PathLike[str] | str = DEFAULT_PATH) -> None:
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(prefix=target.name + ".", suffix=".tmp", dir=target.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as handle:
            json.dump(memory, handle, ensure_ascii=True, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(tmp_name, target)
    finally:
        if os.path.exists(tmp_name):
            os.unlink(tmp_name)


def _clean(value: Any, limit: int = 240) -> str:
    return " ".join(str(value or "").split())[:limit]


def _strength(value: Any, default: float = 0.5) -> float:
    try:
        return round(max(0.0, min(1.0, float(value))), 3)
    except (TypeError, ValueError):
        return default


def _key(value: Any) -> str:
    return _clean(value).casefold()


def decay_immediate(memory: dict[str, Any], now: int | None = None) -> None:
    now = int(time.time() if now is None else now)
    kept = []
    for desire in memory.get("desires", []):
        if desire.get("kind") == "persistent":
            kept.append(desire)
            continue
        last_seen = int(desire.get("last_seen_ts", desire.get("born_ts", 0)) or 0)
        age = max(0, now - last_seen)
        desire["strength"] = round(_strength(desire.get("strength")) * max(0.0, 1.0 - age / IMMEDIATE_TTL_SECS), 3)
        if age <= IMMEDIATE_TTL_SECS and desire["strength"] >= 0.05:
            kept.append(desire)
    memory["desires"] = kept


def apply_event(
    event: dict[str, Any],
    path: os.PathLike[str] | str = DEFAULT_PATH,
    now: int | None = None,
) -> dict[str, Any]:
    """Apply one reflection event and atomically persist the resulting memory."""
    now = int(time.time() if now is None else now)
    memory = load_memory(path)
    decay_immediate(memory, now)
    kind = _clean(event.get("type"), 32).casefold()
    source = _clean(event.get("source") or "reflection", 80)

    if kind in {"like", "dislike"}:
        bucket = "likes" if kind == "like" else "dislikes"
        subject = _clean(event.get("subject"))
        if subject:
            existing = next((x for x in memory[bucket] if _key(x.get("subject")) == _key(subject)), None)
            incoming = _strength(event.get("strength"), 0.6)
            if existing:
                existing["strength"] = round(min(1.0, _strength(existing.get("strength")) + incoming * 0.2), 3)
                existing["why"] = _clean(event.get("why") or existing.get("why"))
                existing["source"] = source
                existing["ts"] = now
                existing["seen_count"] = int(existing.get("seen_count", 1)) + 1
            else:
                memory[bucket].append({
                    "subject": subject,
                    "why": _clean(event.get("why")),
                    "strength": incoming,
                    "source": source,
                    "ts": now,
                    "seen_count": 1,
                })

    elif kind == "desire":
        want = _clean(event.get("want"))
        if want:
            existing = next((x for x in memory["desires"] if _key(x.get("want")) == _key(want)), None)
            incoming_kind = _clean(event.get("kind") or "immediate", 16).casefold()
            incoming_kind = "persistent" if incoming_kind == "persistent" else "immediate"
            incoming = _strength(event.get("strength"), 0.55)
            if existing:
                recurrence = int(existing.get("recurrence", 1)) + 1
                existing.update({
                    "last_seen_ts": now,
                    "recurrence": recurrence,
                    "source": source,
                    "status": _clean(event.get("status") or existing.get("status") or "open", 20),
                    "strength": round(min(1.0, _strength(existing.get("strength")) + incoming * 0.15), 3),
                })
                if incoming_kind == "persistent" or recurrence >= PROMOTE_AFTER:
                    existing["kind"] = "persistent"
                    existing["promoted_ts"] = now
            else:
                memory["desires"].append({
                    "want": want,
                    "kind": incoming_kind,
                    "status": _clean(event.get("status") or "open", 20),
                    "strength": incoming,
                    "born_ts": now,
                    "last_seen_ts": now,
                    "progress": _clean(event.get("progress")),
                    "source": source,
                    "recurrence": 1,
                })

    elif kind == "opinion":
        claim = _clean(event.get("claim"))
        if claim:
            existing = next((x for x in memory["opinions"] if _key(x.get("claim")) == _key(claim)), None)
            payload = {"claim": claim, "confidence": _strength(event.get("confidence"), 0.55), "source": source, "ts": now}
            if existing:
                existing.update(payload)
            else:
                memory["opinions"].append(payload)

    elif kind == "curiosity":
        question = _clean(event.get("question"))
        if question:
            existing = next((x for x in memory["curiosities"] if _key(x.get("question")) == _key(question)), None)
            payload = {"question": question, "source": source, "ts": now}
            if existing:
                existing.update(payload)
            else:
                memory["curiosities"].append(payload)

    for bucket, identity in (("likes", "subject"), ("dislikes", "subject"), ("desires", "want"), ("opinions", "claim"), ("curiosities", "question")):
        memory[bucket] = sorted(
            memory[bucket],
            key=lambda x: (_strength(x.get("strength", x.get("confidence", 0.5))), int(x.get("last_seen_ts", x.get("ts", 0)) or 0)),
            reverse=True,
        )[:100]
    save_memory(memory, path)
    return memory


def prompt_context(path: os.PathLike[str] | str = DEFAULT_PATH) -> str:
    memory = load_memory(path)
    decay_immediate(memory)
    active = [d for d in memory["desires"] if d.get("status", "open") in {"open", "pursuing"}]
    active.sort(key=lambda d: (_strength(d.get("strength")), int(d.get("last_seen_ts", 0))), reverse=True)
    likes = sorted(memory["likes"], key=lambda x: _strength(x.get("strength")), reverse=True)[:3]
    dislikes = sorted(memory["dislikes"], key=lambda x: _strength(x.get("strength")), reverse=True)[:2]
    lines = []
    if active:
        lines.append(f"ACTIVE DESIRE (pursue exactly this one): {active[0].get('want')} [{active[0].get('kind', 'immediate')}]")
    if likes:
        lines.append("CURRENT LIKES: " + "; ".join(_clean(x.get("subject")) for x in likes))
    if dislikes:
        lines.append("CURRENT DISLIKES: " + "; ".join(_clean(x.get("subject")) for x in dislikes))
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--path", default=str(DEFAULT_PATH))
    parser.add_argument("--context", action="store_true")
    parser.add_argument("--event", help="JSON reflection event")
    args = parser.parse_args()
    if args.event:
        print(json.dumps(apply_event(json.loads(args.event), args.path), ensure_ascii=True))
    elif args.context:
        print(prompt_context(args.path))
    else:
        print(json.dumps(load_memory(args.path), ensure_ascii=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
