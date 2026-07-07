"""Deterministic Treasury income scout (lane-aware, anti-SaaS).

Each run rotates to the next real income-source query, scrapes it via
browser_agent.py, and does two things:

  1. Appends the raw leads to knowledge/treasury_ledger.md (audit trail).
  2. Classifies each lead into a LANE and appends it to the structured
     For You queue, knowledge/work_queue.json, which the /work board reads.

Lanes encode the operator's standing policy:

  you     -- overlay / emote / voice-over work the operator does personally
             (decent RODE mic + voices.com). Surface it; share Xaiando85@gmail.com.
             The operator decides; agents do NOT auto-commit this lane.
  agent   -- deliverables the court can actually produce (emote/overlay/art/
             music packs). Auto-proceed IF payment to PayPal/Ko-fi is
             confirmable; a required deposit escalates to the operator.
             Agents may sign up / correspond autonomously with Rollnrocka@hotmail.com.
  bounty  -- bug bounties. Either side can pursue; great funding for better gear.
             Agents may register autonomously with Rollnrocka@hotmail.com. This is
             the ONLY way we touch SaaS -- we engage a SaaS only to get paid to
             find its bugs, never as customers.
  idea    -- anti-SaaS product scouting: places people resent a subscription for
             something simple. Candidate one-off paid apps / website tools that set
             people free from SaaS, and bait to pull eyes onto the bigger projects.

Prints one JSON line: {"ok", "query", "lane", "found", "queued", "headline"}.
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
QUEUE = os.path.join(ROOT, "knowledge", "work_queue.json")
STATE = os.path.join(ROOT, "config", ".treasury_state.json")

# Emails (operator's standing decision):
#   OPERATOR_EMAIL -- the operator handles these personally; surface, don't auto-commit.
#   AUTONOMOUS_EMAIL -- not tied to any login or payment; agents may share it freely
#                       (operator is well able to spot scams).
OPERATOR_EMAIL = "Xaiando85@gmail.com"
AUTONOMOUS_EMAIL = "Rollnrocka@hotmail.com"

# (query, lane) -- rotated one per run. Ordered so a few passes cover every lane.
QUERIES = [
    # you -- operator-personal: overlays, emotes, voice-over
    ("freelance Twitch stream overlay design gig paid", "you"),
    ("custom animated Discord emote commission gig paid", "you"),
    ("short voice over narration gig marketplace paid", "you"),
    ("voices.com voice over jobs apply remote", "you"),
    # agent -- court-producible deliverables with a real payout
    ("sell custom Discord emote pack marketplace PayPal payout", "agent"),
    ("royalty free music licensing marketplace submit tracks payout", "agent"),
    ("sell generative art asset pack Gumroad Ko-fi payout", "agent"),
    ("itch.io sell game art and music asset packs", "agent"),
    # bounty -- the only sanctioned reason to touch SaaS
    ("open source bug bounty programs cash rewards PayPal", "bounty"),
    ("public web app bug bounty program payout list", "bounty"),
    # idea -- anti-SaaS one-off-app scouting
    ("people frustrated paying monthly subscription for a simple tool", "idea"),
    ("replace expensive SaaS subscription with one time purchase app", "idea"),
    ("self hosted offline alternative to subscription software", "idea"),
]

# Keyword nudges: a lead's text can override its query's default lane.
YOU_KW = ("voice over", "voiceover", "voice-over", "narration", "overlay", "emote", "emoji", "alerts")
BOUNTY_KW = ("bug bounty", "vulnerability reward", "vdp", "hackerone", "bugcrowd", "security reward")
IDEA_KW = ("subscription", "saas", "self hosted", "self-hosted", "one time purchase", "one-time")

LANE_META = {
    "you": {
        "label": "YOU (operator does this)",
        "email": OPERATOR_EMAIL,
        "action": "Operator handles this personally (RODE mic / voices.com / overlay design). "
                  "Surface only -- do not auto-commit.",
    },
    "agent": {
        "label": "AGENT (court can deliver)",
        "email": AUTONOMOUS_EMAIL,
        "action": "Court can deliver. Auto-proceed only if payment to PayPal/Ko-fi is confirmable "
                  "(cost-free to us); a required deposit comes to the operator for approval.",
    },
    "bounty": {
        "label": "BOUNTY (either can pursue)",
        "email": AUTONOMOUS_EMAIL,
        "action": "Bug bounty -- funds better gear. Register/report autonomously. "
                  "The only sanctioned reason to engage a SaaS.",
    },
    "idea": {
        "label": "IDEA (anti-SaaS product)",
        "email": None,
        "action": "Candidate one-off paid app / website tool to free people from a subscription. "
                  "Log as product idea; bait for the bigger projects.",
    },
}


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


def _load_queue() -> list:
    try:
        with open(QUEUE, "r", encoding="utf-8") as handle:
            data = json.load(handle)
            return data if isinstance(data, list) else []
    except Exception:
        return []


QUEUE_CAP = 600


def _save_queue(entries: list) -> None:
    os.makedirs(os.path.dirname(QUEUE), exist_ok=True)
    # Keep a generous history so the board can show the full back-catalogue of leads.
    trimmed = entries[-QUEUE_CAP:]
    try:
        with open(QUEUE, "w", encoding="utf-8") as handle:
            json.dump(trimmed, handle, indent=2)
    except Exception:
        pass


# Domains/titles the scraper keeps returning that are not real income leads
# (generic homepages, unrelated local results, app-store listings, resale junk).
NOISE_DOMAINS = (
    "ai.google", "google.com/search", "theopen.com", "osloopen.no",
    "facebook.com/marketplace", "vinted.com", "sikt.no", "apps.apple.com",
    "play.google.com", "porndig", "pornhub", "xvideos", "xnxx", "deepai.org",
)
NOISE_TITLE_KW = (
    "how we're making ai", "live streaming app", "- app store", "sikt ki",
    # adult content the scraper sometimes returns for "sell ... online" queries
    "porn", " xxx", "xxx ", "sex video", "nsfw", "hd porn",
    # generic non-opportunities / unrelated named hits
    "nordic conference", "actu people", "saksbehandling",
)


def _is_noise(title: str, url: str) -> bool:
    u, t = url.lower(), title.lower()
    if any(dom in u for dom in NOISE_DOMAINS):
        return True
    if any(kw in t for kw in NOISE_TITLE_KW):
        return True
    return False


def _lane_for_query(query: str) -> str:
    """Infer a default lane for an arbitrary (possibly historical) scout query."""
    q = query.lower()
    if any(k in q for k in ("voice over", "voiceover", "voice-over", "narration",
                            "overlay", "emote", "emoji", "sticker", "alerts")):
        return "you"
    if any(k in q for k in ("bug bounty", "vulnerability", "security reward",
                            "hackerone", "bugcrowd", "vdp")):
        return "bounty"
    if any(k in q for k in ("subscription", "saas", "self hosted", "self-hosted",
                            "one time purchase", "one-time", "independent")):
        return "idea"
    return "agent"


def _classify(title: str, url: str, default_lane: str) -> str:
    blob = f"{title} {url}".lower()
    if any(k in blob for k in BOUNTY_KW):
        return "bounty"
    if any(k in blob for k in YOU_KW):
        # voice-over / overlay / emote stays operator-personal even if found under another query
        return "you" if default_lane != "agent" else default_lane
    if default_lane == "idea" or any(k in blob for k in IDEA_KW):
        return "idea" if default_lane == "idea" else default_lane
    return default_lane


def scout() -> dict:
    idx = _load_index()
    query, lane = QUERIES[idx % len(QUERIES)]
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
        return {"ok": False, "query": query, "lane": lane, "found": 0,
                "queued": 0, "headline": f"scout error: {exc}"}

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

    seen, deduped = set(), []
    for t, u in findings:
        if u in seen:
            continue
        seen.add(u)
        deduped.append((t, u))
    deduped = deduped[:5]

    # 1) Raw audit trail (unchanged behaviour).
    os.makedirs(os.path.dirname(LEDGER), exist_ok=True)
    ts = int(time.time())
    with open(LEDGER, "a", encoding="utf-8") as handle:
        handle.write(f'\n- [{ts}] SCOUT [{lane}] "{query}":\n')
        if deduped:
            for t, u in deduped:
                handle.write(f"    - {t[:90]} -- {u}\n")
        else:
            handle.write("    - (no concrete links found this pass)\n")
        handle.write("    (raw scout; human evaluates pay, fit, and risk before acting)\n")

    # 2) Structured For You queue (what /work reads).
    queue = _load_queue()
    known = {e.get("url") for e in queue}
    queued = 0
    for t, u in deduped:
        if u in known or _is_noise(t, u):
            continue
        entry_lane = _classify(t, u, lane)
        meta = LANE_META[entry_lane]
        queue.append({
            "ts": ts,
            "lane": entry_lane,
            "lane_label": meta["label"],
            "title": t[:120],
            "url": u,
            "query": query,
            "contact_email": meta["email"],
            "action": meta["action"],
            "status": "new",
        })
        known.add(u)
        queued += 1
    _save_queue(queue)

    headline = (
        f"{len(deduped)} leads / {queued} new for [{lane}] '{query}'"
        if deduped else f"no leads for [{lane}] '{query}'"
    )
    return {"ok": True, "query": query, "lane": lane,
            "found": len(deduped), "queued": queued, "headline": headline}


def backfill_from_ledger() -> dict:
    """Rebuild knowledge/work_queue.json from the full treasury_ledger.md history.

    The structured queue only started filling when this scout was upgraded, so the
    hundreds of leads already logged were invisible to /work. This parses every
    historical SCOUT block, classifies each lead into a lane, de-dupes by URL, and
    writes them all to the queue (merging with anything already there).
    """
    header_re = re.compile(
        r'^\s*-\s*\[(\d+)\]\s*SCOUT\s*(?:\[(\w+)\]\s*)?"(.+?)"\s*:?\s*$')
    lead_re = re.compile(r'^\s+-\s+(.*?)\s+--\s+(https?://\S+)\s*$')

    try:
        with open(LEDGER, "r", encoding="utf-8") as handle:
            lines = handle.read().splitlines()
    except Exception as exc:
        return {"ok": False, "error": f"cannot read ledger: {exc}"}

    queue = _load_queue()
    known = {e.get("url") for e in queue}
    added = 0

    ts, query, default_lane = 0, "", "agent"
    for line in lines:
        hm = header_re.match(line)
        if hm:
            ts = int(hm.group(1))
            tagged = hm.group(2)
            query = hm.group(3).strip()
            default_lane = tagged if tagged in LANE_META else _lane_for_query(query)
            continue
        lm = lead_re.match(line)
        if not lm:
            continue
        title, url = lm.group(1).strip(), lm.group(2).strip()
        if url in known or _is_noise(title, url):
            continue
        lane = _classify(title, url, default_lane)
        meta = LANE_META[lane]
        queue.append({
            "ts": ts,
            "lane": lane,
            "lane_label": meta["label"],
            "title": title[:120],
            "url": url,
            "query": query,
            "contact_email": meta["email"],
            "action": meta["action"],
            "status": "new",
        })
        known.add(url)
        added += 1

    _save_queue(queue)
    return {"ok": True, "added": added, "total": len(queue[-QUEUE_CAP:])}


if __name__ == "__main__":
    import sys
    if len(sys.argv) > 1 and sys.argv[1] in ("--backfill", "backfill"):
        print(json.dumps(backfill_from_ledger(), ensure_ascii=True))
    else:
        print(json.dumps(scout(), ensure_ascii=True))
