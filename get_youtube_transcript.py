from __future__ import annotations

import re
import sys
from urllib.parse import parse_qs, urlparse


def video_id_from_url(value: str) -> str:
    value = value.strip()
    if re.fullmatch(r"[A-Za-z0-9_-]{11}", value):
        return value
    parsed = urlparse(value)
    host = parsed.netloc.lower().split(":")[0]
    if host in {"youtu.be", "www.youtu.be"}:
        candidate = parsed.path.strip("/").split("/")[0]
    elif host.endswith("youtube.com"):
        if parsed.path == "/watch":
            candidate = parse_qs(parsed.query).get("v", [""])[0]
        elif parsed.path.startswith(("/shorts/", "/live/", "/embed/")):
            parts = [part for part in parsed.path.split("/") if part]
            candidate = parts[1] if len(parts) > 1 else ""
        else:
            candidate = ""
    else:
        candidate = ""
    if not re.fullmatch(r"[A-Za-z0-9_-]{11}", candidate):
        raise ValueError("Could not extract a YouTube video id from the supplied URL.")
    return candidate


def fetch_snippets(video_id: str):
    from youtube_transcript_api import YouTubeTranscriptApi

    api = YouTubeTranscriptApi()
    try:
        transcript = api.fetch(video_id, languages=["en"])
        return list(transcript)
    except Exception:
        # Let the library choose an available transcript/language rather than
        # failing a multilingual source simply because English is absent.
        transcript = api.fetch(video_id)
        return list(transcript)


def field(item, name: str, default=None):
    if hasattr(item, name):
        return getattr(item, name)
    if isinstance(item, dict):
        return item.get(name, default)
    return default


def stamp(seconds: float) -> str:
    total = max(0, int(seconds))
    hours, rem = divmod(total, 3600)
    minutes, secs = divmod(rem, 60)
    if hours:
        return f"{hours:02d}:{minutes:02d}:{secs:02d}"
    return f"{minutes:02d}:{secs:02d}"


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: get_youtube_transcript.py <youtube-url-or-id>", file=sys.stderr)
        return 2
    try:
        video_id = video_id_from_url(sys.argv[1])
        snippets = fetch_snippets(video_id)
    except Exception as exc:
        print(f"YouTube transcript unavailable: {exc}", file=sys.stderr)
        return 1

    emitted = 0
    for item in snippets:
        text = str(field(item, "text", "") or "").replace("\n", " ").strip()
        start = float(field(item, "start", 0.0) or 0.0)
        if not text:
            continue
        print(f"[{stamp(start)}] {text}")
        emitted += 1
    if not emitted:
        print("YouTube transcript unavailable: transcript was empty.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
