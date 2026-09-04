"""Lane-aware public opportunity scout. It never applies, posts, or moves money."""

from __future__ import annotations

from html.parser import HTMLParser
import ipaddress
import json
import os
from pathlib import Path
import socket
import sys
import tempfile
import time
from urllib.parse import parse_qs, quote_plus, unquote, urljoin, urlparse
from urllib.request import HTTPRedirectHandler, Request, build_opener


MAX_BODY = 512 * 1024
QUEUE_CAP = 600
QUERIES = [
    ("freelance Twitch stream overlay design commission paid", "you"),
    ("custom animated emote commission marketplace paid", "you"),
    ("remote short voice over narration gig paid", "you"),
    ("sell custom emote art pack marketplace payout", "agent"),
    ("royalty free original music licensing marketplace submit payout", "agent"),
    ("sell generative art game asset pack marketplace", "agent"),
    ("open source bug bounty cash reward program", "bounty"),
    ("public web application bug bounty payout list", "bounty"),
    ("people frustrated paying subscription for simple software tool", "idea"),
    ("offline one time purchase alternative to subscription software", "idea"),
]
NOISE_HOSTS = {
    "google.com", "facebook.com", "apps.apple.com", "play.google.com",
    "vinted.com", "pinterest.com",
}
NOISE_WORDS = ("porn", "xxx", "casino", "gambling", "app store")


def output(ok, message, data=None, exit_code=0):
    print(json.dumps({
        "ok": ok,
        "capability": "treasury",
        "message": str(message)[:1000],
        "data": data or {},
    }, ensure_ascii=False))
    return exit_code


def public_url(value):
    parsed = urlparse(value)
    if parsed.scheme not in ("http", "https") or not parsed.hostname:
        raise ValueError("only public HTTP(S) opportunity URLs are allowed")
    if parsed.username or parsed.password:
        raise ValueError("credential-bearing URLs are refused")
    host = parsed.hostname.rstrip(".").lower()
    if host == "localhost" or host.endswith((".local", ".localhost", ".internal")):
        raise ValueError("local opportunity URL refused")
    addresses = socket.getaddrinfo(host, parsed.port or (443 if parsed.scheme == "https" else 80))
    if not addresses:
        raise ValueError("opportunity hostname did not resolve")
    for address in addresses:
        ip = ipaddress.ip_address(address[4][0])
        if not ip.is_global:
            raise ValueError("private or special-purpose opportunity address refused")
    return value


class SafeRedirects(HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        public_url(urljoin(request.full_url, new_url))
        return super().redirect_request(request, fp, code, message, headers, new_url)


OPENER = build_opener(SafeRedirects())


def fetch(url):
    public_url(url)
    request = Request(url, headers={"User-Agent": "Teledra-V2-Treasury/1.0"})
    with OPENER.open(request, timeout=12) as response:
        body = response.read(MAX_BODY + 1)
        if len(body) > MAX_BODY:
            raise ValueError("opportunity feed exceeded 512 KiB")
        return body.decode("utf-8", errors="replace"), response.geturl()


class LinkParser(HTMLParser):
    def __init__(self):
        super().__init__()
        self.links = []
        self.current = None
        self.text = []

    def handle_starttag(self, tag, attrs):
        if tag == "a":
            self.current = dict(attrs).get("href")
            self.text = []

    def handle_data(self, data):
        if self.current:
            self.text.append(data)

    def handle_endtag(self, tag):
        if tag == "a" and self.current:
            title = " ".join("".join(self.text).split())
            self.links.append((title, self.current))
            self.current = None
            self.text = []


def unwrap_duckduckgo(url):
    parsed = urlparse(url)
    query = parse_qs(parsed.query)
    if "uddg" in query:
        return unquote(query["uddg"][0])
    return url


def links_from_body(body, base_url):
    stripped = body.lstrip()
    if stripped.startswith(("[", "{")):
        try:
            payload = json.loads(body)
            values = payload if isinstance(payload, list) else payload.get("items", payload.get("results", []))
            links = []
            for item in values if isinstance(values, list) else []:
                if isinstance(item, dict):
                    url = item.get("url") or item.get("link")
                    title = item.get("title") or item.get("name") or "opportunity"
                    if url:
                        links.append((str(title), str(url)))
            return links
        except Exception:
            pass
    parser = LinkParser()
    parser.feed(body)
    links = []
    for title, url in parser.links:
        absolute = unwrap_duckduckgo(urljoin(base_url, url))
        if absolute.startswith(("http://", "https://")):
            links.append((title or urlparse(absolute).hostname or "opportunity", absolute))
    return links


def classify(title, url, default):
    value = f"{title} {url}".lower()
    if any(word in value for word in ("bug bounty", "vulnerability", "hackerone", "bugcrowd", " vdp")):
        return "bounty"
    if any(word in value for word in ("voice over", "voiceover", "narration", "overlay", "emote", "emoji")):
        return "you" if default != "agent" else "agent"
    if default == "idea" and any(word in value for word in ("subscription", "saas", "one time", "offline", "self-hosted")):
        return "idea"
    return default


def noise(title, url):
    parsed = urlparse(url)
    host = (parsed.hostname or "").lower()
    value = f" {title.lower()} {url.lower()} "
    return any(host == blocked or host.endswith(f".{blocked}") for blocked in NOISE_HOSTS) or any(word in value for word in NOISE_WORDS)


def load_json(path, default):
    try:
        with path.open("r", encoding="utf-8") as handle:
            return json.load(handle)
    except Exception:
        return default


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    handle, temporary = tempfile.mkstemp(prefix=path.name, suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(handle, "w", encoding="utf-8") as stream:
            json.dump(value, stream, indent=2, ensure_ascii=False)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        try:
            os.unlink(temporary)
        except OSError:
            pass


def main():
    data_root = Path(os.environ.get("TELEDRA_DATA_DIR", Path(tempfile.gettempdir()) / "Teledra V2"))
    root = data_root / "treasury"
    state_path = root / "state.json"
    queue_path = root / "work_queue.json"
    ledger_path = root / "scout_ledger.jsonl"
    state = load_json(state_path, {"index": 0})
    index = int(state.get("index", 0))
    query, lane = QUERIES[index % len(QUERIES)]
    state["index"] = index + 1
    atomic_json(state_path, state)

    configured = [part.strip() for part in os.environ.get("TELEDRA_TREASURY_URLS", "").split(",") if part.strip()]
    targets = configured[:8] or [f"https://html.duckduckgo.com/html/?q={quote_plus(query)}"]
    findings = []
    errors = []
    for target in targets:
        try:
            body, final_url = fetch(target)
            findings.extend(links_from_body(body, final_url))
        except Exception as error:
            errors.append(f"{target}: {error}")

    unique = []
    seen = set()
    for title, url in findings:
        try:
            public_url(url)
        except Exception:
            continue
        normalized = url.split("#", 1)[0]
        if normalized in seen or noise(title, normalized):
            continue
        seen.add(normalized)
        unique.append((" ".join(title.split())[:160] or "opportunity", normalized))
        if len(unique) == 8:
            break

    queue = load_json(queue_path, [])
    if not isinstance(queue, list):
        queue = []
    known = {item.get("url") for item in queue if isinstance(item, dict)}
    now = int(time.time())
    added = []
    for title, url in unique:
        if url in known:
            continue
        item_lane = classify(title, url, lane)
        item = {
            "ts": now,
            "lane": item_lane,
            "title": title,
            "url": url,
            "query": query,
            "status": "new",
            "contact": os.environ.get("TELEDRA_TREASURY_CONTACT", "") if item_lane == "you" else "",
            "action": "Human review required; the court never applies, transacts, or pays a deposit.",
        }
        queue.append(item)
        added.append(item)
        known.add(url)
    atomic_json(queue_path, queue[-QUEUE_CAP:])
    root.mkdir(parents=True, exist_ok=True)
    with ledger_path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps({
            "ts": now, "query": query, "lane": lane, "found": len(unique),
            "added": len(added), "errors": errors[:4],
        }, ensure_ascii=False) + "\n")

    message = f"Scouted {len(unique)} lead(s); queued {len(added)} new [{lane}] item(s); no applications or transactions attempted"
    return output(True, message, {
        "query": query,
        "lane": lane,
        "found": len(unique),
        "queued": len(added),
        "items": added[:5],
        "errors": errors[:4],
        "queue_path": str(queue_path),
    })


if __name__ == "__main__":
    sys.exit(main())
