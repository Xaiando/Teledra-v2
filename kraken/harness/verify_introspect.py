"""verify_introspect — the self-audit must be evidence-grounded, not vibes."""

from __future__ import annotations

import os


def verify(job: dict, result: dict, ctx: dict) -> dict:
    reasons = []
    output = result.get("output")
    if not result.get("ok") or not output:
        return {"passed": False, "reasons": ["introspect produced no output"]}
    path = output if os.path.isabs(output) else os.path.join(ctx["root"], output)
    if not os.path.exists(path):
        return {"passed": False, "reasons": [f"output missing: {output}"]}
    with open(path, "r", encoding="utf-8", errors="ignore") as fh:
        text = fh.read()
    # a real introspection has both a backlog and the deterministic evidence
    for heading in ("## Improvement backlog", "## Evidence"):
        if heading not in text:
            reasons.append(f"missing section: {heading}")
    if "Total verdicts analyzed" not in text and "No journal history" not in text:
        reasons.append("evidence block lacks verdict counts")
    return {"passed": not reasons, "reasons": reasons}
