"""Kraken harness dispatch — mechanical verification before anything ships.

A skill names its harness in SKILL.md frontmatter (harness: verify_research).
Harness modules live in kraken/harness/ and expose verify(job, result, ctx)
-> {"passed": bool, "reasons": [...]}. No harness named = a basic sanity check.
"""

from __future__ import annotations

import importlib.util
import os


def verify(job: dict, result: dict, ctx: dict, harness_name: str) -> dict:
    if harness_name:
        path = os.path.join(ctx["root"], "harness", harness_name + ".py")
        if os.path.exists(path):
            spec = importlib.util.spec_from_file_location(f"kraken_{harness_name}", path)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            return module.verify(job, result, ctx)
        return {"passed": False, "reasons": [f"harness {harness_name} not found"]}
    return _basic(result, ctx)


def _basic(result: dict, ctx: dict) -> dict:
    reasons = []
    if not result.get("ok"):
        reasons.append("skill reported ok=false")
    output = result.get("output")
    if output:
        path = output if os.path.isabs(output) else os.path.join(ctx["root"], output)
        if not os.path.exists(path):
            reasons.append(f"declared output missing: {output}")
        elif os.path.getsize(path) < 40:
            reasons.append(f"output suspiciously small: {output}")
    return {"passed": not reasons, "reasons": reasons}
