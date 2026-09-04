"""Run one approved MCP stdio request with a complete bounded handshake."""

import json
import os
import queue
import subprocess
import sys
import threading
import time

PROTOCOL_VERSION = "2024-11-05"
MAX_RESULT_BYTES = 512 * 1024


def configured_path():
    configured = os.environ.get("TELEDRA_MCP_CONFIG", "").strip()
    if configured:
        return configured
    data_root = os.environ.get("TELEDRA_DATA_DIR", "").strip()
    current = os.path.join(data_root, "mcp", "servers.json") if data_root else ""
    workspace = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    legacy = os.path.join(os.path.dirname(workspace), "Teledra", "config", "mcp_servers.json")
    if current and os.path.isfile(current):
        return current
    return legacy if os.path.isfile(legacy) else current


def load_servers(config_path):
    with open(config_path, "r", encoding="utf-8") as handle:
        return json.load(handle).get("servers", {})


def fail(message):
    print(json.dumps({"ok": False, "capability": "mcp", "message": str(message)[:1000], "data": {}}))
    raise SystemExit(2)


def load_spec(config_path, name):
    servers = load_servers(config_path)
    if isinstance(servers, list):
        spec = next((item for item in servers if item.get("name") == name), None)
    else:
        spec = servers.get(name)
    if not isinstance(spec, dict) or spec.get("enabled") is not True:
        raise ValueError(f"unknown or disabled MCP server {name}")
    if not spec.get("command"):
        raise ValueError(f"MCP server {name} has no command")
    return spec


class Session:
    def __init__(self, spec):
        environment = dict(os.environ)
        environment.update(spec.get("env", {}) or {})
        creation_flags = 0x08000000 if os.name == "nt" else 0
        self.process = subprocess.Popen(
            [spec["command"], *list(spec.get("args", []) or [])],
            cwd=spec.get("cwd") or None,
            env=environment,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
            creationflags=creation_flags,
        )
        self.lines = queue.Queue()
        self.next_id = 0
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        try:
            for line in self.process.stdout:
                if line.strip():
                    self.lines.put(line.strip())
        finally:
            self.lines.put(None)

    def send(self, message):
        self.process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        self.process.stdin.flush()

    def request(self, method, parameters=None, timeout=25):
        self.next_id += 1
        request_id = self.next_id
        message = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if parameters is not None:
            message["params"] = parameters
        self.send(message)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                line = self.lines.get(timeout=max(0.05, deadline - time.monotonic()))
            except queue.Empty:
                break
            if line is None:
                break
            try:
                response = json.loads(line)
            except json.JSONDecodeError:
                continue
            if response.get("id") != request_id:
                continue
            if "error" in response:
                raise RuntimeError(response["error"])
            return response.get("result", {})
        raise TimeoutError(f"timeout waiting for {method}")

    def initialize(self):
        self.request(
            "initialize",
            {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "teledra-v2", "version": "0.1.0"},
            },
        )
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def close(self):
        try:
            self.process.terminate()
            self.process.wait(timeout=2)
        except Exception:
            try:
                self.process.kill()
                self.process.wait(timeout=2)
            except Exception:
                pass


session = None
try:
    request = json.load(sys.stdin)
    server = str(request.pop("server", "")).strip()
    config_path = configured_path()
    if request.get("action") == "list_servers":
        if not config_path:
            raise ValueError("no MCP server configuration was found")
        servers = load_servers(config_path)
        if isinstance(servers, list):
            names = [item.get("name") for item in servers if item.get("enabled") is True]
        else:
            names = [name for name, spec in servers.items() if isinstance(spec, dict) and spec.get("enabled") is True]
        print(json.dumps({"ok": True, "capability": "mcp", "message": f"{len(names)} MCP server(s) enabled", "data": {"servers": names}}, ensure_ascii=False))
        raise SystemExit(0)
    if not server or not config_path:
        raise ValueError("server or MCP configuration missing")
    method = str(request.get("method", "tools/list"))
    parameters = request.get("params")
    spec = load_spec(config_path, server)
    session = Session(spec)
    session.initialize()
    result = session.request(method, parameters)
    encoded = json.dumps(result, ensure_ascii=False)
    if len(encoded.encode("utf-8")) > MAX_RESULT_BYTES:
        raise ValueError("MCP result exceeded the 512 KiB safety limit")
    print(json.dumps({"ok": True, "capability": "mcp", "message": f"MCP {method} completed", "data": result}, ensure_ascii=False))
except Exception as error:
    fail(error)
finally:
    if session is not None:
        session.close()
