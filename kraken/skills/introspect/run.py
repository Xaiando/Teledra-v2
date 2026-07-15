"""introspect — aggregate the taskforce's own history into an improvement backlog."""

from __future__ import annotations

import glob
import json
import os
import re
from collections import Counter, defaultdict


def _read_journal(root: str) -> list[dict]:
    entries = []
    for path in sorted(glob.glob(os.path.join(root, "journal", "*.jsonl"))):
        try:
            with open(path, "r", encoding="utf-8") as fh:
                for line in fh:
                    line = line.strip()
                    if line:
                        try:
                            entries.append(json.loads(line))
                        except json.JSONDecodeError:
                            continue
        except OSError:
            continue
    return entries


def _norm_reason(reason: str) -> str:
    """Collapse a failure reason to a stable signature for counting."""
    text = str(reason).splitlines()[0].lower()
    text = re.sub(r"[0-9]+", "N", text)
    text = re.sub(r"['\"`].*?['\"`]", "X", text)
    text = re.sub(r"[a-z]:\\\\?[^\s]+", "PATH", text)
    return text[:90].strip()


def _aggregate(root: str, focus: str) -> dict:
    entries = _read_journal(root)
    verdicts = [e for e in entries if e.get("verdict")]
    by_skill_status = defaultdict(Counter)
    reason_counts = Counter()
    reason_examples = {}
    repair_costs = []

    focus_lower = (focus or "").lower()
    focus_words = [w for w in focus_lower.split() if len(w) > 3]
    is_game_focus = any(w in focus_lower for w in ["game", "platformer", "html", "browser", "polish", "captain"])
    for e in verdicts:
        skill = e.get("skill", "?")
        note = str(e.get("notes", "") or e.get("input", "")).lower()
        match = not focus or focus == "all" or skill == focus or any(w in note or w in skill.lower() for w in focus_words)
        if not match and not (is_game_focus and skill == "code_forge"):
            continue
        by_skill_status[skill][e["verdict"]] += 1
        for r in (e.get("reasons") or []):
            sig = _norm_reason(r)
            if sig:
                reason_counts[sig] += 1
                reason_examples.setdefault(sig, (e.get("job", "?"), str(r)[:200]))

    # forge repair cost from the lesson log
    lessons_path = os.path.join(root, "lessons", "code_forge_lessons.jsonl")
    lessons = []
    try:
        with open(lessons_path, "r", encoding="utf-8") as fh:
            for line in fh:
                if line.strip():
                    try:
                        lessons.append(json.loads(line))
                    except json.JSONDecodeError:
                        continue
    except OSError:
        pass
    for les in lessons:
        repair_costs.append((les.get("attempts", 0), bool(les.get("final_ok"))))

    totals = Counter()
    for counter in by_skill_status.values():
        totals.update(counter)

    return {
        "n_verdicts": sum(totals.values()),
        "totals": dict(totals),
        "by_skill": {s: dict(c) for s, c in by_skill_status.items()},
        "top_reasons": reason_counts.most_common(12),
        "reason_examples": reason_examples,
        "forge_repairs": len(repair_costs),
        "forge_failed_after_repair": sum(1 for a, ok in repair_costs if not ok),
    }


def execute(job: dict, ctx: dict) -> dict:
    focus = (job.get("input") or "all").strip() or "all"
    agg = _aggregate(ctx["root"], focus)
    ctx["log"](f"introspect: {agg['n_verdicts']} verdicts, "
               f"{len(agg['top_reasons'])} distinct failure signatures")

    if agg["n_verdicts"] == 0:
        report = (
            f"# Introspection — {focus}\n\n"
            "_Self-authored from 0 journal verdicts._\n\n"
            "## Improvement backlog (ranked)\n\n"
            "1. **Run more game production jobs** (code_forge for platformers like Captain Comic): "
            "Current history is thin for game-specific patterns; queue rebuilds and polishes for browser games "
            "to generate verdicts on button wiring, canvas setup, enemy drops, camera, and repair success.\n\n"
            "2. **Strengthen code_forge repair prompts and skeleton injection**: "
            "Prevent re-introduction of inline handlers, duplicate IDs (e.g. restartBtn), missing tabindex, "
            "and canvas size issues by making skeleton forcing and brutal UI rules even stricter in skills/code_forge/run.py and kernel/game_prompts.py.\n\n"
            "3. **Improve recall and lesson application for HTML games**: "
            "Ensure past UI and platformer lessons are always surfaced and followed for captain-comic-style tasks.\n\n"
            "## Evidence\n\n"
            "Total verdicts analyzed: 0\n"
            "Overall: {}\n\n"
            "Per-skill outcomes:\n(none - insufficient history)\n\n"
            "Forge: 0 repair episodes, 0 still failed after max repairs.\n"
        )
        return {"ok": True, "output": _write(ctx, job, report), "notes": "0 verdicts (structured report)"}

    # deterministic evidence block
    ev_lines = [f"- **{s}**: {c}" for s, c in sorted(
        agg["by_skill"].items(), key=lambda kv: -sum(kv[1].values()))]
    reason_lines = []
    for sig, n in agg["top_reasons"]:
        job_id, sample = agg["reason_examples"].get(sig, ("?", ""))
        reason_lines.append(f"- ({n}x) `{sig}` — e.g. {job_id}: {sample[:120]}")

    evidence = (
        f"Total verdicts analyzed: {agg['n_verdicts']}\n"
        f"Overall: {agg['totals']}\n\n"
        f"Per-skill outcomes:\n" + "\n".join(ev_lines) + "\n\n"
        f"Recurring failure signatures:\n" + "\n".join(reason_lines) + "\n\n"
        f"Forge: {agg['forge_repairs']} repair episodes, "
        f"{agg['forge_failed_after_repair']} still failed after max repairs.\n"
    )

    # qwen ranks and proposes — evidence stays deterministic above
    proposal = ctx["llm"].generate(
        "You are Kraken auditing your OWN operational history to plan self-"
        "improvement. Given the failure evidence below, produce a RANKED "
        "improvement backlog (max 5 items). For each: the pattern, why it "
        "matters, and a concrete proposed fix.\n"
        "STRICT RULES:\n"
        "- ONLY reference these real files: kraken.py, kernel/game_prompts.py, kernel/recall.py, skills/code_forge/run.py, skills/introspect/run.py, harness/game_checks.py, harness/verify_code.py, harness/audit_playability.py, kernel/queue.py, kernel/supervisor.py.\n"
        "- NEVER mention or propose changes to non-existent files (skills/platformer/game.js, kernel/code_forge.py, harness/configurator.py, any .js under skills, src/, etc.). If you can't cite a real file, do not invent one.\n"
        "Good example: 'In skills/code_forge/run.py after loading lessons, if polish task and good on-disk HTML exists, preload it as code= to the initial _prompt so the model receives the full current game + task and uses the improve-existing instructions.'\n"
        "Prioritize: fast reliable initial generation for big Captain Comic HTML (preload disk + skeleton for polish), zero UI bugs on buttons/canvas (addEventListener, unique IDs, literals), perfect enemy death drops + overlap collection, 5+ levels, camera. Be terse and exact.\n\n"
        f"FOCUS: {focus}\n\nEVIDENCE:\n{evidence}",
        system="Only propose edits to real listed Kraken Python files. Goal: make the swarm autonomously produce complete, bug-free, high quality Captain Comic clones on first or second try with no repeated UI or mechanic failures.",
        timeout=180,
    )

    report = (
        f"# Introspection — {focus}\n\n"
        f"_Self-authored from {agg['n_verdicts']} journal verdicts._\n\n"
        f"## Improvement backlog (ranked)\n\n{proposal.strip()}\n\n"
        f"## Evidence\n\n{evidence}"
    )
    out = _write(ctx, job, report)
    return {"ok": True, "output": out,
            "notes": f"{agg['n_verdicts']} verdicts, {len(agg['top_reasons'])} signatures"}


def _write(ctx: dict, job: dict, report: str) -> str:
    vault = os.path.join(ctx["root"], "vault")
    os.makedirs(vault, exist_ok=True)
    path = os.path.join(vault, f"{job['id']}-introspection.md")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(report)
    return os.path.relpath(path, ctx["root"])
