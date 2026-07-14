"""Shared hostile-query guards for research skills."""

from __future__ import annotations

import re
from collections import Counter

RESEARCH_STOPWORDS = {
    "the", "a", "an", "and", "or", "of", "to", "in", "on", "for",
    "is", "are", "what", "how", "why", "does", "do", "with", "about",
    "cache", "data", "info", "file", "files", "web", "page", "source",
    "sources", "probe", "test", "round", "official", "current", "latest",
}


def research_terms(question: str) -> list[str]:
    words = re.findall(r"[a-zA-Z][a-zA-Z0-9_-]{2,}", question.lower())
    return [w for w in words if w not in RESEARCH_STOPWORDS and len(w) >= 4]


def terms_match_text(terms: list[str], text: str) -> int:
    """Count question terms that appear as whole words in text."""
    if not terms:
        return 0
    hits = 0
    for term in terms:
        if re.search(rf"\b{re.escape(term)}\b", text, re.IGNORECASE):
            hits += 1
    return hits


def query_sanity(question: str) -> str | None:
    """Reject padding attacks and obvious gibberish before retrieval."""
    text = question.strip()
    if not text:
        return "query is empty"

    if re.search(r"(.)\1{120,}", text, re.IGNORECASE):
        return "query is mostly repeated padding; rephrase as a plain question"

    words = re.findall(r"[a-zA-Z][a-zA-Z0-9_-]{2,}", text)
    if len(words) >= 8:
        counts = Counter(w.lower() for w in words)
        top_word, top_n = counts.most_common(1)[0]
        if top_n / len(words) > 0.45 and len(top_word) <= 2:
            return "query has too little lexical diversity; drop padding"
        if top_n / len(words) > 0.55:
            return "query is dominated by one repeated token; rephrase"

    tokens = text.split()
    if len(tokens) == 1:
        word = tokens[0]
        if len(word) >= 12 and re.fullmatch(r"[a-zA-Z0-9_-]+", word):
            vowels = sum(1 for c in word.lower() if c in "aeiou")
            if vowels / len(word) < 0.22:
                return "query looks like random characters, not a research question"

    return None