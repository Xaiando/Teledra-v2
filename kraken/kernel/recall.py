"""Kraken recall — retrieval over the system's own operational history.

The lesson logs (lessons/*.jsonl) are the seed corpus for self-improvement,
but a seed corpus that nothing reads is inert. recall lets a skill pull the
most relevant past lessons for the task at hand and feed them forward — so
the taskforce stops repeating mistakes it has already paid for.

This is the flywheel: forge -> fail -> repair -> log -> RECALL -> forge better.
Pure stdlib, offline, no model call — cheap enough to run before every forge.
"""

from __future__ import annotations

import json
import os
import re

STOP = {"the", "a", "an", "and", "or", "of", "to", "in", "on", "for", "is",
        "are", "write", "return", "returning", "function", "that", "with",
        "given", "value", "values", "using", "use", "list", "string", "int"}


def _terms(text: str) -> set[str]:
    words = re.findall(r"[a-zA-Z][a-zA-Z0-9_]{2,}", (text or "").lower())
    return {w for w in words if w not in STOP}


def _load_lessons(root: str, name: str) -> list[dict]:
    path = os.path.join(root, "lessons", name)
    out: list[dict] = []
    try:
        with open(path, "r", encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if line:
                    try:
                        out.append(json.loads(line))
                    except json.JSONDecodeError:
                        continue
    except OSError:
        pass
    return out


def code_lessons(root: str, task: str, k: int = 3) -> list[dict]:
    """Most relevant past code_forge lessons for `task`.

    Scores by term overlap with the past task text. Lessons that record a
    REPAIR (had failure reasons) are the most instructive, so they get a
    boost — the point is to surface 'here's what bit us and how we fixed it'.
    Now also loads game (HTML) lessons when the task looks game-related.
    """
    want = _terms(task)
    if not want:
        return []
    scored = []
    files = ["code_forge_lessons.jsonl"]
    # include rich/game lessons when task smells like a game (html / canvas / platformer etc)
    if any(x in (task or "").lower() for x in ("html", "canvas", "game", "platform", "shooter", "jump", "captain", "index.html")):
        files.append("code_forge_game_lessons.jsonl")
    for fname in files:
        for lesson in _load_lessons(root, fname):
            have = _terms(lesson.get("task", ""))
            overlap = len(want & have)
            if not overlap:
                continue
            boost = 2 if lesson.get("reasons") else 0
            if fname.endswith("game_lessons.jsonl"):
                boost += 1  # slight extra for game-specific hard-won knowledge
            scored.append((overlap + boost, lesson))
    scored.sort(key=lambda pair: pair[0], reverse=True)
    # de-dup by job_id if present
    seen_ids = set()
    out = []
    for score, les in scored:
        jid = les.get("job_id")
        if jid and jid in seen_ids:
            continue
        seen_ids.add(jid or str(les))
        out.append(les)
        if len(out) >= k:
            break
    return out


def format_code_lessons(lessons: list[dict]) -> str:
    """Render lessons as a compact 'avoid these past mistakes' briefing."""
    if not lessons:
        return ""
    blocks = []
    for lesson in lessons:
        reasons = lesson.get("reasons") or []
        first = reasons[0].splitlines()[0] if reasons else "(passed clean)"
        outcome = "eventually passed" if lesson.get("final_ok") else "still failed"
        blocks.append(
            f"- Past task: {lesson.get('task', '')[:160]}\n"
            f"  What bit it: {first[:200]}\n"
            f"  Outcome after {lesson.get('attempts', '?')} repair(s): {outcome}"
        )
    return ("HARD-WON LESSONS FROM PAST FORGES (avoid repeating these mistakes):\n"
            + "\n".join(blocks) + "\n")
