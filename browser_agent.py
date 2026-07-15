import os
import sys
import json
import re
import html
import asyncio
import argparse
import ipaddress
import urllib.parse
from typing import List, Tuple, Set

import aiohttp
import chardet
import defusedxml.ElementTree as ET
import tldextract
from bs4 import BeautifulSoup
from sklearn.feature_extraction.text import TfidfVectorizer
from sklearn.metrics.pairwise import cosine_similarity
import random

# Constants
MAX_SEARCH_RESULTS = 4
MAX_PAGE_PARAGRAPHS = 7
MAX_RESPONSE_BYTES = 2_000_000
MAX_JSON_RAW_CHARS = 24_000
MAX_SOURCE_EXCERPT_CHARS = 5_000


def bounded_excerpt(text: str, limit: int = MAX_SOURCE_EXCERPT_CHARS) -> str:
    """Return complete evidence clauses only; never promote a cut-off fragment."""
    compact = " ".join(text.split())
    if len(compact) <= limit:
        return compact
    candidate = compact[:limit]
    known_abbreviations = {
        "dr",
        "etc",
        "jr",
        "mr",
        "mrs",
        "ms",
        "prof",
        "sr",
        "st",
        "vs",
    }
    last_boundary = -1
    inside_double_quote = False
    inside_single_quote = False
    parenthesis_depth = 0
    bracket_depth = 0
    brace_depth = 0
    for index, character in enumerate(candidate):
        previous = candidate[index - 1] if index else ""
        following = candidate[index + 1] if index + 1 < len(candidate) else ""
        if character == '"':
            inside_double_quote = not inside_double_quote
            continue
        if character == "“":
            inside_double_quote = True
            continue
        if character == "”":
            inside_double_quote = False
            continue
        if character in "'‘’" and not (previous.isalnum() and following.isalnum()):
            if character == "‘":
                inside_single_quote = True
            elif character == "’":
                inside_single_quote = False
            else:
                inside_single_quote = not inside_single_quote
            continue
        if character == "(":
            parenthesis_depth += 1
        elif character == ")":
            parenthesis_depth = max(0, parenthesis_depth - 1)
        elif character == "[":
            bracket_depth += 1
        elif character == "]":
            bracket_depth = max(0, bracket_depth - 1)
        elif character == "{":
            brace_depth += 1
        elif character == "}":
            brace_depth = max(0, brace_depth - 1)
        if (
            inside_double_quote
            or inside_single_quote
            or parenthesis_depth
            or bracket_depth
            or brace_depth
        ):
            continue
        if character not in ".;!?":
            continue
        boundary_end = index + 1
        if character == ".":
            if previous == "." or following == ".":
                continue
            if previous.isdigit() and following.isdigit():
                continue
            token_start = index
            while token_start > 0 and candidate[token_start - 1].isalnum():
                token_start -= 1
            token = candidate[token_start:index].lower()
            dotted = len(token) == 1 and token_start > 0 and candidate[token_start - 1] == "."
            if dotted or token in known_abbreviations:
                continue
        while boundary_end < len(candidate) and candidate[boundary_end] in "\"')]}”’":
            boundary_end += 1
        if boundary_end == len(candidate) or candidate[boundary_end].isspace():
            last_boundary = boundary_end
    if last_boundary < max(30, limit // 4):
        return ""
    return candidate[:last_boundary].strip()

# Dynamic User-Agent rotation for bot mitigation evasion
USER_AGENTS = [
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Edge/124.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:125.0) Gecko/20100101 Firefox/125.0",
]

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

def is_safe_public_http_url(value: str) -> bool:
    """Reject obvious local/private targets before the autonomous fetcher sees them."""
    try:
        parsed = urllib.parse.urlparse(value)
        if parsed.scheme not in ("http", "https") or not parsed.hostname:
            return False
        hostname = parsed.hostname.rstrip(".").lower()
        if hostname == "localhost" or hostname.endswith((".localhost", ".local")):
            return False
        try:
            address = ipaddress.ip_address(hostname)
        except ValueError:
            return True
        return not (
            address.is_private
            or address.is_loopback
            or address.is_link_local
            or address.is_multicast
            or address.is_reserved
            or address.is_unspecified
        )
    except ValueError:
        return False


async def request_text(session: aiohttp.ClientSession, url: str, timeout: int = 10) -> str:
    if not is_safe_public_http_url(url):
        raise RuntimeError("Blocked non-public or malformed URL")
    try:
        async with session.get(
            url, 
            headers={"User-Agent": random.choice(USER_AGENTS), "Accept-Language": "en-US,en;q=0.9"}, 
            timeout=aiohttp.ClientTimeout(total=timeout)
        ) as response:
            response.raise_for_status()
            if not is_safe_public_http_url(str(response.url)):
                raise RuntimeError("Blocked redirect to a non-public URL")
            content_type = response.headers.get("Content-Type", "").lower()
            if content_type and not any(
                kind in content_type
                for kind in ("text/", "json", "xml", "xhtml", "rss", "atom")
            ):
                raise RuntimeError(f"Unsupported response content type: {content_type}")
            declared = response.content_length
            if declared is not None and declared > MAX_RESPONSE_BYTES:
                raise RuntimeError(
                    f"Response exceeded {MAX_RESPONSE_BYTES} byte safety limit"
                )
            raw = await response.content.read(MAX_RESPONSE_BYTES + 1)
            if len(raw) > MAX_RESPONSE_BYTES:
                raise RuntimeError(
                    f"Response exceeded {MAX_RESPONSE_BYTES} byte safety limit"
                )
            # Dynamic robust byte-sniffing instead of static regex
            encoding = chardet.detect(raw)['encoding'] or 'utf-8'
            return raw.decode(encoding, errors='ignore')
    except Exception as e:
        raise RuntimeError(f"Request failed: {e}")

def is_url(value: str) -> bool:
    parsed = urllib.parse.urlparse(value)
    return parsed.scheme in ("http", "https") and bool(parsed.netloc)

def normalize_url(href: str, base_url: str = None) -> str | None:
    if not href:
        return None
    href = html.unescape(href).strip()
    if not href or href.startswith("#") or href.startswith("javascript:"):
        return None

    if href.startswith("//"):
        href = "https:" + href

    import urllib.parse
    if href.startswith("/l/") or href.startswith("/url?") or "duckduckgo.com/l/?" in href:
        parsed = urllib.parse.urlparse(href)
        params = urllib.parse.parse_qs(parsed.query)
        for key in ("uddg", "q", "u"):
            if params.get(key):
                return params[key][0]

    if base_url:
        href = urllib.parse.urljoin(base_url, href)

    parsed = urllib.parse.urlparse(href)
    if parsed.scheme in ("http", "https") and parsed.netloc:
        return href
    return None

def should_skip_url(url: str) -> bool:
    if not is_safe_public_http_url(url):
        return True
    ext = tldextract.extract(url)
    domain = f"{ext.domain}.{ext.suffix}".lower()
    skipped_domains = {"duckduckgo.com", "google.com", "bing.com", "googleusercontent.com"}
    return domain in skipped_domains


def source_quality(url: str, title: str = "") -> Tuple[str, float]:
    """Small deterministic prior. The synthesizer still has to ground every claim."""
    parsed = urllib.parse.urlparse(url)
    host = (parsed.hostname or "").lower()
    path = parsed.path.lower()
    label = "secondary"
    score = 0.55

    if host.endswith(".gov") or ".gov." in host:
        label, score = "primary", 0.95
    elif host.endswith(".edu") or ".edu." in host:
        label, score = "primary", 0.88
    elif host in {"arxiv.org", "doi.org", "pubmed.ncbi.nlm.nih.gov"}:
        label, score = "primary", 0.90
    elif any(token in host for token in ("docs.", "developer.", "standards.")):
        label, score = "primary", 0.84
    elif any(token in path for token in ("/docs/", "/documentation/", "/reference/", "/spec")):
        label, score = "primary", 0.80
    elif host.endswith("wikipedia.org"):
        label, score = "tertiary", 0.42

    lower_title = title.lower()
    if any(
        marker in lower_title
        for marker in ("error", "not found", "security check", "before you continue")
    ):
        label, score = "unusable", 0.05
    return label, score


def parse_research_output(query: str, content: str) -> dict:
    """Turn the legacy readable output into a machine-checkable evidence bundle."""
    sources = []
    diagnostics = []
    current_title = None
    current_url = None
    current_lines = []
    in_diagnostics = False

    def flush_source() -> None:
        nonlocal current_title, current_url, current_lines
        excerpt = bounded_excerpt(
            " ".join(line.strip() for line in current_lines if line.strip())
        )
        if current_title and current_url and excerpt:
            kind, quality = source_quality(current_url, current_title)
            if kind != "unusable":
                host = urllib.parse.urlparse(current_url).hostname or ""
                sources.append(
                    {
                        "id": f"S{len(sources) + 1}",
                        "title": current_title.strip(),
                        "url": current_url.strip(),
                        "domain": host.lower(),
                        "source_kind": kind,
                        "quality": quality,
                        "excerpt": excerpt,
                    }
                )
        current_title = None
        current_url = None
        current_lines = []

    for raw_line in content.splitlines():
        line = raw_line.strip()
        if line.startswith("--- CONTENT FROM "):
            flush_source()
            current_title = line[len("--- CONTENT FROM ") :].strip(" -")
            in_diagnostics = False
        elif line.startswith("--- SEARCH DIAGNOSTICS"):
            flush_source()
            in_diagnostics = True
        elif line.startswith("URL:") and current_title:
            current_url = line[4:].strip().rstrip(" -")
        elif in_diagnostics:
            if line:
                diagnostics.append(line)
        elif current_title:
            current_lines.append(line)
    flush_source()

    # Preserve a direct URL as evidence even when its page has unusual markup.
    if not sources and is_url(query) and is_safe_public_http_url(query):
        clean = " ".join(
            line.strip()
            for line in content.splitlines()
            if line.strip() and not line.startswith("---")
        )
        clean = bounded_excerpt(clean)
        if clean and not clean.lower().startswith("failed to fetch"):
            kind, quality = source_quality(query, query)
            sources.append(
                {
                    "id": "S1",
                    "title": query,
                    "url": query,
                    "domain": urllib.parse.urlparse(query).hostname or "",
                    "source_kind": kind,
                    "quality": quality,
                    "excerpt": clean,
                }
            )

    return {
        "schema_version": 1,
        "query": query,
        "sources": sources[:MAX_SEARCH_RESULTS],
        "diagnostics": diagnostics[-12:],
        "raw_text": content[:MAX_JSON_RAW_CHARS],
    }

def site_domains(query: str) -> List[str]:
    domains = []
    for match in re.findall(r"\bsite:([^\s]+)", query, flags=re.IGNORECASE):
        domain = match.strip().lower().lstrip(".")
        if domain:
            domains.append(domain)
    return domains

def matches_site_scope(query: str, url: str) -> bool:
    domains = site_domains(query)
    if not domains:
        return True
    ext = tldextract.extract(url)
    host_domain = f"{ext.domain}.{ext.suffix}".lower()
    return any(host_domain == d or host_domain.endswith("." + d) for d in domains)

def looks_relevant_tfidf(query: str, *fields: str) -> bool:
    haystack = " ".join(field or "" for field in fields)
    if not haystack.strip():
        return False
        
    # Remove site: filters from query for TF-IDF scoring
    clean_query = re.sub(r"\bsite:[^\s]+", " ", query, flags=re.IGNORECASE).strip()
    if not clean_query:
        return True
        
    try:
        vectorizer = TfidfVectorizer(stop_words='english')
        tfidf_matrix = vectorizer.fit_transform([clean_query, haystack])
        score = cosine_similarity(tfidf_matrix[0:1], tfidf_matrix[1:2])[0][0]
        return score > 0.05
    except ValueError:
        # Fallback if vocabulary is empty
        return True

def unique_urls(urls: List[str]) -> List[str]:
    seen = set()
    out = []
    for url in urls:
        url = url.split("#", 1)[0]
        if url not in seen:
            seen.add(url)
            out.append(url)
    return out

async def search_duckduckgo(session: aiohttp.ClientSession, query: str) -> List[str]:
    import urllib.parse
    url = "https://html.duckduckgo.com/html/?q=" + urllib.parse.quote(query)
    html_text = await request_text(session, url, timeout=8)
    if "anomaly-modal" in html_text or "bots use DuckDuckGo" in html_text:
        raise RuntimeError("DuckDuckGo returned an anti-bot challenge")
        
    soup = BeautifulSoup(html_text, 'html.parser')
    links = []
    for a in soup.find_all('a', href=True):
        href = normalize_url(a.get('href', ''), "https://html.duckduckgo.com")
        if href and not should_skip_url(href) and matches_site_scope(query, href):
            links.append(href)
    return unique_urls(links)[:MAX_SEARCH_RESULTS]

async def search_google(session: aiohttp.ClientSession, query: str) -> List[str]:
    import urllib.parse
    url = "https://www.google.com/search?num=10&hl=en&q=" + urllib.parse.quote(query)
    html_text = await request_text(session, url, timeout=8)
    if "unusual traffic" in html_text.lower() or "sorry/index" in html_text:
        raise RuntimeError("Google returned an anti-bot challenge")

    soup = BeautifulSoup(html_text, 'html.parser')
    links = []
    for a in soup.find_all('a', href=True):
        href = normalize_url(a.get('href', ''), "https://www.google.com")
        if href and not should_skip_url(href) and matches_site_scope(query, href):
            links.append(href)
    return unique_urls(links)[:MAX_SEARCH_RESULTS]

async def search_bing_rss(session: aiohttp.ClientSession, query: str) -> List[str]:
    import urllib.parse
    url = "https://www.bing.com/search?format=rss&q=" + urllib.parse.quote(query)
    xml_text = await request_text(session, url, timeout=8)
    root = ET.fromstring(xml_text)
    links = []
    for item in root.findall("./channel/item"):
        title = item.findtext("title", "")
        desc = item.findtext("description", "")
        link_el = item.find("link")
        if link_el is not None and link_el.text:
            href = normalize_url(link_el.text)
            if (
                href
                and not should_skip_url(href)
                and matches_site_scope(query, href)
                and looks_relevant_tfidf(query, title, desc, href)
            ):
                links.append(href)
    return unique_urls(links)[:MAX_SEARCH_RESULTS]

async def search_google_news(session: aiohttp.ClientSession, query: str) -> List[Tuple[str, str, str]]:
    import urllib.parse
    url = (
        "https://news.google.com/rss/search?q="
        + urllib.parse.quote(query)
        + "&hl=en-US&gl=US&ceid=US:en"
    )
    xml_text = await request_text(session, url, timeout=10)
    root = ET.fromstring(xml_text)
    items = []
    for item in root.findall("./channel/item")[:MAX_SEARCH_RESULTS]:
        title = item.findtext("title", "").strip()
        desc = item.findtext("description", "").strip()
        pub_date = item.findtext("pubDate", "").strip()
        
        soup = BeautifulSoup(desc, 'html.parser')
        clean_desc = soup.get_text(separator=' ', strip=True)
        
        if title and looks_relevant_tfidf(query, title, clean_desc):
            items.append((title, clean_desc, pub_date))
    return items

async def search_wikipedia(session: aiohttp.ClientSession, query: str) -> str:
    import urllib.parse
    search_url = (
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch="
        + urllib.parse.quote(query)
        + "&format=json"
    )
    try:
        data = json.loads(await request_text(session, search_url, timeout=8))
        search_results = data.get("query", {}).get("search", [])
        if not search_results:
            return ""

        title = search_results[0]["title"]
        extract_url = (
            "https://en.wikipedia.org/w/api.php?action=query&prop=extracts"
            "&exintro&explaintext&titles="
            + urllib.parse.quote(title)
            + "&format=json"
        )
        ext_data = json.loads(await request_text(session, extract_url, timeout=8))
        pages = ext_data.get("query", {}).get("pages", {})
        for page_info in pages.values():
            extract = page_info.get("extract", "")
            if extract:
                page_url = "https://en.wikipedia.org/wiki/" + urllib.parse.quote(
                    title.replace(" ", "_")
                )
                return (
                    f"--- CONTENT FROM Wikipedia: {title}\n"
                    f"URL: {page_url} ---\n{extract}"
                )
    except Exception as exc:
        print(f"Warning: Wikipedia lookup failed: {exc}", file=sys.stderr)
    return ""

async def fetch_page_summary(session: aiohttp.ClientSession, url: str) -> str:
    html_text = await request_text(session, url, timeout=10)
    soup = BeautifulSoup(html_text, 'html.parser')
    
    title = soup.title.string.strip() if soup.title and soup.title.string else url
    
    # Remove script, style elements
    for script in soup(["script", "style"]):
        script.extract()
        
    paragraphs = []
    seen = set()
    
    for tag in soup.find_all(['p', 'h1', 'h2', 'h3', 'article', 'li']):
        text = tag.get_text(separator=' ', strip=True)
        # Use a more natural language heuristic instead of a rigid 45-char limit
        if text and len(text.split()) > 4 and text not in seen:
            seen.add(text)
            paragraphs.append(text)
        if len(paragraphs) >= MAX_PAGE_PARAGRAPHS:
            break
            
    if not paragraphs:
        return ""
    return f"--- CONTENT FROM {title}\nURL: {url} ---\n" + "\n".join(paragraphs)

def is_news_query(query: str) -> bool:
    lowered = query.lower()
    markers = ("latest", "today", "recent", "news", "breaking", "current", "this week", "2026")
    return any(marker in lowered for marker in markers)

def load_tavily_key() -> str:
    """Tavily API key from the environment or config/tavily_key.txt (optional)."""
    key = os.environ.get("TAVILY_API_KEY", "").strip()
    if key:
        return key
    try:
        base = os.path.dirname(os.path.abspath(__file__))
        with open(os.path.join(base, "config", "tavily_key.txt"), "r", encoding="utf-8") as f:
            return f.read().strip()
    except Exception:
        return ""

async def search_tavily(session: aiohttp.ClientSession, query: str, api_key: str) -> str:
    """Agent-native search via Tavily: pre-chunked, RAG-ready content with no
    CAPTCHA/anti-bot battles. Used when a key is configured; scraping engines
    remain the fallback."""
    site_scopes = re.findall(r"\bsite:([^\s]+)", query)
    clean_query = re.sub(r"\bsite:[^\s]+", " ", query).strip() or query
    payload = {
        "api_key": api_key,
        "query": clean_query,
        "search_depth": "advanced",
        "max_results": MAX_SEARCH_RESULTS,
        "include_answer": True,
    }
    if site_scopes:
        payload["include_domains"] = site_scopes
    if is_news_query(query):
        payload["topic"] = "news"
    async with session.post(
        "https://api.tavily.com/search",
        json=payload,
        timeout=aiohttp.ClientTimeout(total=25),
    ) as resp:
        if resp.status != 200:
            raise RuntimeError(f"Tavily returned HTTP {resp.status}")
        data = await resp.json()

    sections = []
    answer = (data.get("answer") or "").strip()
    if answer:
        sections.append(f"--- TAVILY SYNTHESIZED ANSWER ---\n{answer}")
    for result in data.get("results", [])[:MAX_SEARCH_RESULTS]:
        title = (result.get("title") or "").strip()
        url = (result.get("url") or "").strip()
        content = (result.get("content") or "").strip()
        if content:
            sections.append(f"--- CONTENT FROM {title}\nURL: {url} ---\n{content}")
    return "\n\n".join(sections).strip()

async def search_and_scrape(query: str) -> str:
    async with aiohttp.ClientSession() as session:
        if is_url(query):
            try:
                return await fetch_page_summary(session, query)
            except Exception as exc:
                return f"Failed to fetch URL {query}: {exc}"

        diagnostics = []
        sections = []

        # Preferred path: Tavily agentic search when a key is configured.
        tavily_key = load_tavily_key()
        if tavily_key:
            try:
                tavily_content = await search_tavily(session, query, tavily_key)
                if tavily_content:
                    return tavily_content
                diagnostics.append("Tavily returned no usable content; falling back to scraping engines.")
            except Exception as exc:
                diagnostics.append(f"Tavily failed ({exc}); falling back to scraping engines.")

        # Run news search concurrently if needed
        news_task = None
        if is_news_query(query):
            news_task = asyncio.create_task(search_google_news(session, query))

        # Run search engines concurrently
        search_tasks = [
            ("Google", asyncio.create_task(search_google(session, query))),
            ("Bing RSS", asyncio.create_task(search_bing_rss(session, query))),
            ("DuckDuckGo", asyncio.create_task(search_duckduckgo(session, query))),
        ]

        result_links = []
        for name, task in search_tasks:
            try:
                links = await task
                if links:
                    result_links = links
                    diagnostics.append(f"{name} returned {len(links)} result link(s).")
                    break
                diagnostics.append(f"{name} returned no usable links.")
            except Exception as exc:
                diagnostics.append(f"{name} failed: {exc}")

        # Cancel any still-running search tasks; otherwise they keep hitting the
        # network and raise "exception was never retrieved" / pending-task
        # warnings when the session closes.
        pending = [task for _, task in search_tasks if not task.done()]
        for task in pending:
            task.cancel()
        if pending:
            await asyncio.gather(*pending, return_exceptions=True)

        # Check news results
        if news_task:
            try:
                news_items = await news_task
                if news_items:
                    lines = ["--- GOOGLE NEWS RSS RESULTS ---"]
                    for title, desc, pub_date in news_items:
                        suffix = f" ({pub_date})" if pub_date else ""
                        lines.append(f"- {title}{suffix}")
                        if desc:
                            lines.append(f"  {desc}")
                    sections.append("\n".join(lines))
            except Exception as exc:
                diagnostics.append(f"Google News failed: {exc}")

        # Scrape results concurrently
        if result_links:
            scrape_tasks = [fetch_page_summary(session, link) for link in result_links]
            scraped_results = await asyncio.gather(*scrape_tasks, return_exceptions=True)
            
            scraped_sections = []
            for link, summary in zip(result_links, scraped_results):
                if isinstance(summary, Exception):
                    diagnostics.append(f"Failed to fetch {link}: {summary}")
                elif summary:
                    scraped_sections.append(summary)
                if len(scraped_sections) >= 2:
                    break
                    
            sections.extend(scraped_sections)

        # Fallback to wikipedia
        if not sections:
            wiki = await search_wikipedia(session, query)
            if wiki:
                sections.append(wiki)

        if diagnostics:
            sections.append("--- SEARCH DIAGNOSTICS ---\n" + "\n".join(diagnostics))

        return "\n\n".join(sections).strip()

def main():
    parser = argparse.ArgumentParser(description="Autonomous Web Intelligence Agent")
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit a versioned evidence bundle instead of the legacy readable text",
    )
    parser.add_argument("query", nargs="+", help="The search query or URL")
    args = parser.parse_args()
    
    query = " ".join(args.query).strip()
    content = asyncio.run(search_and_scrape(query))
    
    if args.json:
        print(json.dumps(parse_research_output(query, content), ensure_ascii=False))
    elif not content:
        print("No usable web results found.")
    else:
        print(content)

if __name__ == "__main__":
    main()
