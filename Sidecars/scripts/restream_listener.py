"""Persistent Restream websocket listener.

The token is read from stdin so it never appears in a process command line.
Only bounded author/text JSON objects are written to stdout.
"""

from __future__ import annotations

import asyncio
import json
import sys

import aiohttp


async def listen(token: str) -> None:
    endpoint = "wss://backend.chat.restream.io/ws/embed?token=" + token
    timeout = aiohttp.ClientTimeout(total=None, connect=20, sock_read=None)
    while True:
        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                async with session.ws_connect(endpoint, heartbeat=25, max_msg_size=512 * 1024) as websocket:
                    print("Restream connected", file=sys.stderr, flush=True)
                    async for message in websocket:
                        if message.type != aiohttp.WSMsgType.TEXT:
                            if message.type in {aiohttp.WSMsgType.CLOSED, aiohttp.WSMsgType.ERROR}:
                                break
                            continue
                        try:
                            packet = json.loads(message.data)
                        except (ValueError, TypeError):
                            continue
                        if packet.get("action") != "event":
                            continue
                        event = (packet.get("payload") or {}).get("eventPayload") or {}
                        text = str(event.get("text") or "").strip()
                        if not text:
                            continue
                        author = event.get("author") or {}
                        name = str(
                            author.get("displayName")
                            or author.get("name")
                            or author.get("username")
                            or "Anonymous"
                        )
                        print(
                            json.dumps(
                                {
                                    "author": name[:80],
                                    "text": text[:2000],
                                },
                                ensure_ascii=True,
                            ),
                            flush=True,
                        )
        except asyncio.CancelledError:
            raise
        except Exception as error:
            print(f"Restream reconnecting after {type(error).__name__}", file=sys.stderr, flush=True)
        await asyncio.sleep(5)


def main() -> int:
    token = sys.stdin.readline(4096).strip()
    if not token or len(token) > 2048 or any(character.isspace() for character in token):
        print("Restream token is absent or invalid", file=sys.stderr, flush=True)
        return 2
    asyncio.run(listen(token))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
