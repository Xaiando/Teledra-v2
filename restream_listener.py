import sys
import json
import asyncio
import aiohttp

async def main():
    # The token arrives on stdin, never as an argument: a command line is
    # visible to process listings, crash diagnostics, and parent-process logs
    # for the whole life of the listener.
    token = sys.stdin.readline().strip()
    if not token:
        print(json.dumps({"error": "No token provided"}), file=sys.stderr, flush=True)
        return 1

    url = f"wss://backend.chat.restream.io/ws/embed?token={token}"

    # Never echo any part of the token, not even a prefix.
    print("Connecting to Restream chat server...", file=sys.stderr, flush=True)
    
    while True:
        try:
            async with aiohttp.ClientSession() as session:
                async with session.ws_connect(url) as ws:
                    print("Connected to Restream chat server!", file=sys.stderr, flush=True)
                    async for msg in ws:
                        if msg.type == aiohttp.WSMsgType.TEXT:
                            try:
                                data = json.loads(msg.data)
                                action = data.get("action")
                                if action == "event":
                                    payload = data.get("payload", {})
                                    event_payload = payload.get("eventPayload", {})
                                    
                                    text = event_payload.get("text")
                                    if not text:
                                        continue
                                        
                                    author_info = event_payload.get("author", {})
                                    author_name = (
                                        author_info.get("displayName") or 
                                        author_info.get("name") or 
                                        author_info.get("username") or 
                                        "Anonymous"
                                    )
                                    
                                    out = {
                                        "author": author_name,
                                        "text": text
                                    }
                                    print(json.dumps(out), flush=True)
                            except Exception as e:
                                print(f"Error parsing websocket message: {e}", file=sys.stderr, flush=True)
                        elif msg.type == aiohttp.WSMsgType.CLOSED:
                            print("WebSocket connection closed by remote host", file=sys.stderr, flush=True)
                            break
                        elif msg.type == aiohttp.WSMsgType.ERROR:
                            print("WebSocket connection error occurred", file=sys.stderr, flush=True)
                            break
        except Exception as e:
            print(f"Failed to connect or establish connection: {e}", file=sys.stderr, flush=True)
            
        print("Attempting reconnection in 5 seconds...", file=sys.stderr, flush=True)
        await asyncio.sleep(5)

if __name__ == "__main__":
    try:
        sys.exit(asyncio.run(main()) or 0)
    except KeyboardInterrupt:
        sys.exit(0)
