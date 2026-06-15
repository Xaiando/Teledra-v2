"""Deterministic Treasury income scout.

The Treasury used to depend on the LLM emitting a [RESEARCH:]/[DELEGATE:] tag,
which llama3 rarely did, so knowledge/treasury_ledger.md stayed empty. This
script removes that dependency: each run rotates to the next real income-source
query, scrapes it via browser_agent.py, and appends a STRUCTURED set of leads
(title + URL) straight to the ledger. The Treasurer then reports from a ledger
that is actually full. Money/gig-acceptance still stays with the human.

Prints one JSON line: {"ok", "query", "found", "headline"}.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import time

ROOT = os.path.abspath(os.path.dirname(__file__))
PYTHON = os.path.join(ROOT, ".venv", "Scripts", "python.exe")
BROWSER = os.path.join(ROOT, "browser_agent.py")
LEDGER = os.path.join(ROOT, "knowledge", "treasury_ledger.md")
STATE = os.path.join(ROOT, "config", ".treasury_state.json")

# Concrete, searchable income paths aligned with the kingdom's actual assets:
# generative art, music, stream overlays/emotes, workshop tools, agent services.
QUERIES = [
    "freelance generative art commission marketplace for artists",
    "sell custom Discord emotes and stickers marketplace",
    "sell Twitch stream overlays and alerts marketplace creators",
    "royalty free music licensing marketplace submit tracks",
    "Gumroad best selling digital art and asset packs",
    "itch.io sell game art asset packs",
    "open source bug bounty programs cash rewards",
    "AI agent task bounty platform rewards",
    "Twitch small streamer sponsorship program apply",
    "animated emoji pack commission marketplace",
]


def _load_index() -> int:
    try:
        with open(STATE, "r", encoding="utf-8") as handle:
            return int(json.load(handle).get("index", 0))
    except Exception:
        return 0


def _save_index(idx: int) -> None:
    os.makedirs(os.path.dirname(STATE), exist_ok=True)
    try:
        with open(STATE, "w", encoding="utf-8") as handle:
            json.dump({"index": idx}, handle)
    except Exception:
        pass


def scout() -> dict:
    idx = _load_index()
    query = QUERIES[idx % len(QUERIES)]
    _save_index(idx + 1)

    try:
        proc = subprocess.run(
            [PYTHON, BROWSER, query],
            cwd=ROOT,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=75,
        )
        out = proc.stdout or ""
    except Exception as exc:
        return {"ok": False, "query": query, "found": 0, "headline": f"scout error: {exc}"}

    findings = []
    title = "result"
    for line in out.splitlines():
        m = re.match(r"---\s*CONTENT FROM\s*(.+?)\s*$", line)
        if m:
            title = m.group(1).strip().rstrip(" -").strip() or "result"
            continue
        m = re.match(r"\s*URL:\s*(https?://\S+)", line)
        if m:
            findings.append((title, m.group(1).strip()))

    # De-dupe by URL, cap to a tidy handful.
    seen, deduped = set(), []
    for t, u in findings:
        if u in seen:
            continue
        seen.add(u)
        deduped.append((t, u))
    deduped = deduped[:5]

    os.makedirs(os.path.dirname(LEDGER), exist_ok=True)
    ts = int(time.time())
    with open(LEDGER, "a", encoding="utf-8") as handle:
        handle.write(f'\n- [{ts}] SCOUT "{query}":\n')
        if deduped:
            for t, u in deduped:
                handle.write(f"    - {t[:90]} -- {u}\n")
        else:
            handle.write("    - (no concrete links found this pass)\n")
        handle.write("    (raw scout; human evaluates pay, fit, and risk before acting)\n")

    headline = (
        f"{len(deduped)} leads for '{query}'" if deduped else f"no leads for '{query}'"
    )
    return {"ok": True, "query": query, "found": len(deduped), "headline": headline}


if __name__ == "__main__":
    print(json.dumps(scout(), ensure_ascii=True))
