"""Fetch bounded YouTube evidence, with an explicit title-only fallback."""

from __future__ import annotations

import html
import json
import re
import sys
import time
from urllib.parse import parse_qs, urlencode, urlparse
from urllib.request import Request, urlopen

MAX_TRANSCRIPT_CHARS = 6_000
NETWORK_BUDGET_SECONDS = 22


def video_id(url):
    value = html.unescape(str(url)).strip().strip("<>\"'").rstrip(".,;!?)\"]}")
    if value.startswith("//"):
        value = "https:" + value
    elif "://" not in value:
        value = "https://" + value
    try:
        parsed = urlparse(value)
        host = (parsed.hostname or "").lower().rstrip(".")
        if parsed.scheme not in ("http", "https") or parsed.username or parsed.password:
            return ""
        if host in ("youtu.be", "www.youtu.be"):
            candidate = parsed.path.strip("/").split("/", 1)[0]
        elif host in ("youtube.com", "youtube-nocookie.com") or host.endswith(
            (".youtube.com", ".youtube-nocookie.com")
        ):
            path = parsed.path.rstrip("/")
            if path == "/watch":
                candidate = parse_qs(parsed.query).get("v", [""])[0]
            elif path.startswith(("/shorts/", "/live/", "/embed/", "/v/")):
                candidate = path.split("/")[2]
            elif path == "/attribution_link":
                target = parse_qs(parsed.query).get("u", [""])[0]
                # Only a relative watch target; never follow arbitrary redirects.
                return video_id("https://www.youtube.com" + target) if target.startswith("/watch?") else ""
            else:
                candidate = ""
        else:
            candidate = ""
    except ValueError:
        return ""
    return candidate if re.fullmatch(r"[A-Za-z0-9_-]{11}", candidate) else ""


def fetch_metadata(url):
    endpoint = "https://www.youtube.com/oembed?" + urlencode({"url": url, "format": "json"})
    request = Request(endpoint, headers={"User-Agent": "Teledra/2"})
    try:
        with urlopen(request, timeout=4) as response:
            data = json.loads(response.read(65_536))
        return {
            "title": str(data.get("title", ""))[:500],
            "author": str(data.get("author_name", ""))[:200],
        }
    except Exception:
        # Captions can still succeed if oEmbed is unavailable.
        return {}


def choose_transcript(transcripts, languages):
    available = list(transcripts)
    if not available:
        raise RuntimeError("This video exposes no captions or transcript.")

    def rank(transcript):
        code = getattr(transcript, "language_code", "").lower()
        for index, language in enumerate(languages):
            if code == language.lower() or code.split("-")[0] == language.lower().split("-")[0]:
                return index, bool(getattr(transcript, "is_generated", False))
        return len(languages), bool(getattr(transcript, "is_generated", False))

    # Accept the video's original language when English captions do not exist.
    return min(available, key=rank)


def fetch_transcript(identifier, languages, deadline):
    from requests import Session
    from youtube_transcript_api import YouTubeTranscriptApi

    class BoundedSession(Session):
        def request(self, method, url, **kwargs):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("YouTube transcript retrieval timed out.")
            kwargs["timeout"] = min(6, remaining)
            return super().request(method, url, **kwargs)

    with BoundedSession() as session:
        if hasattr(YouTubeTranscriptApi, "list"):
            transcripts = YouTubeTranscriptApi(http_client=session).list(identifier)
        else:
            # Older optional installs used static methods and returned dict snippets.
            transcripts = YouTubeTranscriptApi.list_transcripts(identifier)
        selected = choose_transcript(transcripts, languages)
        return selected.fetch(), {
            "language": getattr(selected, "language", ""),
            "language_code": getattr(selected, "language_code", ""),
            "is_generated": bool(getattr(selected, "is_generated", False)),
        }


def transcript_excerpt(transcript):
    snippets = [item.get("text", "") if isinstance(item, dict) else getattr(item, "text", "") for item in transcript]
    text = " ".join(" ".join(html.unescape(str(snippet)).split()) for snippet in snippets).strip()
    if not text:
        raise RuntimeError("This video exposes an empty transcript.")
    if len(text) <= MAX_TRANSCRIPT_CHARS:
        return text, len(text), False
    # Long podcasts need evidence from throughout the recording, not only its intro.
    separator = "\n[... transcript excerpt gap ...]\n"
    window = (MAX_TRANSCRIPT_CHARS - len(separator) * 3) // 4
    starts = [round(index * (len(text) - window) / 3) for index in range(4)]
    excerpts = []
    for start in starts:
        end = start + window
        if start:
            boundary = text.find(" ", start, min(start + 100, end))
            start = boundary + 1 if boundary != -1 else start
        if end < len(text):
            boundary = text.rfind(" ", max(start, end - 100), end)
            end = boundary if boundary != -1 else end
        excerpts.append(text[start:end].strip())
    return separator.join(excerpts), len(text), True


def failure_message(error):
    names = {cls.__name__ for cls in type(error).__mro__}
    if "ModuleNotFoundError" in names or "ImportError" in names:
        return "YouTube captions support is missing from the configured Python; install Sidecars/requirements-optional.txt with TELEDRA_PYTHON."
    if names & {"RequestBlocked", "IpBlocked", "PoTokenRequired"}:
        return "YouTube blocked caption retrieval for this connection. Only available video metadata can be used."
    if names & {"TranscriptsDisabled", "NoTranscriptFound"}:
        return "This video has no accessible captions or transcript."
    if names & {"VideoUnavailable", "VideoUnplayable", "AgeRestricted"}:
        return "This video is unavailable, private, restricted, or not playable from this connection."
    if names & {"Timeout", "TimeoutError", "ConnectTimeout", "ReadTimeout"}:
        return "YouTube caption retrieval timed out."
    return " ".join(str(error).split())[:500] or type(error).__name__


def handle_request(request):
    if not isinstance(request, dict):
        raise ValueError("The YouTube request must be a JSON object.")
    identifier = video_id(request.get("url", ""))
    if not identifier:
        raise ValueError("A valid youtube.com or youtu.be video URL is required.")
    url = "https://www.youtube.com/watch?v=" + identifier
    deadline = time.monotonic() + NETWORK_BUDGET_SECONDS
    data = {"url": url, "video_id": identifier, **fetch_metadata(url)}
    languages = request.get("languages", ["en"])
    if not isinstance(languages, list) or not languages or not all(isinstance(item, str) for item in languages):
        languages = ["en"]
    try:
        transcript, details = fetch_transcript(identifier, languages, deadline)
        text, total, truncated = transcript_excerpt(transcript)
        data.update(details)
        data.update({
            "transcript": text,
            "content_status": "transcript",
            "transcript_characters": total,
            "truncated": truncated,
            "coverage": "excerpts spread across the transcript" if truncated else "complete available transcript",
        })
        message = f"Fetched {len(text)} transcript characters"
        if truncated:
            message += f" in excerpts from {total} characters"
    except Exception as error:
        reason = failure_message(error)
        if not data.get("title"):
            return {"ok": False, "capability": "youtube", "message": reason, "data": data}
        data.update({
            "transcript": "",
            "content_status": "metadata_only",
            "transcript_error": reason,
            "evidence_limit": "Only the title and channel are available. Do not claim to have watched, heard, or summarized the video's contents.",
        })
        message = f"Video identified; transcript unavailable. {reason}"
    return {"ok": True, "capability": "youtube", "message": message, "data": data}


def main():
    try:
        raw = sys.stdin.read(65_536)
        response = handle_request(json.loads(raw) if raw.strip() else {})
    except Exception as error:
        response = {"ok": False, "capability": "youtube", "message": failure_message(error), "data": {}}
    # ASCII JSON survives Windows' legacy stdout encodings while decoding to Unicode.
    print(json.dumps(response, ensure_ascii=True))
    return 0 if response["ok"] else 2


if __name__ == "__main__":
    sys.exit(main())
