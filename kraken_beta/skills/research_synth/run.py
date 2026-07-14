"""research_synth â€” synthesize web research into a final cited report."""

from __future__ import annotations

import json
import os

from kraken.kernel import query_guard


def execute(job: dict, ctx: dict) -> dict:
    llm = ctx["llm"]
    log = ctx["log"]

    try:
        payload = json.loads(job["input"])
        question = payload["question"]
        sources = payload["sources"]
    except Exception as e:
        return {
            "ok": False,
            "notes": f"Invalid input JSON payload: {e}"
        }

    if not sources:
        report = f"# {question}\n\nNo sources were successfully fetched, so no synthesis can be performed.\n"
        out_path = _write(ctx, job, report)
        return {
            "ok": True,
            "output": out_path,
            "notes": "No sources available for synthesis"
        }

    # Load source texts
    excerpts = []
    for idx, src in enumerate(sources):
        path = src["path"]
        url = src["url"]
        title = src["title"]
        try:
            with open(path, "r", encoding="utf-8", errors="ignore") as fh:
                text = fh.read()
            excerpts.append(
                f"=== Source [{idx + 1}] ===\n"
                f"Title: {title}\n"
                f"URL: {url}\n"
                f"Content:\n{text[:4000]}"
            )
        except OSError as e:
            log(f"Error reading source file {path}: {e}")

    if excerpts and not _sources_relevant(question, excerpts, llm, log):
        log("relevance gate: fetched sources are tangential to the question")
        lines = [
            f"# Research Report: {question}\n",
            "The fetched sources do not directly address this question "
            "(shared vocabulary only, no substantive answer).\n",
            "## Sources\n",
        ]
        for idx, src in enumerate(sources):
            lines.append(f"{idx + 1}. [{src['title']}]({src['url']})")
        report = "\n".join(lines) + "\n"
        out_path = _write(ctx, job, report)
        return {
            "ok": True,
            "output": out_path,
            "notes": "sources tangential to question; synthesis skipped",
        }

    log(f"Synthesizing report for {len(sources)} sources")

    prompt = (
        "You are Kraken, a precise research synthesis assistant. Synthesize a brief report answering the question using ONLY the provided sources.\n"
        "Instructions:\n"
        "1. Answer the question objectively and directly.\n"
        "2. Cite your statements using bracketed numbers corresponding to the source index (e.g. [1], [2]).\n"
        "3. Do not invent any facts or citations not present in the sources.\n"
        "4. Keep the synthesis under 400 words.\n\n"
        f"QUESTION: {question}\n\n"
        "SOURCES:\n\n" + "\n\n".join(excerpts)
    )

    answer = llm.generate(
        prompt,
        system="You are Kraken, the Teledra kingdom's silent research assistant. Precise, honest, no flattery.",
    )

    # Format final report
    report_lines = [
        f"# Research Report: {question}\n",
        answer.strip() + "\n",
        "## Sources\n"
    ]
    for idx, src in enumerate(sources):
        report_lines.append(f"{idx + 1}. [{src['title']}]({src['url']})")

    report = "\n".join(report_lines) + "\n"
    out_path = _write(ctx, job, report)

    return {
        "ok": True,
        "output": out_path,
        "notes": f"Synthesized report with {len(sources)} sources cited"
    }


def _sources_relevant(question: str, excerpts: list[str], llm, log) -> bool:
    terms = query_guard.research_terms(question)
    if len(terms) >= 2:
        for text in excerpts:
            matched = query_guard.terms_match_text(terms, text)
            need = len(terms) if len(terms) <= 2 else max(2, (len(terms) * 2 + 2) // 3)
            if matched >= need:
                log(f"relevance gate mechanical bypass: {matched}/{len(terms)} whole-word terms.")
                return True

    # LLM fallback
    verdict = llm.generate(
        "Do these sources contain material that DIRECTLY helps answer the "
        "question (not just shared vocabulary)? Reply ONLY yes or no.\n\n"
        f"QUESTION: {question}\n\nSOURCES:\n\n"
        + "\n\n".join(excerpts[:3]),
        timeout=90,
    ).strip().lower()
    log(f"relevance gate verdict: {verdict[:40]}")
    return verdict.startswith("yes")


def _write(ctx: dict, job: dict, report: str) -> str:
    vault = os.path.join(ctx["root"], "vault")
    os.makedirs(vault, exist_ok=True)
    path = os.path.join(vault, f"{job['id']}-report.md")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(report)
    return os.path.relpath(path, ctx["root"])
