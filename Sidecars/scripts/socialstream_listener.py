"""Persistent Social Stream Ninja chat listener.

Social Stream Ninja gathers chat from Twitch, YouTube, Kick and the rest into
one session and, when "Send chat messages to API server" is enabled, relays it
over its API server on channel 4. This joins that channel and writes one
bounded JSON object per chat message to stdout.

The session ID is read from stdin so it never appears in a process command
line: anyone holding it can read the chat and, with remote control enabled,
send commands to the extension. Status lines go to stderr and never repeat it.
"""

from __future__ import annotations

import asyncio
import html
import json
import re
import sys
import time

import aiohttp

ENDPOINT = "wss://io.socialstream.ninja/join/{session}/4"
SESSION_PATTERN = re.compile(r"^[A-Za-z0-9_-]{4,200}$")
TAG = re.compile(r"<[^>]*>")
SPACE = re.compile(r"\s+")


def plain(value) -> str:
    """Chat text without markup. SSN may send sanitized HTML (emotes, links)."""
    text = TAG.sub(" ", str(value or ""))
    return SPACE.sub(" ", html.unescape(text)).strip()


def message(packet: dict) -> dict | None:
    """The court's view of one SSN packet, or None when it is not chat."""
    name = plain(packet.get("chatname"))
    if not name:
        return None
    text = plain(packet.get("chatmessage"))
    donation = plain(packet.get("hasDonation"))
    event = plain(packet.get("event"))
    if event and not text and not donation:
        return None  # follows, joins and the like carry no words to answer
    if donation:
        text = f"[donation: {donation}] {text}".strip()
    if not text:
        return None
    return {
        "author": name[:80],
        "platform": plain(packet.get("type"))[:30],
        "text": text[:2000],
    }


async def listen(session: str) -> None:
    endpoint = ENDPOINT.format(session=session)
    timeout = aiohttp.ClientTimeout(total=None, connect=20, sock_read=None)
    recent: dict[tuple[str, str], float] = {}
    delay = 5
    while True:
        try:
            async with aiohttp.ClientSession(timeout=timeout) as client:
                async with client.ws_connect(endpoint, heartbeat=25, max_msg_size=512 * 1024) as websocket:
                    delay = 5
                    print(
                        "Social Stream connected; waiting for chat (needs 'Send chat messages to API server' on in SSN)",
                        file=sys.stderr,
                        flush=True,
                    )
                    async for frame in websocket:
                        if frame.type != aiohttp.WSMsgType.TEXT:
                            if frame.type in {aiohttp.WSMsgType.CLOSED, aiohttp.WSMsgType.ERROR}:
                                break
                            continue
                        try:
                            packet = json.loads(frame.data)
                        except (ValueError, TypeError):
                            continue
                        if not isinstance(packet, dict):
                            continue
                        chat = message(packet)
                        if chat is None:
                            continue
                        # The relay can deliver one message more than once.
                        now = time.monotonic()
                        key = (chat["author"], chat["text"])
                        if now - recent.get(key, -1e9) < 5:
                            continue
                        recent = {k: t for k, t in recent.items() if now - t < 60}
                        recent[key] = now
                        print(json.dumps(chat, ensure_ascii=True), flush=True)
            print("Social Stream disconnected; reconnecting", file=sys.stderr, flush=True)
        except asyncio.CancelledError:
            raise
        except Exception as error:
            print(f"Social Stream reconnecting after {type(error).__name__}", file=sys.stderr, flush=True)
        await asyncio.sleep(delay)
        delay = min(delay * 2, 60)


def main() -> int:
    session = sys.stdin.readline(4096).strip()
    if not SESSION_PATTERN.match(session):
        print("Social Stream session ID is absent or invalid", file=sys.stderr, flush=True)
        return 2
    asyncio.run(listen(session))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
