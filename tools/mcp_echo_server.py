"""Minimal local MCP server (stdio, newline-delimited JSON-RPC) used to verify
the Teledra MCP bridge handshake without any network/npx dependency. Exposes one
tool, `echo`, that returns its `text` argument. Not part of the runtime."""

import json
import sys


def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except Exception:
            continue
        method = req.get("method")
        rid = req.get("id")
        if method == "initialize":
            send({"jsonrpc": "2.0", "id": rid, "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "echo", "version": "1.0"},
            }})
        elif method == "notifications/initialized":
            continue
        elif method == "tools/list":
            send({"jsonrpc": "2.0", "id": rid, "result": {"tools": [
                {"name": "echo", "description": "Echo back the provided text.",
                 "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}}
            ]}})
        elif method == "tools/call":
            params = req.get("params", {})
            text = (params.get("arguments") or {}).get("text", "")
            send({"jsonrpc": "2.0", "id": rid, "result": {
                "content": [{"type": "text", "text": f"echo: {text}"}], "isError": False,
            }})
        elif rid is not None:
            send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "method not found"}})


if __name__ == "__main__":
    main()
