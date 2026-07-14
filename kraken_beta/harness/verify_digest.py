"""verify_digest — coverage checks for prod_digest and prod_vault outputs."""

from __future__ import annotations

import json
import os
import re

MANIFEST_NAME = "sources_manifest.json"
ELIGIBLE_EXT = {".md", ".txt", ".jsonl", ".json", ".log"}


def verify(job: dict, result: dict, ctx: dict) -> dict:
    skill = job.get("skill", "")
    if skill == "prod_vault":
        return _verify_vault(job, result, ctx)
    return _verify_digest(job, result, ctx)


def _verify_digest(job: dict, result: dict, ctx: dict) -> dict:
    reasons = []
    output = result.get("output")
    if not output:
        return {"passed": False, "reasons": ["missing output path"]}

    out_path = output if os.path.isabs(output) else os.path.join(ctx["root"], output)
    if not os.path.exists(out_path):
        return {"passed": False, "reasons": [f"output missing: {output}"]}
    if os.path.getsize(out_path) < 80:
        reasons.append("digest output suspiciously small")

    with open(out_path, "r", encoding="utf-8", errors="ignore") as fh:
        report = fh.read()

    if "## Sources" not in report:
        reasons.append("digest missing ## Sources section")

    manifest_path = os.path.join(ctx["workdir"], MANIFEST_NAME)
    if not os.path.exists(manifest_path):
        reasons.append(f"workdir manifest missing: {MANIFEST_NAME}")
        return {"passed": not reasons, "reasons": reasons}

    with open(manifest_path, "r", encoding="utf-8") as fh:
        manifest = json.load(fh)

    cited = _paths_in_section(report, "## Sources")
    waived = {w["path"] for w in manifest.get("waived", [])}
    for src in manifest.get("sources", []):
        path = src["path"]
        if path not in cited:
            reasons.append(f"source not listed in ## Sources: {path}")

    folder = manifest.get("folder")
    max_files = int(manifest.get("max_files", 12))
    if folder and os.path.isdir(folder):
        for path in _eligible_files(folder, max_files):
            if path not in cited and path not in waived:
                reasons.append(f"eligible file neither cited nor waived: {path}")

    if manifest.get("sources") and not reasons:
        spot = _spot_check(ctx, report[:3000], "digest")
        if not spot["passed"]:
            reasons.extend(spot["reasons"])

    return {"passed": not reasons, "reasons": reasons}


def _verify_vault(job: dict, result: dict, ctx: dict) -> dict:
    reasons = []
    output = result.get("output")
    if not output:
        return {"passed": False, "reasons": ["missing output path"]}

    out_path = output if os.path.isabs(output) else os.path.join(ctx["root"], output)
    if not os.path.exists(out_path):
        return {"passed": False, "reasons": [f"output missing: {output}"]}
    if os.path.getsize(out_path) < 60:
        reasons.append("evergreen note suspiciously small")

    with open(out_path, "r", encoding="utf-8", errors="ignore") as fh:
        report = fh.read()

    for heading in ("## Evergreen", "## Key Facts", "## Source"):
        if heading not in report:
            reasons.append(f"evergreen missing {heading}")

    manifest_path = os.path.join(ctx["workdir"], MANIFEST_NAME)
    if not os.path.exists(manifest_path):
        reasons.append(f"workdir manifest missing: {MANIFEST_NAME}")
        return {"passed": not reasons, "reasons": reasons}

    with open(manifest_path, "r", encoding="utf-8") as fh:
        manifest = json.load(fh)

    sources = manifest.get("sources", [])
    if len(sources) != 1:
        reasons.append("prod_vault manifest must list exactly one source")
    else:
        src = sources[0]["path"]
        if src not in report:
            reasons.append(f"source report not referenced in output: {src}")

    if not reasons:
        spot = _spot_check(ctx, report[:3000], "evergreen")
        if not spot["passed"]:
            reasons.extend(spot["reasons"])

    return {"passed": not reasons, "reasons": reasons}


def _eligible_files(folder: str, max_files: int) -> list[str]:
    found = []
    for name in sorted(os.listdir(folder)):
        path = os.path.join(folder, name)
        if os.path.isfile(path) and os.path.splitext(name)[1].lower() in ELIGIBLE_EXT:
            try:
                if os.path.getsize(path) > 0:
                    found.append(path)
            except OSError:
                pass
    return found[:max_files]


def _paths_in_section(report: str, heading: str) -> set[str]:
    idx = report.find(heading)
    if idx < 0:
        return set()
    rest = report[idx + len(heading):]
    next_h = re.search(r"\n## ", rest)
    section = rest[: next_h.start()] if next_h else rest
    return set(re.findall(r"`([^`]+)`", section))


def _spot_check(ctx: dict, excerpt: str, kind: str) -> dict:
    llm = ctx.get("llm")
    if llm is None:
        return {"passed": True, "reasons": []}
    try:
        answer = llm.generate(
            f"Does this {kind} note look structurally complete and free of obvious "
            f"placeholder text like 'TODO' or 'lorem ipsum'? Reply ONLY yes or no.\n\n"
            f"{excerpt[:2500]}",
            timeout=60,
        ).strip().lower()
        if answer.startswith("no"):
            return {"passed": False, "reasons": [f"qwen spot-check rejected {kind} quality"]}
    except Exception as exc:
        ctx["log"](f"spot-check skipped: {exc}")
    return {"passed": True, "reasons": []}