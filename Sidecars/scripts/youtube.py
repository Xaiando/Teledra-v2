"""Fetch a bounded YouTube transcript through an optional local dependency."""

import json
import re
import sys
from urllib.parse import parse_qs, urlparse


def read_request():
    raw = sys.stdin.read(65536)
    return json.loads(raw) if raw.strip() else {}


def video_id(url):
    parsed = urlparse(url)
    host = (parsed.hostname or "").lower()
    if host == "youtu.be":
        candidate = parsed.path.strip("/").split("/", 1)[0]
    elif host.endswith("youtube.com"):
        if parsed.path == "/watch":
            candidate = parse_qs(parsed.query).get("v", [""])[0]
        elif parsed.path.startswith(("/shorts/", "/live/", "/embed/")):
            candidate = parsed.path.split("/")[2]
        else:
            candidate = ""
    else:
        candidate = ""
    return candidate if re.fullmatch(r"[A-Za-z0-9_-]{6,20}", candidate) else ""


try:
    request = read_request()
    url = str(request.get("url", "")).strip()
    identifier = video_id(url)
    if not identifier:
        raise ValueError("a valid youtube.com or youtu.be video URL is required")

    from youtube_transcript_api import YouTubeTranscriptApi

    try:
        transcript = YouTubeTranscriptApi().fetch(identifier)
        snippets = [getattr(item, "text", str(item)) for item in transcript]
    except AttributeError:
        # Compatibility with releases exposing the older static API.
        transcript = YouTubeTranscriptApi.get_transcript(identifier)
        snippets = [str(item.get("text", "")) for item in transcript]
    text = " ".join(" ".join(snippet.split()) for snippet in snippets).strip()
    if not text:
        raise RuntimeError("the video exposes no transcript")
    text = text[:16000]
    response = {
        "ok": True,
        "capability": "youtube",
        "message": f"Fetched {len(text)} transcript characters",
        "data": {"url": url, "video_id": identifier, "transcript": text},
    }
except Exception as error:
    response = {
        "ok": False,
        "capability": "youtube",
        "message": str(error),
        "data": {},
    }

print(json.dumps(response, ensure_ascii=False))
sys.exit(0 if response["ok"] else 2)
