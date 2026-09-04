"""Poll a configured Restream-compatible JSON endpoint once."""
import json
import hashlib
import os
import sys
import tempfile
import urllib.request

url = os.environ.get("TELEDRA_STREAM_CHAT_URL", "").strip()
if not url:
    print(json.dumps({"ok": False, "capability": "stream_chat", "message": "TELEDRA_STREAM_CHAT_URL is not configured", "data": {}}))
    sys.exit(2)
try:
    with urllib.request.urlopen(url, timeout=10) as response:
        raw = response.read(512 * 1024 + 1)
    if len(raw) > 512 * 1024:
        raise ValueError("stream chat response exceeded 512 KiB")
    payload = json.loads(raw)
    messages = payload if isinstance(payload, list) else payload.get("messages", [])
    messages = [
        message if not isinstance(message, str) else message[:2000]
        for message in messages[-50:]
    ]
    signature = hashlib.sha256(json.dumps(messages, sort_keys=True).encode("utf-8")).hexdigest()
    state_path = os.path.join(tempfile.gettempdir(), "teledra-v2-stream-chat.sha256")
    previous = ""
    try:
        with open(state_path, "r", encoding="ascii") as handle:
            previous = handle.read().strip()
    except OSError:
        pass
    if signature == previous:
        messages = []
    else:
        with open(state_path, "w", encoding="ascii") as handle:
            handle.write(signature)
    print(json.dumps({"ok": True, "capability": "stream_chat", "message": f"Received {len(messages)} chat message(s)", "data": {"messages": messages}}))
except Exception as error:
    print(json.dumps({"ok": False, "capability": "stream_chat", "message": str(error), "data": {}}))
    sys.exit(2)
