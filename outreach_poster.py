"""Autonomous outreach poster for Teledra's Diplomat.

This is the only component that actually performs OUTWARD posting. It is gated
behind operator-supplied credentials so nothing leaves the machine until the
user opts in:

  * config/moltbook.json       -> Moltbook (the social network for AI agents)
  * config/outreach_channels.json -> generic webhooks (Discord/Slack/custom)

Subcommands (the Rust runtime invokes these; a JSON job is read from stdin for
``post``):

  python outreach_poster.py register   # register the agent on Moltbook
  python outreach_poster.py status     # check Moltbook claim/activation status
  python outreach_poster.py channels   # report which outward channels are live
  python outreach_poster.py post       # stdin: {"title": "...", "content": "..."}

All subcommands print a single JSON object to stdout describing exactly what
happened, so the court can record HONEST evidence (it only claims a post
succeeded when a real 2xx response came back). Moltbook's published limit is one
post per 30 minutes; a local cooldown in config/.outreach_state.json enforces a
safe margin so the kingdom never gets rate-limited or banned.

Moltbook API reference: https://www.moltbook.com/skill.md
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.request

ROOT = os.path.abspath(os.path.dirname(__file__))
CONFIG_DIR = os.path.join(ROOT, "config")
MOLTBOOK_CFG = os.path.join(CONFIG_DIR, "moltbook.json")
CHANNELS_CFG = os.path.join(CONFIG_DIR, "outreach_channels.json")
STATE_PATH = os.path.join(CONFIG_DIR, ".outreach_state.json")

# Must use the www host or Moltbook strips the Authorization header.
MOLTBOOK_BASE = "https://www.moltbook.com/api/v1"
# 30 min is Moltbook's published floor; pad to ~31 min to stay safely under it.
DEFAULT_MIN_POST_INTERVAL = 1860


def _load_json(path: str, default):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except Exception:
        return default


def _save_json(path: str, data) -> None:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as handle:
        json.dump(data, handle, indent=2, ensure_ascii=True)
    os.replace(tmp, path)


def _http_json(method: str, url: str, headers: dict, body=None, timeout: int = 20):
    """Returns (status_code, parsed_or_text). status 0 means transport failure."""
    data = None
    if body is not None:
        data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(url, data=data, method=method)
    for key, value in headers.items():
        req.add_header(key, value)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read().decode("utf-8", errors="replace")
            status = getattr(resp, "status", resp.getcode())
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode("utf-8", errors="replace") if exc.fp else str(exc)
        status = exc.code
    except Exception as exc:  # timeout, DNS, connection refused, etc.
        return 0, {"error": f"{type(exc).__name__}: {exc}"}
    try:
        return status, json.loads(raw)
    except Exception:
        return status, raw


def _state() -> dict:
    return _load_json(STATE_PATH, {})


def _record_post_time(channel: str) -> None:
    state = _state()
    last = state.get("last_post", {})
    last[channel] = int(time.time())
    state["last_post"] = last
    _save_json(STATE_PATH, state)


def _seconds_since_last(channel: str) -> float:
    last = _state().get("last_post", {}).get(channel)
    if not last:
        return float("inf")
    return time.time() - float(last)


def _extract_post_reference(payload) -> str:
    if not isinstance(payload, dict):
        return ""
    for key in ("permalink", "url", "post_url", "link"):
        value = payload.get(key)
        if isinstance(value, str) and value:
            return value
    for container_key in ("post", "data"):
        inner = payload.get(container_key)
        if isinstance(inner, dict):
            for key in ("permalink", "url", "id", "post_id"):
                value = inner.get(key)
                if value:
                    return f"{key}={value}"
    for key in ("id", "post_id"):
        value = payload.get(key)
        if value:
            return f"{key}={value}"
    return ""


def _moltbook_cfg() -> dict:
    return _load_json(MOLTBOOK_CFG, {})


def _deep_find(payload, candidate_keys) -> str:
    """Recursively search a JSON payload for the first matching key (case- and
    underscore-insensitive). Moltbook nests/renames fields, so we can't assume a
    flat snake_case shape."""
    wanted = {k.lower().replace("_", "") for k in candidate_keys}
    stack = [payload]
    while stack:
        node = stack.pop()
        if isinstance(node, dict):
            for k, v in node.items():
                if isinstance(k, str) and k.lower().replace("_", "") in wanted:
                    if isinstance(v, (str, int)) and str(v):
                        return str(v)
                if isinstance(v, (dict, list)):
                    stack.append(v)
        elif isinstance(node, list):
            stack.extend(node)
    return ""


def moltbook_register() -> dict:
    cfg = _moltbook_cfg()
    if cfg.get("api_key"):
        return {
            "ok": True,
            "channel": "moltbook",
            "detail": "already registered; api_key present",
            "claim_url": cfg.get("claim_url", ""),
        }
    name = cfg.get("agent_name") or "Teledra"
    description = cfg.get("description") or (
        "Teledra's Sovereign Court: an autonomous AI kingdom of fractal art, "
        "live-coded music, and workshop tools."
    )
    status, payload = _http_json(
        "POST",
        f"{MOLTBOOK_BASE}/agents/register",
        {"Content-Type": "application/json"},
        {"name": name, "description": description},
    )
    ok = 200 <= status < 300 and isinstance(payload, dict)
    api_key = _deep_find(payload, ["api_key", "apiKey", "key", "token", "secret"]) if ok else ""
    claim_url = _deep_find(payload, ["claim_url", "claimUrl", "claim_link", "claimLink"]) if ok else ""
    verification = _deep_find(payload, ["verification_code", "verificationCode", "code"]) if ok else ""
    if ok and api_key:
        cfg["api_key"] = api_key
        cfg["claim_url"] = claim_url
        cfg["verification_code"] = verification
        cfg["pending_claim"] = True
        _save_json(MOLTBOOK_CFG, cfg)
    raw = payload if isinstance(payload, str) else json.dumps(payload)
    return {
        "ok": ok and bool(api_key),
        "channel": "moltbook",
        "status": status,
        "api_key_saved": bool(api_key),
        "claim_url": claim_url,
        "verification_code": verification,
        "raw": raw[:1500],
        "detail": "registered; human must complete claim/verification"
        if (ok and api_key)
        else f"registration response did not yield an api_key (status {status})",
    }


def moltbook_status() -> dict:
    cfg = _moltbook_cfg()
    api_key = cfg.get("api_key", "")
    if not api_key:
        return {"ok": False, "channel": "moltbook", "detail": "no api_key configured"}
    status, payload = _http_json(
        "GET",
        f"{MOLTBOOK_BASE}/agents/status",
        {"Authorization": f"Bearer {api_key}"},
    )
    return {
        "ok": 200 <= status < 300,
        "channel": "moltbook",
        "status": status,
        "detail": payload,
    }


def moltbook_post(title: str, content: str) -> dict:
    cfg = _moltbook_cfg()
    if not cfg.get("enabled"):
        return {"channel": "moltbook", "enabled": False, "skipped": "not enabled"}
    api_key = cfg.get("api_key", "")
    if not api_key:
        return {
            "channel": "moltbook",
            "enabled": True,
            "ok": False,
            "skipped": "no api_key; run register and complete the human claim first",
        }

    interval = float(cfg.get("min_post_interval_seconds") or DEFAULT_MIN_POST_INTERVAL)
    waited = _seconds_since_last("moltbook")
    if waited < interval:
        return {
            "channel": "moltbook",
            "enabled": True,
            "ok": False,
            "skipped": f"cooldown: {int(interval - waited)}s until next allowed post",
        }

    submolt = cfg.get("default_submolt") or "agents"
    body = {
        "submolt_name": submolt,
        "title": title[:300],
        "content": content[:40000],
        "type": "text",
    }
    status, payload = _http_json(
        "POST",
        f"{MOLTBOOK_BASE}/posts",
        {"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"},
        body,
    )
    ok = 200 <= status < 300
    if ok:
        _record_post_time("moltbook")
    detail = _extract_post_reference(payload) or (
        payload if isinstance(payload, str) else json.dumps(payload)[:400]
    )
    return {
        "channel": "moltbook",
        "enabled": True,
        "ok": ok,
        "status": status,
        "submolt": submolt,
        "detail": detail,
    }


def webhook_post(channel: dict, text: str) -> dict:
    name = channel.get("name", "webhook")
    if not channel.get("enabled"):
        return {"channel": name, "enabled": False, "skipped": "not enabled"}
    url = channel.get("url", "")
    if not url:
        return {"channel": name, "enabled": True, "ok": False, "skipped": "no url"}
    # Per-channel cooldown so an enabled webhook (e.g. Discord) is not spammed
    # on every diplomacy cycle. Default 15 minutes; override with min_interval_seconds.
    interval = float(channel.get("min_interval_seconds") or 900)
    waited = _seconds_since_last(name)
    if waited < interval:
        return {
            "channel": name,
            "enabled": True,
            "ok": False,
            "skipped": f"cooldown: {int(interval - waited)}s until next allowed post",
        }
    field = channel.get("content_field", "content")
    body = dict(channel.get("extra_fields", {}))
    body[field] = text[: int(channel.get("max_chars", 1900))]
    headers = {"Content-Type": "application/json"}
    headers.update(channel.get("headers", {}))
    status, payload = _http_json("POST", url, headers, body)
    ok = 200 <= status < 300
    if ok:
        _record_post_time(name)
    return {
        "channel": name,
        "enabled": True,
        "ok": ok,
        "status": status,
        "detail": payload if isinstance(payload, str) else json.dumps(payload)[:300],
    }


def cmd_channels() -> dict:
    cfg = _moltbook_cfg()
    channels = _load_json(CHANNELS_CFG, {}).get("channels", [])
    live = []
    if cfg.get("enabled") and cfg.get("api_key"):
        live.append("moltbook")
    for ch in channels:
        if ch.get("enabled") and ch.get("url"):
            live.append(ch.get("name", "webhook"))
    return {"any_enabled": bool(live), "live_channels": live}


def cmd_post() -> dict:
    try:
        job = json.loads(sys.stdin.read() or "{}")
    except Exception as exc:
        return {"posted": False, "error": f"bad job json: {exc}"}

    title = (job.get("title") or "").strip()
    content = (job.get("content") or "").strip()
    if not content:
        return {"posted": False, "error": "empty content"}
    if not title:
        title = content.splitlines()[0][:120] if content else "A note from Teledra's court"

    results = [moltbook_post(title, content)]
    for ch in _load_json(CHANNELS_CFG, {}).get("channels", []):
        results.append(webhook_post(ch, content))

    any_enabled = any(r.get("enabled") for r in results)
    posted = any(r.get("ok") for r in results)
    return {"posted": posted, "any_enabled": any_enabled, "results": results}


def main() -> int:
    cmd = sys.argv[1] if len(sys.argv) > 1 else "channels"
    if cmd == "register":
        out = moltbook_register()
    elif cmd == "status":
        out = moltbook_status()
    elif cmd == "channels":
        out = cmd_channels()
    elif cmd == "post":
        out = cmd_post()
    else:
        out = {"error": f"unknown command: {cmd}"}
    print(json.dumps(out, ensure_ascii=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
