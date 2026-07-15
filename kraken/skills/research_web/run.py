"""research_web â€” search the web, fetch pages, and spawn research_synth."""

from __future__ import annotations

import json
import os
import re
import time
import base64
import urllib.parse
import requests
from bs4 import BeautifulSoup

from kraken.kernel import query_guard, research_cache

USER_AGENT = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"


def _generate_queries(question: str, llm) -> list[str]:
    prompt = (
        f"Generate 2 distinct, concise search queries (keywords only) to search the web for answering this question: '{question}'.\n"
        "Crucial rules for query formatting:\n"
        "1. Wrap only the specific library or tool name in double quotes (e.g. \"egui\" or \"emilk/egui\").\n"
        "2. Do NOT wrap 'Rust' and the library name together in a single quoted phrase (e.g. do NOT generate \"Rust egui\" or \"Rust egui maintainer\"). Keep 'Rust' as an unquoted separate keyword.\n"
        "Return a JSON list of strings. Example: [\"\\\"egui\\\" maintainer\", \"\\\"emilk/egui\\\" repository\"]. Do not write anything else."
    )
    try:
        res = llm.generate_json(prompt)
        if isinstance(res, list):
            return [str(q) for q in res[:2]]
    except Exception:
        pass
    # fallback: clean simple terms
    clean = re.sub(r"[^\w\s]", "", question)
    words = [w for w in clean.split() if len(w) > 3][:4]
    return [" ".join(words)] if words else [question]


def _load_json_file(path: str) -> dict:
    if os.path.exists(path):
        try:
            with open(path, "r", encoding="utf-8") as fh:
                return json.load(fh)
        except Exception:
            pass
    return {}


def _save_json_file(path: str, data: dict) -> None:
    try:
        with open(path, "w", encoding="utf-8") as fh:
            json.dump(data, fh, indent=2)
    except Exception:
        pass


def _get_searxng_instances(workspace: str, log) -> list[str]:
    path = os.path.join(workspace, "searxng_instances.json")
    if os.path.exists(path):
        try:
            mtime = os.path.getmtime(path)
            if time.time() - mtime < 12 * 3600:
                with open(path, "r", encoding="utf-8") as fh:
                    return json.load(fh)
        except Exception:
            pass
    log("Fetching fresh SearxNG instances list from searx.space...")
    try:
        r = requests.get("https://searx.space/data/instances.json", timeout=6)
        if r.status_code == 200:
            data = r.json()
            candidates = []
            for url, inst in data.get("instances", {}).items():
                if not url.startswith("https://"):
                    continue
                initial = inst.get("timing", {}).get("initial", {})
                search = inst.get("timing", {}).get("search", {})
                if initial.get("success_percentage", 0) > 95.0 and search.get("success_percentage", 0) > 95.0:
                    median = search.get("all", {}).get("median", 99.0)
                    if median < 3.0:
                        candidates.append((url, median))
            candidates.sort(key=lambda x: x[1])
            best_urls = [x[0] for x in candidates]
            if best_urls:
                _save_json_file(path, best_urls)
                return best_urls
    except Exception as e:
        log(f"Failed to fetch SearxNG instances: {e}")

    return [
        "https://searx.be",
        "https://opnxng.com",
        "https://copp.gg",
        "https://search.mdosch.de",
        "https://searxng.website",
        "https://search.rhscz.eu",
        "https://searx.ononoki.org",
        "https://baresearch.org"
    ]


def _query_engine(engine_name: str, query: str, log) -> list[str]:
    headers = {
        "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        "Accept-Language": "en-US,en;q=0.5",
        "Referer": "https://www.google.com/"
    }
    links = []

    if engine_name == "ddg_lite":
        url = "https://lite.duckduckgo.com/lite/"
        try:
            resp = requests.post(url, data={"q": query}, headers=headers, timeout=12)
        except Exception:
            resp = None
        if resp is None or resp.status_code != 200 or not resp.text.strip():
            resp = requests.get(url, params={"q": query}, headers=headers, timeout=12)

        if resp.status_code != 200:
            raise RuntimeError(f"HTTP status {resp.status_code}")
        text = resp.text.lower()
        if "anomaly-modal" in text or "captcha" in text or "challenge-form" in text or "not loading?" in text:
            raise RuntimeError("DDG Lite captcha challenge detected")

        soup = BeautifulSoup(resp.text, "html.parser")
        result_anchors = soup.find_all("a", class_="result-link")
        if not result_anchors:
            result_anchors = soup.find_all("a")
        for a in result_anchors:
            href = a.get("href", "")
            if not href:
                continue
            if href.startswith("//"):
                href = "https:" + href
            if "duckduckgo.com/l/?" in href or "/l/?" in href:
                parsed = urllib.parse.urlparse(href)
                qs = urllib.parse.parse_qs(parsed.query)
                if "uddg" in qs:
                    href = qs["uddg"][0]
            if href.startswith("http") and "duckduckgo.com" not in href:
                if href not in links:
                    links.append(href)

    elif engine_name == "ddg_html":
        url = "https://html.duckduckgo.com/html/"
        resp = requests.post(url, data={"q": query}, headers=headers, timeout=12)
        if resp.status_code != 200:
            raise RuntimeError(f"HTTP status {resp.status_code}")
        text = resp.text.lower()
        if "anomaly-modal" in text or "captcha" in text or "challenge-form" in text or "not loading?" in text:
            raise RuntimeError("DDG HTML captcha challenge detected")

        soup = BeautifulSoup(resp.text, "html.parser")
        for a in soup.find_all("a"):
            href = a.get("href", "")
            if href.startswith("http") and "duckduckgo.com" not in href:
                if href not in links:
                    links.append(href)

    elif engine_name == "bing":
        url = "https://www.bing.com/search"
        bing_headers = dict(headers)
        bing_headers["User-Agent"] = "Mozilla/5.0"
        resp = requests.get(url, params={"q": query, "setlang": "en", "cc": "us"}, headers=bing_headers, timeout=12)
        if resp.status_code != 200:
            raise RuntimeError(f"HTTP status {resp.status_code}")
        text = resp.text.lower()
        if "captcha" in text or "quick security check" in text or "identity verification" in text:
            raise RuntimeError("Bing captcha challenge detected")

        soup = BeautifulSoup(resp.text, "html.parser")
        result_elements = soup.find_all(class_=re.compile("b_algo"))
        if not result_elements:
            result_elements = soup.find_all("h2")
        for el in result_elements:
            for a in el.find_all("a"):
                href = a.get("href", "")
                if "bing.com/ck/a?" in href:
                    parsed = urllib.parse.urlparse(href)
                    qs = urllib.parse.parse_qs(parsed.query)
                    if "u" in qs:
                        val = qs["u"][0]
                        if val.startswith("a1"):
                            val = val[2:]
                        val = val.replace("-", "+").replace("_", "/")
                        val += "=" * (4 - len(val) % 4) if len(val) % 4 else ""
                        try:
                            href = base64.b64decode(val).decode("utf-8")
                        except Exception:
                            continue
                if href.startswith("http") and "bing.com" not in href and "microsoft.com" not in href:
                    if href not in links:
                        links.append(href)

    elif engine_name == "yahoo":
        url = "https://search.yahoo.com/search"
        resp = requests.get(url, params={"p": query}, headers=headers, timeout=12)
        if resp.status_code != 200:
            raise RuntimeError(f"HTTP status {resp.status_code}")
        text = resp.text.lower()
        if "consent.yahoo.com" in resp.url or "guce.yahoo.com" in resp.url or "read this before you proceed" in text or "les dette fÃ¸r du" in text:
            raise RuntimeError("Yahoo consent wall/captcha detected")

        soup = BeautifulSoup(resp.text, "html.parser")
        result_elements = soup.find_all(class_=re.compile("algo|compTitle"))
        if not result_elements:
            result_elements = soup.find_all("h3")
        for el in result_elements:
            for a in el.find_all("a"):
                href = a.get("href", "")
                if "r.search.yahoo.com/" in href:
                    for part in href.split("/"):
                        if part.startswith("RU="):
                            val = urllib.parse.unquote(part[3:])
                            if "yahoo.com" not in val and "yahoo.co" not in val and val.startswith("http"):
                                if val not in links:
                                    links.append(val)

    elif engine_name == "mojeek":
        url = "https://www.mojeek.com/search"
        resp = requests.get(url, params={"q": query}, headers=headers, timeout=12)
        if resp.status_code != 200:
            raise RuntimeError(f"HTTP status {resp.status_code}")
        text = resp.text.lower()
        if "captcha" in text or "verification required" in text or "please complete the challenge" in text:
            raise RuntimeError("Mojeek captcha challenge detected")

        soup = BeautifulSoup(resp.text, "html.parser")
        result_elements = soup.find_all(class_=re.compile("result"))
        if not result_elements:
            result_elements = soup.find_all("h2")
        for el in result_elements:
            for a in el.find_all("a"):
                href = a.get("href", "")
                if href.startswith("http") and "mojeek.com" not in href:
                    if href not in links:
                        links.append(href)

    elif engine_name.startswith("searxng:"):
        instance_url = engine_name.split("searxng:", 1)[1]
        url = instance_url.rstrip("/") + "/search"
        resp = requests.get(url, params={"q": query}, headers=headers, timeout=12)
        if resp.status_code != 200:
            raise RuntimeError(f"HTTP status {resp.status_code}")
        text = resp.text.lower()
        if "anubis" in text or "captcha" in text or "not a bot" in text or "challenge" in text:
            raise RuntimeError("SearxNG instance captcha challenge/Anubis wall detected")

        soup = BeautifulSoup(resp.text, "html.parser")
        for a in soup.find_all("a"):
            href = a.get("href", "")
            if href.startswith("http") and instance_url not in href and "searx" not in href:
                if href not in links:
                    links.append(href)

    return links


def _search_web_resilient(query: str, workspace: str, root: str, log) -> tuple[list[str], str]:
    cache_path = os.path.join(workspace, "research_query_cache.json")
    cache = _load_json_file(cache_path)
    q_key = query.lower().strip()

    if q_key in cache:
        entry = cache[q_key]
        cached_time = entry.get("timestamp", 0)
        if time.time() - cached_time < 24 * 3600:
            if research_cache.verify_entry(root, entry):
                cached_links = entry.get("links", [])
                engine = entry.get("engine", "unknown")
                log(f"Query '{query}' resolved from signed cache (served by {engine})")
                return cached_links, f"cache:{engine}"
            log(f"Query '{query}' cache entry rejected (unsigned or tampered)")
            del cache[q_key]
            _save_json_file(cache_path, cache)

    cooldown_path = os.path.join(workspace, "research_engine_cooldowns.json")
    cooldowns = _load_json_file(cooldown_path)

    searxng_urls = _get_searxng_instances(workspace, log)

    engines = ["ddg_lite", "ddg_html", "bing", "yahoo", "mojeek"]
    for s_url in searxng_urls[:5]:
        engines.append(f"searxng:{s_url}")

    active_engines = []
    cooldown_engines = []
    now = time.time()
    for eng in engines:
        cooldown_until = cooldowns.get(eng, 0)
        if now >= cooldown_until:
            active_engines.append(eng)
        else:
            cooldown_engines.append((eng, cooldown_until))

    if not active_engines:
        log("All search engines are on cooldown! Resetting cooldown list.")
        active_engines = list(engines)
        cooldowns = {}
        _save_json_file(cooldown_path, cooldowns)
    else:
        cooldown_engines.sort(key=lambda x: x[1])
        active_engines.extend([x[0] for x in cooldown_engines])

    links = []
    successful_engine = None

    for eng in active_engines:
        time.sleep(2.0)
        log(f"Trying search engine: {eng}")
        try:
            links = _query_engine(eng, query, log)
            if links:
                log(f"Successfully retrieved {len(links)} links from {eng}")
                successful_engine = eng
                break
            else:
                log(f"Engine {eng} returned 0 results. Putting on cooldown.")
                cooldowns[eng] = time.time() + 600
                _save_json_file(cooldown_path, cooldowns)
        except Exception as e:
            log(f"Engine {eng} failed/blocked: {e}. Putting on cooldown.")
            cooldowns[eng] = time.time() + 600
            _save_json_file(cooldown_path, cooldowns)

    if successful_engine and links:
        cache[q_key] = research_cache.sign_entry(
            root, links, successful_engine, time.time()
        )
        _save_json_file(cache_path, cache)

    return links, successful_engine or "failed"


def _fetch_clean_text(url: str) -> tuple[str, str]:
    """Fetches url and returns (title, clean_body_text)."""
    try:
        headers = {"User-Agent": USER_AGENT}
        resp = requests.get(url, headers=headers, timeout=10)
        if resp.status_code == 200:
            soup = BeautifulSoup(resp.text, "html.parser")
            title = soup.title.text.strip() if soup.title else url
            # Remove script and style elements
            for script in soup(["script", "style", "header", "footer", "nav"]):
                script.decompose()
            text = soup.get_text(separator="\n")
            # Clean up whitespace
            lines = (line.strip() for line in text.splitlines())
            chunks = (phrase for line in lines for phrase in line.split("  "))
            clean_text = "\n".join(chunk for chunk in chunks if chunk)
            return title, clean_text[:8000]  # limit to 8k chars
    except Exception as e:
        return "", f"Failed to fetch {url}: {e}"
    return "", ""


def execute(job: dict, ctx: dict) -> dict:
    question = job["input"]
    llm = ctx["llm"]
    log = ctx["log"]
    root = ctx["root"]

    sanity = query_guard.query_sanity(question)
    if sanity:
        log(f"query rejected: {sanity}")
        manifest_path = os.path.join(ctx["workdir"], "sources.json")
        with open(manifest_path, "w", encoding="utf-8") as fh:
            json.dump([], fh)
        return {
            "ok": False,
            "output": os.path.relpath(manifest_path, root),
            "notes": f"query rejected: {sanity}",
        }

    log(f"Generating search queries for: {question}")
    queries = _generate_queries(question, llm)
    log(f"Search queries: {queries}")

    candidate_links = []
    attribution = {}
    query_results = []
    for query in queries:
        log(f"Searching for query: '{query}'")
        links, engine = _search_web_resilient(query, ctx["workspace"], root, log)
        attribution[query] = {
            "engine": engine,
            "links_count": len(links)
        }
        query_results.append(links)

    # Round-robin interleaving of query results to ensure diversity
    max_len = max(len(r) for r in query_results) if query_results else 0
    for i in range(max_len):
        for r in query_results:
            if i < len(r):
                link = r[i]
                ignore_domains = ["steampowered.com", "steamcommunity.com", "facepunch.com", "softonic.com", "epicgames.com", "thegreatdividetour.us"]
                if any(domain in link.lower() for domain in ignore_domains):
                    continue
                if link not in candidate_links:
                    candidate_links.append(link)

    attr_path = os.path.join(ctx["workdir"], "engine_attribution.json")
    _save_json_file(attr_path, attribution)

    log(f"Found {len(candidate_links)} unique candidate links. Fetching top links...")
    fetched_sources = []

    # Try to fetch up to 4 successful pages, aiming for >=3
    for url in candidate_links[:8]:
        if len(fetched_sources) >= 4:
            break
        log(f"Fetching source: {url}")
        title, text = _fetch_clean_text(url)
        if title and len(text) > 200:
            filename = f"source_{len(fetched_sources)}.txt"
            filepath = os.path.join(ctx["workdir"], filename)
            with open(filepath, "w", encoding="utf-8") as fh:
                fh.write(text)
            fetched_sources.append({
                "url": url,
                "title": title,
                "path": os.path.abspath(filepath)
            })
            log(f"Successfully fetched and saved: {title}")
        else:
            log(f"Skipped/failed to fetch content from: {url}")

    # Write manifest file
    manifest_path = os.path.join(ctx["workdir"], "sources.json")
    with open(manifest_path, "w", encoding="utf-8") as fh:
        json.dump(fetched_sources, fh, indent=2)

    # Acceptance requires >=3 real sources. Return ok=False to trigger retry/feedback loop if failed.
    if len(fetched_sources) < 3:
        log(f"Fetched fewer than 3 sources ({len(fetched_sources)}). Requeuing...")
        return {
            "ok": False,
            "output": os.path.relpath(manifest_path, ctx["root"]),
            "notes": f"Only fetched {len(fetched_sources)} sources (required >= 3)"
        }

    # Spawn research_synth child job
    synth_input = json.dumps({
        "question": question,
        "sources": fetched_sources
    }, ensure_ascii=False)

    return {
        "ok": True,
        "output": os.path.relpath(manifest_path, ctx["root"]),
        "notes": f"Fetched {len(fetched_sources)} sources",
        "children": [
            {"skill": "research_synth", "input": synth_input}
        ]
    }
