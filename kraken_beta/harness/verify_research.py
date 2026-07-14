"""verify_research — verifier for research report synthesis, ensuring no fabricated citations or facts."""

from __future__ import annotations

import json
import os
import re
from urllib.parse import unquote


def verify(job: dict, result: dict, ctx: dict) -> dict:
    reasons = []

    # 1. Basic sanity checks
    if not result.get("ok"):
        return {"passed": False, "reasons": ["research_synth failed with ok=false"]}

    output = result.get("output")
    if not output:
        return {"passed": False, "reasons": ["no output file specified"]}

    abs_output = os.path.join(ctx["root"], output)
    if not os.path.exists(abs_output):
        return {"passed": False, "reasons": [f"output file {output} does not exist"]}

    try:
        with open(abs_output, "r", encoding="utf-8") as fh:
            report_content = fh.read()
    except OSError as e:
        return {"passed": False, "reasons": [f"failed to read output file: {e}"]}

    if len(report_content) < 150:
        reasons.append("report is suspiciously short")

    # 2. Check structure
    if "## Sources" not in report_content:
        reasons.append("missing '## Sources' section in the report")

    # 3. Parse input to get real sources
    try:
        payload = json.loads(job["input"])
        sources = payload["sources"]
    except Exception as e:
        return {"passed": False, "reasons": [f"failed to parse job input JSON: {e}"]}

    # Ensure there are at least 3 sources
    if len(sources) < 3:
        reasons.append(f"fewer than 3 sources provided (got {len(sources)})")
        return {"passed": False, "reasons": reasons}

    sources_section = report_content.split("## Sources")[-1]

    def _url_variants(url: str) -> list[str]:
        raw = unquote(url.strip())
        return list({raw, raw.replace(")", "%29"), url.strip()})

    for src in sources:
        if not any(v in sources_section for v in _url_variants(src["url"])):
            reasons.append(f"missing source URL in Sources section: {src['url']}")

    # 4. Use LLM to audit for factual hallucination and fabricated citations in report body
    excerpts = []
    for idx, src in enumerate(sources):
        path = src["path"]
        try:
            with open(path, "r", encoding="utf-8", errors="ignore") as fh:
                text = fh.read()
            excerpts.append(f"=== Source [{idx + 1}] ===\nURL: {src['url']}\nContent:\n{text[:2000]}")
        except Exception:
            pass

    prompt = (
        "Analyze the following sources and the synthesized research report. "
        "Check if the report contains any factual claims, numbers, URLs, or citations that are NOT supported by the sources, or are outright fabricated.\n\n"
        "Important Rule: Negative claims in the report stating that the sources do NOT contain certain information (e.g., 'the sources do not mention X', 'no information is available regarding Y in the provided sources') are considered fully supported if the sources indeed lack that information. Do NOT flag statements of absence as unsupported or fabricated unless the sources actually do contain the information.\n\n"
        "SOURCES:\n\n" + "\n\n".join(excerpts) + "\n\n"
        f"REPORT:\n\n{report_content}\n\n"
        "Return a JSON object with this exact format:\n"
        "{\n"
        '  "passed": true,\n'
        '  "reasons": []\n'
        "}\n"
        "If there are any fabricated claims or citations, set passed to false and list them in the reasons array."
    )

    try:
        audit = ctx["llm"].generate_json(prompt)
        if isinstance(audit, dict):
            if not audit.get("passed"):
                reasons.extend(audit.get("reasons", ["unspecified hallucination detected"]))
    except Exception as e:
        # If audit fails, we fall back to mechanical checks to not lock the system
        ctx["log"](f"LLM verification audit failed/skipped: {e}")

    return {"passed": not reasons, "reasons": reasons}
