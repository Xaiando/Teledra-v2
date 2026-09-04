"""Operator-gated external outreach for Teledra.

The Rust court only invokes this sidecar after a direct /Diplomat command or an
explicit /Approve decision. Moltbook credentials are sent exclusively to the
documented ``https://www.moltbook.com/api/v1`` host; redirects to any other
host are refused so an Authorization header cannot leak.
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


MOLTBOOK_BASE = "https://www.moltbook.com/api/v1"
MOLTBOOK_HOST = "www.moltbook.com"
MAX_RESPONSE_BYTES = 64 * 1024
DEFAULT_POST_COOLDOWN = 31 * 60
DEFAULT_COMMENT_COOLDOWN = 25
DEFAULT_UPVOTE_COOLDOWN = 3


def emit(ok: bool, message: str, data: dict | None = None) -> int:
    print(
        json.dumps(
            {
                "ok": ok,
                "capability": "outreach",
                "message": message,
                "data": data or {},
            },
            ensure_ascii=True,
        )
    )
    return 0 if ok else 2


def data_root() -> Path:
    configured = os.environ.get("TELEDRA_DATA_DIR", "").strip()
    if configured:
        return Path(configured)
    return Path(__file__).resolve().parents[2] / "data"


def config_path() -> Path:
    configured = os.environ.get("TELEDRA_MOLTBOOK_CONFIG", "").strip()
    if configured:
        return Path(configured)
    current = data_root() / "outreach" / "moltbook.json"
    legacy = legacy_config_dir() / "moltbook.json"
    return current if current.exists() or not legacy.exists() else legacy


def legacy_config_dir() -> Path:
    configured = os.environ.get("TELEDRA_LEGACY_CONFIG_DIR", "").strip()
    if configured:
        return Path(configured)
    # Compatibility with the installed V1 tree next to this V2 workspace. This
    # reads the existing operator-owned credentials without copying or logging
    # them; a V2 config always takes precedence as soon as one is created.
    return Path(__file__).resolve().parents[2].parent / "Teledra" / "config"


def channels_config_path() -> Path:
    configured = os.environ.get("TELEDRA_OUTREACH_CHANNELS_CONFIG", "").strip()
    if configured:
        return Path(configured)
    current = data_root() / "outreach" / "channels.json"
    legacy = legacy_config_dir() / "outreach_channels.json"
    return current if current.exists() or not legacy.exists() else legacy


def state_path() -> Path:
    return data_root() / "outreach" / ".outreach_state.json"


def load_json(path: Path, default):
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
        return value if isinstance(value, type(default)) else default
    except (OSError, ValueError, TypeError):
        return default


def save_json_atomic(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, ensure_ascii=True), encoding="utf-8")
    os.replace(temporary, path)


def moltbook_config() -> dict:
    config = load_json(config_path(), {})
    env_key = os.environ.get("TELEDRA_MOLTBOOK_API_KEY", "").strip()
    if env_key:
        config["api_key"] = env_key
        config.setdefault("enabled", True)
    return config


def cooldown_remaining(kind: str, minimum: float) -> int:
    last = load_json(state_path(), {}).get("last_success", {}).get(kind, 0)
    return max(0, int(minimum - (time.time() - float(last or 0))))


def record_success(kind: str) -> None:
    state = load_json(state_path(), {})
    timestamps = state.setdefault("last_success", {})
    timestamps[kind] = int(time.time())
    save_json_atomic(state_path(), state)


class MoltbookRedirectHandler(urllib.request.HTTPRedirectHandler):
    """Allow only HTTPS redirects that remain on Moltbook's www host."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        parsed = urllib.parse.urlparse(newurl)
        if parsed.scheme != "https" or parsed.hostname != MOLTBOOK_HOST:
            raise urllib.error.HTTPError(
                req.full_url,
                code,
                "refused Moltbook credential redirect",
                headers,
                fp,
            )
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def request_json(method: str, url: str, headers: dict, body=None, *, moltbook=False):
    data = json.dumps(body).encode("utf-8") if body is not None else None
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    opener = (
        urllib.request.build_opener(MoltbookRedirectHandler())
        if moltbook
        else urllib.request.build_opener()
    )
    try:
        with opener.open(request, timeout=20) as response:
            status = getattr(response, "status", response.getcode())
            raw = response.read(MAX_RESPONSE_BYTES).decode("utf-8", errors="replace")
    except urllib.error.HTTPError as error:
        status = error.code
        raw = error.read(MAX_RESPONSE_BYTES).decode("utf-8", errors="replace")
    except (OSError, urllib.error.URLError) as error:
        return 0, {"error": f"{type(error).__name__}: {error}"}
    try:
        return status, json.loads(raw)
    except ValueError:
        return status, {"response": raw[:2000]}


def verification_from(payload):
    if not isinstance(payload, dict):
        return None
    candidates = [payload]
    for key in ("data", "verification", "challenge", "post", "comment"):
        if isinstance(payload.get(key), dict):
            candidates.append(payload[key])
    for candidate in candidates:
        required = candidate.get("verification_required") or candidate.get("requires_verification")
        challenge = candidate.get("verification") or candidate.get("challenge")
        if required or challenge:
            return {
                "required": True,
                "challenge": challenge if isinstance(challenge, (dict, str)) else None,
                "instruction": "Verification needs a separate operator-reviewed action; it was not solved automatically.",
            }
    return None


def reference_from(payload) -> str:
    if not isinstance(payload, dict):
        return ""
    stack = [payload]
    while stack:
        node = stack.pop()
        if not isinstance(node, dict):
            continue
        for key in ("permalink", "url", "post_url", "comment_url", "id", "post_id", "comment_id"):
            value = node.get(key)
            if isinstance(value, (str, int)) and str(value):
                return f"{key}={value}"
        stack.extend(value for value in node.values() if isinstance(value, dict))
    return ""


def parse_fields(payload) -> dict:
    if isinstance(payload, dict):
        return {str(key).lower(): value for key, value in payload.items()}
    text = str(payload or "").strip()
    if text.startswith("{"):
        try:
            parsed = json.loads(text)
            if isinstance(parsed, dict):
                return {str(key).lower(): value for key, value in parsed.items()}
        except ValueError:
            pass
    fields = {}
    for part in text.split(";"):
        if "=" in part:
            key, value = part.split("=", 1)
            fields[key.strip().lower()] = value.strip()
    if not fields and text:
        fields["post_id"] = text
    return fields


def moltbook_request(action: str, job: dict) -> tuple[bool, str, dict]:
    config = moltbook_config()
    if not config.get("enabled"):
        return False, "Moltbook is not enabled", {"config": str(config_path())}
    api_key = str(config.get("api_key") or "").strip()
    if not api_key:
        return False, "Moltbook has no API key", {"config": str(config_path())}

    headers = {"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"}
    if action == "moltbook_post":
        title = str(job.get("title") or "A note from Teledra's court").strip()[:300]
        content = str(job.get("content") or job.get("message") or "").strip()[:40000]
        if not content:
            return False, "Moltbook post content is empty", {}
        cooldown = float(config.get("min_post_interval_seconds") or DEFAULT_POST_COOLDOWN)
        endpoint = f"{MOLTBOOK_BASE}/posts"
        body = {
            "submolt_name": str(job.get("submolt_name") or config.get("default_submolt") or "agents")[:80],
            "title": title,
            "content": content,
        }
        kind = "moltbook_post"
    elif action == "moltbook_comment":
        fields = parse_fields(job.get("payload", job))
        post_id = str(fields.get("post_id") or fields.get("post") or fields.get("id") or "").strip()
        content = str(fields.get("content") or fields.get("text") or fields.get("reply") or fields.get("comment") or "").strip()
        if not post_id or not content:
            return False, "Moltbook comment needs post_id and content", {}
        cooldown = float(config.get("min_comment_interval_seconds") or DEFAULT_COMMENT_COOLDOWN)
        endpoint = f"{MOLTBOOK_BASE}/posts/{urllib.parse.quote(post_id, safe='')}/comments"
        body = {"content": content[:10000]}
        kind = "moltbook_comment"
    elif action == "moltbook_upvote":
        fields = parse_fields(job.get("payload", job))
        post_id = str(fields.get("post_id") or fields.get("post") or fields.get("id") or "").strip()
        if not post_id:
            return False, "Moltbook upvote needs post_id", {}
        cooldown = float(config.get("min_upvote_interval_seconds") or DEFAULT_UPVOTE_COOLDOWN)
        endpoint = f"{MOLTBOOK_BASE}/posts/{urllib.parse.quote(post_id, safe='')}/upvote"
        body = None
        kind = "moltbook_upvote"
    else:
        return False, f"Unsupported Moltbook action: {action}", {}

    remaining = cooldown_remaining(kind, cooldown)
    if remaining:
        return False, f"Moltbook {kind.removeprefix('moltbook_')} cooldown: {remaining}s", {"retry_after_seconds": remaining}
    status, response = request_json("POST", endpoint, headers, body, moltbook=True)
    ok = 200 <= status < 300
    verification = verification_from(response)
    if ok and not verification:
        record_success(kind)
    data = {
        "channel": "moltbook",
        "action": kind,
        "status": status,
        "reference": reference_from(response),
        "verification": verification,
        "response": response,
    }
    if verification:
        return False, "Moltbook requires operator-reviewed verification", data
    return ok, ("Moltbook action confirmed" if ok else f"Moltbook returned HTTP {status}"), data


def moltbook_inbox() -> tuple[bool, str, dict]:
    """Fetch the authenticated home digest without mutating Moltbook."""
    config = moltbook_config()
    api_key = str(config.get("api_key") or "").strip()
    if not config.get("enabled") or not api_key:
        return True, "Moltbook inbox is not configured", {
            "action": "moltbook_inbox",
            "enabled": False,
            "digest": "",
        }
    status, response = request_json(
        "GET",
        f"{MOLTBOOK_BASE}/home",
        {"Authorization": f"Bearer {api_key}"},
        moltbook=True,
    )
    if not 200 <= status < 300:
        return False, f"Moltbook inbox returned HTTP {status}", {
            "action": "moltbook_inbox",
            "status": status,
        }
    home = response if isinstance(response, dict) else {}
    digest_data = {
        "your_account": home.get("your_account", {}),
        "activity_on_your_posts": home.get("activity_on_your_posts", [])[:12]
        if isinstance(home.get("activity_on_your_posts"), list)
        else [],
        "notifications": home.get("notifications", [])[:12]
        if isinstance(home.get("notifications"), list)
        else [],
        "feed": (home.get("feed") or home.get("posts") or [])[:8]
        if isinstance(home.get("feed") or home.get("posts") or [], list)
        else [],
    }
    digest = json.dumps(digest_data, ensure_ascii=True)[:12000]
    return True, "Moltbook inbox refreshed", {
        "action": "moltbook_inbox",
        "enabled": True,
        "status": status,
        "digest": digest,
    }


def generic_post_to(channel: dict, job: dict) -> dict:
    endpoint = str(channel.get("url") or "").strip()
    name = str(channel.get("name") or "generic")[:80]
    parsed = urllib.parse.urlparse(endpoint)
    if parsed.scheme not in {"https", "http"} or not parsed.hostname:
        return {"ok": False, "channel": name, "message": "Outreach URL must be HTTP(S)"}
    interval = float(channel.get("min_interval_seconds") or 900)
    remaining = cooldown_remaining(f"channel:{name}", interval)
    if remaining:
        return {
            "ok": False,
            "channel": name,
            "message": f"channel cooldown: {remaining}s",
            "retry_after_seconds": remaining,
        }
    content = str(job.get("content") or job.get("message") or "")
    field = str(channel.get("content_field") or "content")
    body = dict(channel.get("extra_fields") or {})
    body[field] = content[: int(channel.get("max_chars") or 1900)]
    headers = {"Content-Type": "application/json"}
    headers.update(
        {str(key): str(value) for key, value in dict(channel.get("headers") or {}).items()}
    )
    status, response = request_json("POST", endpoint, headers, body)
    ok = 200 <= status < 300
    if ok:
        record_success(f"channel:{name}")
    return {
        "ok": ok,
        "message": "Generic outreach confirmed" if ok else f"Generic outreach returned HTTP {status}",
        "channel": name,
        "status": status,
        "response": response,
    }


def generic_posts(job: dict) -> list[dict]:
    channels = []
    endpoint = os.environ.get("TELEDRA_OUTREACH_URL", "").strip()
    if endpoint:
        token = os.environ.get("TELEDRA_OUTREACH_TOKEN", "").strip()
        channels.append(
            {
                "name": "environment",
                "enabled": True,
                "url": endpoint,
                "headers": {"Authorization": f"Bearer {token}"} if token else {},
                "min_interval_seconds": 0,
            }
        )
    configured = load_json(channels_config_path(), {}).get("channels", [])
    channels.extend(channel for channel in configured if isinstance(channel, dict))
    seen = set()
    results = []
    for channel in channels:
        endpoint = str(channel.get("url") or "").strip()
        if not channel.get("enabled") or not endpoint or endpoint in seen:
            continue
        seen.add(endpoint)
        results.append(generic_post_to(channel, job))
    return results


def main() -> int:
    try:
        job = json.load(sys.stdin)
    except (ValueError, TypeError) as error:
        return emit(False, f"Invalid outreach payload: {error}")
    if not isinstance(job, dict):
        return emit(False, "Outreach payload must be a JSON object")

    action = str(job.get("action") or "post").strip().lower()
    if action == "moltbook_inbox":
        ok, message, data = moltbook_inbox()
        return emit(ok, message, data)
    if action in {"moltbook_post", "moltbook_comment", "moltbook_upvote"}:
        ok, message, data = moltbook_request(action, job)
        return emit(ok, message, data)

    if action != "post":
        return emit(False, f"Unsupported outreach action: {action}")

    # A normal diplomacy dispatch may target every operator-configured channel,
    # matching V1 while preserving the explicit Rust approval gate.
    content = str(job.get("content") or job.get("message") or "").strip()
    if not content:
        return emit(False, "Diplomacy message is empty")
    results = generic_posts(job)
    config = moltbook_config()
    if config.get("enabled"):
        molt_job = {
            "title": job.get("title") or content.splitlines()[0][:120],
            "content": content,
            "submolt_name": job.get("submolt_name"),
        }
        molt_ok, molt_message, molt_data = moltbook_request("moltbook_post", molt_job)
        results.append({"ok": molt_ok, "message": molt_message, **molt_data})
    posted = any(result.get("ok") for result in results)
    if posted:
        return emit(True, "Approved diplomacy dispatch confirmed", {"results": results})
    return emit(
        False,
        "No configured outreach channel accepted the dispatch",
        {"results": results, "channels_config": str(channels_config_path())},
    )


if __name__ == "__main__":
    raise SystemExit(main())
