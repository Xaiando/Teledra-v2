"""verify_coding_mcp — confinement and report checks for coding_mcp jobs."""

from __future__ import annotations

import json
import os
import re


def verify(job: dict, result: dict, ctx: dict) -> dict:
    reasons: list[str] = []

    if not result.get("ok"):
        return {"passed": True, "reasons": []}

    output = result.get("output")
    if not output:
        return {"passed": False, "reasons": ["no output file specified"]}

    abs_output = os.path.join(ctx["root"], output)
    if not os.path.exists(abs_output):
        return {"passed": False, "reasons": [f"output file {output} does not exist"]}

    try:
        with open(abs_output, "r", encoding="utf-8") as fh:
            report = fh.read()
    except OSError as exc:
        return {"passed": False, "reasons": [f"failed to read output: {exc}"]}

    if len(report.strip()) < 8:
        reasons.append("report is suspiciously short")

    raw = (job.get("input") or "").strip().lstrip("\ufeff")
    if raw.startswith("{"):
        try:
            payload = json.loads(raw)
        except json.JSONDecodeError:
            payload = {}
    else:
        payload = {}

    op = str(payload.get("op") or "")
    path = str(payload.get("path") or "")
    if op in {"read", "tree", "search", "py_compile", "run_tests", "git_status"}:
        if ".." in path.replace("\\", "/"):
            reasons.append("path traversal attempt in job input should not pass")

    result_json = os.path.join(ctx["workdir"], "coding_mcp_result.json")
    if os.path.exists(result_json):
        try:
            with open(result_json, "r", encoding="utf-8") as fh:
                meta = json.load(fh)
            ran_op = str(meta.get("op") or "")
            if op and ran_op and op != ran_op:
                reasons.append(f"requested op {op} but skill ran {ran_op}")
        except (OSError, json.JSONDecodeError):
            pass

    root = os.path.abspath(ctx["root"])
    workspace = os.path.abspath(ctx.get("workspace") or os.path.join(root, "workspace"))
    for match in re.findall(r"[A-Za-z]:\\\\[^\\s\"']+", report):
        norm = os.path.normcase(os.path.normpath(match))
        if not (norm.startswith(os.path.normcase(root)) or norm.startswith(os.path.normcase(workspace))):
            reasons.append(f"report leaked path outside kraken/workspace: {match}")

    return {"passed": not reasons, "reasons": reasons}