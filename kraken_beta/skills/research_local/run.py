"""research_local — cited answers from D:\\Teledra\\knowledge, fully offline."""

from __future__ import annotations

import os
import re

from kraken.kernel import query_guard

KNOWLEDGE = r"D:\Teledra\knowledge"
READ_LIMIT = 6000     # chars per source file
TOP_FILES = 4
STOPWORDS = {"the", "a", "an", "and", "or", "of", "to", "in", "on", "for",
             "is", "are", "what", "how", "why", "does", "do", "with", "about"}


def _terms(question: str) -> list[str]:
    words = re.findall(r"[a-zA-Z][a-zA-Z0-9_-]{2,}", question.lower())
    return [w for w in words if w not in STOPWORDS]


def _answer_denies_info(answer: str) -> bool:
    low = answer.lower()
    markers = (
        "do not contain", "does not contain", "no information",
        "no relevant", "cannot extract", "impossible to extract",
        "not present in", "no direct information", "cannot answer",
        "does not provide", "not supported by", "impossible to",
    )
    return any(m in low for m in markers)



def _score_file(path: str, terms: list[str]) -> tuple[int, int]:
    """(distinct terms matched, capped total) — coverage beats repetition."""
    try:
        with open(path, "r", encoding="utf-8", errors="ignore") as fh:
            text = fh.read(200_000).lower()
    except OSError:
        return (0, 0)
    counts = [text.count(t) for t in terms]
    return (sum(1 for c in counts if c), sum(min(c, 5) for c in counts))


CORPUS_DIRS = [KNOWLEDGE, r"D:\Teledra"]  # root holds the design docs


def _candidates() -> list[str]:
    out = []
    for base in CORPUS_DIRS:
        for name in os.listdir(base):
            path = os.path.join(base, name)
            # skip the huge chat archives; they drown the signal
            if os.path.isfile(path) and not name.startswith("chat_logs"):
                if name.endswith((".md", ".txt")) or (
                        base == KNOWLEDGE and name.endswith(".json")):
                    out.append(path)
    return out


KINGDOM_LEXICON = {"teledra", "wizard", "court", "crown", "kraken", "ornith",
                   "swarm", "diplomat", "treasury", "fractus", "moltbook",
                   "kingdom", "orator", "cenedra"}


def _should_escalate(question: str, llm) -> bool:
    """Only send questions to the public web that the public web can answer.
    Kingdom-internal affairs live in local files alone — escalating them just
    burns cycles on honest failures."""
    # deterministic guard first: kingdom vocabulary => internal, no judge call
    words = set(re.findall(r"[a-z]+", question.lower()))
    if words & KINGDOM_LEXICON:
        return False
    verdict = llm.generate(
        "Is this question about the internal affairs of a private project "
        "called Teledra (its court, treasury, policies, kingdom, agents) — "
        "answerable only from its own private files? Or is it a general/world "
        "question the public web could answer? Reply ONLY internal or general.\n\n"
        f"QUESTION: {question}",
        timeout=60,
    ).strip().lower()
    return verdict.startswith("general")


def execute(job: dict, ctx: dict) -> dict:
    question = job["input"]
    llm = ctx["llm"]
    sanity = query_guard.query_sanity(question)
    if sanity:
        ctx["log"](f"query rejected: {sanity}")
        report = (f"# {question[:200]}{'...' if len(question) > 200 else ''}\n\n"
                  f"Cannot research this query: {sanity}\n\n## Sources\n\n(none)\n")
        out_path = _write(ctx, job, report)
        return {"ok": True, "output": out_path, "notes": "hostile/low-info query rejected"}

    terms = _terms(question)
    ctx["log"](f"terms: {terms}")

    scored = sorted(
        ((_score_file(p, terms), p) for p in _candidates()), reverse=True
    )
    min_distinct = max(1, len(terms) // 3)
    sources = [p for score, p in scored[:TOP_FILES] if score[0] >= min_distinct]

    if not sources:
        report = (f"# {question}\n\nThe kingdom's knowledge base has no relevant "
                  f"material on this yet.\n\n## Sources\n\n(none found)\n")
        out_path = _write(ctx, job, report)
        result = {"ok": True, "output": out_path, "notes": "no local sources"}
        # fall through to the web: local-first, kraken-armed second
        if (os.path.isdir(os.path.join(ctx["root"], "skills", "research_web"))
                and _should_escalate(question, llm)):
            result["children"] = [{"skill": "research_web", "input": question}]
            result["notes"] += "; escalated to research_web"
        else:
            result["notes"] += "; internal question, web escalation skipped"
        return result

    excerpts = []
    for path in sources:
        with open(path, "r", encoding="utf-8", errors="ignore") as fh:
            excerpts.append(f"=== {os.path.basename(path)} ===\n{fh.read(READ_LIMIT)}")
    ctx["log"](f"reading {len(sources)} sources")

    # relevance gate: term overlap is easily fooled; let qwen veto tangents.
    # Sample around term hits, not file heads — the answer may sit mid-file.
    # one source per call — a 7B judge is reliable on single docs, not stacks.
    # But the judge is FLAKY on borderline phrasings; when the mechanical
    # match is strong (most distinct terms present), trust it without asking.
    strong_bar = max(2, (2 * len(terms)) // 3)
    strength = {p: s for s, p in scored}
    relevant = []
    for path, excerpt in zip(list(sources), excerpts):
        if strength.get(path, (0, 0))[0] >= strong_bar:
            relevant.append((path, excerpt))
            continue
        verdict = llm.generate(
            "Does this document contain material that DIRECTLY helps answer "
            "the question (not just shared vocabulary)? If partially, answer "
            "yes. Reply ONLY yes or no.\n\n"
            f"QUESTION: {question}\n\nDOCUMENT:\n\n{excerpt[:5000]}",
            timeout=90,
        ).strip().lower()
        if verdict.startswith("yes"):
            relevant.append((path, excerpt))
    ctx["log"](f"relevance gate: {len(relevant)}/{len(sources)} sources relevant")
    if relevant:
        sources = [p for p, _ in relevant]
        excerpts = [e for _, e in relevant]
    if not relevant:
        ctx["log"]("relevance gate: local sources are tangential")
        report = (f"# {question}\n\nLocal knowledge only brushes this topic "
                  f"(shared vocabulary, no direct answer).\n\n## Sources\n\n(none relevant)\n")
        out_path = _write(ctx, job, report)
        result = {"ok": True, "output": out_path, "notes": "local sources tangential"}
        if (os.path.isdir(os.path.join(ctx["root"], "skills", "research_web"))
                and _should_escalate(question, llm)):
            result["children"] = [{"skill": "research_web", "input": question}]
            result["notes"] += "; escalated to research_web"
        else:
            result["notes"] += "; internal question, web escalation skipped"
        return result

    answer = llm.generate(
        "Answer the question using ONLY the source excerpts. Be concrete and "
        "brief (under 300 words). Name the SPECIFIC mechanisms, terms, and "
        "values the sources use (e.g. exact feature names, listed steps) "
        "rather than paraphrasing vaguely. If the sources only partially "
        "answer it, say what is missing. Do not invent facts.\n\n"
        f"QUESTION: {question}\n\nSOURCES:\n\n" + "\n\n".join(excerpts),
        system="You are Kraken, the Teledra kingdom's silent research assistant. "
               "Precise, honest, no flattery.",
    )

    answer = answer.strip()
    if _answer_denies_info(answer):
        ctx["log"]("answer denies corpus coverage — stripping tangential sources")
        report = (f"# {question}\n\n{answer}\n\n"
                  f"## Sources\n\n(none relevant — tangential matches only)\n")
    else:
        lines = "\n".join(f"- `{p}`" for p in sources)
        report = f"# {question}\n\n{answer}\n\n## Sources\n\n{lines}\n"
    out_path = _write(ctx, job, report)
    return {"ok": True, "output": out_path, "notes": f"{len(sources)} sources cited"}


def _write(ctx: dict, job: dict, report: str) -> str:
    vault = os.path.join(ctx["root"], "vault")
    os.makedirs(vault, exist_ok=True)
    path = os.path.join(vault, f"{job['id']}-report.md")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(report)
    return os.path.relpath(path, ctx["root"])
