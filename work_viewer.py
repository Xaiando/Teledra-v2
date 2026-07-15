"""Teledra Work Board -- the /work command opens this in its own console window.

Reads the structured For You queue that treasury_scout.py builds
(knowledge/work_queue.json) plus the tail of knowledge/treasury_ledger.md, and
lays the job suggestions out by LANE so the operator can read the details and
decide. Read-only: it never accepts a gig or moves money -- it just shows you
what the court has found and what the standing policy says to do with it.

Lanes:
  YOU    -- overlay / emote / voice-over you do personally (share Xaiando85@gmail.com)
  AGENT  -- deliverables the court can fulfil if payment (PayPal/Ko-fi) is confirmable
  BOUNTY -- bug bounties; either of you; the only sanctioned reason to touch SaaS
  IDEA   -- anti-SaaS product ideas: candidate one-off paid apps / website tools
"""

from __future__ import annotations

import json
import os
import re
import sys
from datetime import datetime

_EMAIL_RE = re.compile(r"\b[\w.+-]+@[\w.-]+\.\w+\b")


def _strip_emails(text: str) -> str:
    """Old queue entries baked the operator email into `action`; drop it so the
    board reads as a job description, not a contact dump."""
    cleaned = _EMAIL_RE.sub("", text or "")
    # Drop connector words left dangling right before punctuation, e.g.
    # "share ;" / "correspond as ." -> remove the now-empty phrase.
    cleaned = re.sub(r"\b(?:share|correspond as|as)\s*(?=[;.,]|$)", "", cleaned, flags=re.I)
    # An em/double dash now sitting just before punctuation ("only --;") is noise.
    cleaned = re.sub(r"\s*--\s*(?=[;.,])", "", cleaned)
    cleaned = re.sub(r"\s{2,}", " ", cleaned)
    cleaned = re.sub(r"\s+([;.,])", r"\1", cleaned)
    # Collapse duplicated sentence punctuation left after removal ("operator..").
    cleaned = re.sub(r"([.;,])(\s*[.;,])+", r"\1", cleaned)
    return cleaned.strip(" ;,-")


def _source_from_url(url: str) -> str:
    """Human 'where to find it' label from a URL host."""
    try:
        host = url.split("//", 1)[-1].split("/", 1)[0]
        return host[4:] if host.startswith("www.") else host
    except Exception:
        return ""

# Lead titles/URLs (and the raw ledger) can carry Unicode; a cp1252 console
# would otherwise crash on the first stray glyph. Make output tolerant.
try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

ROOT = os.path.abspath(os.path.dirname(__file__))
QUEUE = os.path.join(ROOT, "knowledge", "work_queue.json")
LEDGER = os.path.join(ROOT, "knowledge", "treasury_ledger.md")

LANE_ORDER = ["you", "agent", "bounty", "idea"]
LANE_TITLE = {
    "you": "YOU  --  operator does this personally (RODE mic / voices.com / overlays)",
    "agent": "AGENT  --  court can deliver; auto-proceed if PayPal/Ko-fi payment confirmable",
    "bounty": "BOUNTY  --  bug bounties; either of you; only sanctioned SaaS contact",
    "idea": "IDEA  --  anti-SaaS product scouting; one-off paid apps / website tools",
}
BAR = "=" * 78
RULE = "-" * 78


def _load_queue() -> list:
    try:
        with open(QUEUE, "r", encoding="utf-8") as handle:
            data = json.load(handle)
            return data if isinstance(data, list) else []
    except FileNotFoundError:
        return []
    except Exception as exc:
        print(f"(could not read work queue: {exc})")
        return []


def _fmt_ts(ts) -> str:
    try:
        return datetime.fromtimestamp(int(ts)).strftime("%Y-%m-%d %H:%M")
    except Exception:
        return "?"


def _ledger_tail(n: int = 16) -> list:
    try:
        with open(LEDGER, "r", encoding="utf-8") as handle:
            lines = handle.read().splitlines()
        return lines[-n:]
    except Exception:
        return []


def render() -> None:
    queue = _load_queue()
    print(BAR)
    print("  TELEDRA WORK BOARD  --  income leads & job suggestions")
    print(f"  {len(queue)} leads in the For You queue  |  generated {datetime.now():%Y-%m-%d %H:%M}")
    print(BAR)

    if not queue:
        print("\n  The queue is empty so far.")
        print("  Run /treasury in the TUI a few times to let the scout fill it,")
        print("  then reopen /work.\n")
    else:
        by_lane = {lane: [] for lane in LANE_ORDER}
        for entry in queue:
            by_lane.setdefault(entry.get("lane", "agent"), []).append(entry)

        for lane in LANE_ORDER:
            items = by_lane.get(lane, [])
            if not items:
                continue
            print(f"\n{RULE}")
            print(f"  {LANE_TITLE.get(lane, lane.upper())}   ({len(items)})")
            print(RULE)
            # newest first
            for entry in sorted(items, key=lambda e: e.get("ts", 0), reverse=True):
                title = (entry.get("title") or "untitled").strip()
                url = (entry.get("url") or "").strip()
                action = _strip_emails((entry.get("action") or "").strip())
                print(f"\n  - {title}")
                if action:
                    print(f"      What:  {action}")
                if url:
                    src = _source_from_url(url)
                    print(f"      Where: {url}" + (f"   ({src})" if src else ""))
                # Contact only matters for the YOU lane (operator does this outreach
                # personally); elsewhere it's just noise, so we keep it off the board.
                email = entry.get("contact_email")
                if email and lane == "you":
                    print(f"      Contact: {email}")
                print(f"      (found {_fmt_ts(entry.get('ts'))} via \"{entry.get('query', '')}\")")

    tail = _ledger_tail()
    if tail:
        print(f"\n{BAR}")
        print("  RECENT RAW SCOUT LOG (knowledge/treasury_ledger.md tail)")
        print(BAR)
        for line in tail:
            print(f"  {line}")

    print(f"\n{BAR}")
    print("  Policy: YOU = you handle it (agents only surface, never auto-commit).")
    print("  AGENT = auto-proceed only if PayPal/Ko-fi payment is confirmable; any deposit")
    print("  comes to you first. BOUNTY = the only sanctioned SaaS contact. We never become")
    print("  SaaS customers; we only get paid to find their bugs.")
    print(BAR)


def main() -> int:
    try:
        render()
    except Exception as exc:
        print(f"Work board error: {type(exc).__name__}: {exc}")
    try:
        input("\nPress Enter to close this window... ")
    except (EOFError, KeyboardInterrupt):
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
