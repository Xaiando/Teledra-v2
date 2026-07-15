"""research_fanout — decompose a broad question, delegate, then synthesize."""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "..")))
from kraken.kernel.queue import Queue


def execute(job: dict, ctx: dict) -> dict:
    if job["input"].startswith("SYNTH:"):
        return _synthesize(job, ctx)
    return _fanout(job, ctx)


def _fanout(job: dict, ctx: dict) -> dict:
    llm = ctx["llm"]
    subs = llm.generate_json(
        "Split this research question into 2 to 4 focused sub-questions that "
        "together cover it. Each sub-question MUST be fully self-contained: "
        "repeat the original subject explicitly (e.g. 'the Teledra kingdom's "
        "music system'), never a bare 'it', 'the system', or generic 'the "
        "kingdom'. Answer as a JSON array of strings, nothing else.\n\n"
        f"QUESTION: {job['input']}",
    )
    if not isinstance(subs, list) or not subs:
        return {"ok": False, "notes": "decomposition did not yield a list"}
    subs = [str(s).strip() for s in subs if str(s).strip()][:4]
    ctx["log"](f"fanout -> {len(subs)} sub-questions")

    children = [{"skill": "research_local", "input": s} for s in subs]
    children.append({"skill": "research_fanout", "input": f"SYNTH:{job['id']}"})

    # a marker file so the join can recover the original question
    with open(os.path.join(ctx["workdir"], "question.txt"), "w", encoding="utf-8") as fh:
        fh.write(job["input"])
    return {"ok": True, "notes": f"spawned {len(subs)} research + 1 synth",
            "output": None, "children": children}


def _synthesize(job: dict, ctx: dict) -> dict:
    llm = ctx["llm"]
    parent_id = job["input"].split(":", 1)[1]
    queue = Queue(ctx["root"])
    siblings = [j for j in queue.all()
                if j.get("parent") == parent_id and j["skill"] == "research_local"]
    pending = [j for j in siblings if j["status"] in ("queued", "running")]
    if pending:
        # join barrier: with parallel workers, siblings may still be grinding
        return {"defer": True,
                "notes": f"waiting on {len(pending)}/{len(siblings)} siblings of {parent_id}"}
    done = [j for j in siblings if j["status"] == "done" and j.get("output")]
    if not done:
        return {"ok": False,
                "notes": f"all {len(siblings)} children of {parent_id} finished without output"}

    question = ""
    qpath = os.path.join(ctx["root"], "jobs", parent_id, "question.txt")
    if os.path.exists(qpath):
        with open(qpath, "r", encoding="utf-8") as fh:
            question = fh.read().strip()

    parts, sources = [], []
    for j in done:
        path = os.path.join(ctx["root"], j["output"])
        with open(path, "r", encoding="utf-8", errors="ignore") as fh:
            text = fh.read()
        parts.append(text[:5000])
        in_sources = False
        for line in text.splitlines():
            if line.startswith("## Sources"):
                in_sources = True
            elif in_sources and line.strip().startswith("-"):
                sources.append(line.strip())

    missing = len(siblings) - len(done)
    merged = llm.generate(
        "Merge these sub-reports into ONE coherent report answering the main "
        "question. Under 500 words. Keep only claims present in the sub-reports."
        + (f" Note that {missing} sub-question(s) failed and are not covered."
           if missing else "") +
        f"\n\nMAIN QUESTION: {question or '(see sub-reports)'}\n\nSUB-REPORTS:\n\n"
        + "\n\n---\n\n".join(parts),
        system="You are Kraken, the Teledra kingdom's silent research assistant.",
    )
    unique_sources = sorted(set(sources))
    report = (f"# Synthesis: {question or parent_id}\n\n{merged.strip()}\n\n"
              f"## Sources\n\n" + "\n".join(unique_sources) + "\n")
    vault = os.path.join(ctx["root"], "vault")
    os.makedirs(vault, exist_ok=True)
    out = os.path.join(vault, f"{job['id']}-synthesis.md")
    with open(out, "w", encoding="utf-8") as fh:
        fh.write(report)
    return {"ok": True, "output": os.path.relpath(out, ctx["root"]),
            "notes": f"merged {len(done)}/{len(siblings)} children of {parent_id}"}
