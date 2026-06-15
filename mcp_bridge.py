"""Teledra MCP bridge — lets the court discover and use approved MCP servers.

MCP (Model Context Protocol) servers are the real capability frontier: file
access, web fetch, search, memory, finance data, etc. This bridge speaks the
stdio JSON-RPC handshake to any server listed (and enabled) in
config/mcp_servers.json, so the kingdom can list a server's tools and call them.
It is OFF by default — nothing launches until the operator enables a server.

Subcommands (the Rust runtime invokes these):
    python mcp_bridge.py list                 # tools across all enabled servers
    python mcp_bridge.py call                 # stdin: {server, tool, arguments}

Every subcommand prints a single JSON line. Nothing here moves money or runs a
server the operator did not approve.

MCP stdio transport = newline-delimited JSON-RPC 2.0 messages.
"""

from __future__ import annotations

import json
import os
import queue
import subprocess
import sys
import threading
import time

ROOT = os.path.abspath(os.path.dirname(__file__))
SERVERS_CFG = os.environ.get(
    "TELEDRA_MCP_CONFIG", os.path.join(ROOT, "config", "mcp_servers.json")
)
PROTOCOL_VERSION = "2024-11-05"


def _load_servers() -> list:
    try:
        with open(SERVERS_CFG, "r", encoding="utf-8") as handle:
            data = json.load(handle)
    except Exception:
        return []
    out = []
    for s in data.get("servers", []):
        if isinstance(s, dict) and s.get("enabled") and s.get("command"):
            out.append(s)
    return out


class _Server:
    """A single MCP stdio session: spawn, initialize, then list/call."""

    def __init__(self, spec: dict):
        self.name = spec.get("name", "mcp")
        self.proc = None
        self._q: "queue.Queue[str]" = queue.Queue()
        self._next_id = 0
        env = dict(os.environ)
        env.update(spec.get("env", {}) or {})
        cmd = [spec["command"]] + list(spec.get("args", []) or [])
        creationflags = 0x0800_0000 if os.name == "nt" else 0  # CREATE_NO_WINDOW
        self.proc = subprocess.Popen(
            cmd,
            cwd=spec.get("cwd") or ROOT,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env=env,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
            creationflags=creationflags,
        )
        threading.Thread(target=self._reader, daemon=True).start()

    def _reader(self):
        try:
            for line in self.proc.stdout:
                line = line.strip()
                if line:
                    self._q.put(line)
        except Exception:
            pass

    def _send(self, msg: dict):
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()

    def _request(self, method: str, params=None, timeout: float = 25.0):
        self._next_id += 1
        rid = self._next_id
        msg = {"jsonrpc": "2.0", "id": rid, "method": method}
        if params is not None:
            msg["params"] = params
        self._send(msg)
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                line = self._q.get(timeout=max(0.05, deadline - time.time()))
            except queue.Empty:
                break
            try:
                data = json.loads(line)
            except Exception:
                continue
            if data.get("id") == rid:
                if "error" in data:
                    raise RuntimeError(data["error"])
                return data.get("result", {})
            # otherwise it's a notification/log line — keep reading
        raise TimeoutError(f"timeout waiting for {method}")

    def initialize(self):
        self._request(
            "initialize",
            {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "teledra", "version": "1.0"},
            },
        )
        # initialized notification (no id)
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def list_tools(self):
        result = self._request("tools/list")
        tools = []
        for t in result.get("tools", []):
            if isinstance(t, dict):
                tools.append(
                    {"name": t.get("name", ""), "description": (t.get("description") or "")[:160]}
                )
        return tools

    def call_tool(self, tool: str, arguments: dict):
        result = self._request("tools/call", {"name": tool, "arguments": arguments or {}})
        # Flatten text content blocks for a concise reply.
        text_parts = []
        for block in result.get("content", []) if isinstance(result, dict) else []:
            if isinstance(block, dict) and block.get("type") == "text":
                text_parts.append(block.get("text", ""))
        return {
            "is_error": bool(result.get("isError")) if isinstance(result, dict) else False,
            "text": "\n".join(text_parts).strip(),
            "raw": result if not text_parts else None,
        }

    def close(self):
        try:
            if self.proc:
                self.proc.terminate()
        except Exception:
            pass


def cmd_list() -> dict:
    specs = _load_servers()
    if not specs:
        return {"ok": True, "any_enabled": False, "servers": [], "note": "no enabled MCP servers"}
    servers = []
    for spec in specs:
        entry = {"server": spec.get("name", "mcp"), "tools": [], "error": ""}
        srv = None
        try:
            srv = _Server(spec)
            srv.initialize()
            entry["tools"] = srv.list_tools()
        except Exception as exc:
            entry["error"] = f"{type(exc).__name__}: {exc}"[:200]
        finally:
            if srv:
                srv.close()
        servers.append(entry)
    return {"ok": True, "any_enabled": True, "servers": servers}


def cmd_call() -> dict:
    try:
        job = json.loads(sys.stdin.read() or "{}")
    except Exception as exc:
        return {"ok": False, "error": f"bad job json: {exc}"}
    target = job.get("server", "")
    tool = job.get("tool", "")
    args = job.get("arguments", {}) or {}
    if not tool:
        return {"ok": False, "error": "missing tool"}
    specs = _load_servers()
    spec = next(
        (s for s in specs if s.get("name") == target),
        specs[0] if (specs and not target) else None,
    )
    if not spec:
        return {"ok": False, "error": f"no enabled server named '{target}'"}
    srv = None
    try:
        srv = _Server(spec)
        srv.initialize()
        result = srv.call_tool(tool, args)
        return {"ok": not result["is_error"], "server": spec.get("name"), "tool": tool, **result}
    except Exception as exc:
        return {"ok": False, "server": spec.get("name"), "tool": tool, "error": f"{type(exc).__name__}: {exc}"[:300]}
    finally:
        if srv:
            srv.close()


def main() -> int:
    cmd = sys.argv[1] if len(sys.argv) > 1 else "list"
    if cmd == "list":
        out = cmd_list()
    elif cmd == "call":
        out = cmd_call()
    else:
        out = {"ok": False, "error": f"unknown command: {cmd}"}
    print(json.dumps(out, ensure_ascii=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
