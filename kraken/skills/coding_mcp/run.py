"""coding_mcp - safe local coding tools for Kraken."""

from __future__ import annotations

import fnmatch
import json
import os
import py_compile
import re
import subprocess
import sys
from pathlib import Path


MAX_READ_BYTES = 80_000
MAX_MATCHES = 120
MAX_PATTERN_LEN = 200
SKIP_DIRS = {".git", "__pycache__", "node_modules", "target", ".venv", "venv"}
REDOS_MARKERS = ("++", "**", "??", "{,")


def _inside(parent: str, child: str) -> bool:
    try:
        Path(child).resolve().relative_to(Path(parent).resolve())
        return True
    except ValueError:
        return False


def _resolve(ctx: dict, raw: str | None) -> str:
    root = os.path.abspath(ctx["root"])
    workspace = os.path.abspath(ctx.get("workspace") or os.path.join(root, "workspace"))
    raw = (raw or ".").strip() or "."
    if os.path.isabs(raw):
        path = os.path.abspath(raw)
    else:
        path = os.path.abspath(os.path.join(workspace, raw))
    if not (_inside(root, path) or _inside(workspace, path)):
        raise ValueError(f"path outside kraken root/workspace: {raw}")
    return path


def _rel(ctx: dict, path: str) -> str:
    root = os.path.abspath(ctx["root"])
    try:
        return os.path.relpath(path, root)
    except ValueError:
        return path


def _parse(raw: str) -> dict:
    raw = raw.strip().lstrip("\ufeff")
    if raw.startswith("{"):
        try:
            return json.loads(raw)
        except json.JSONDecodeError:
            return _parse_loose_object(raw)
    lower = raw.lower()
    if "test" in lower:
        return {"op": "run_tests", "path": "."}
    if "compile" in lower:
        return {"op": "py_compile", "path": "."}
    if "git" in lower and "status" in lower:
        return {"op": "git_status", "path": "."}
    if "search" in lower:
        return {"op": "search", "path": ".", "pattern": raw.split("search", 1)[-1].strip() or "TODO"}
    return {"op": "tree", "path": ".", "max_files": 80}


def _parse_loose_object(raw: str) -> dict:
    """Accept PowerShell-mangled JSON-ish input like {op:tree,path:.}."""
    inner = raw.strip()[1:-1].strip()
    data: dict[str, str | int] = {}
    for part in inner.split(","):
        if ":" not in part:
            continue
        key, value = part.split(":", 1)
        key = key.strip().strip("'\"")
        value = value.strip().strip("'\"")
        if value.isdigit():
            data[key] = int(value)
        else:
            data[key] = value
    return data


def _walk_files(root: str, max_files: int = 200) -> list[str]:
    files: list[str] = []
    for base, dirs, names in os.walk(root):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        for name in sorted(names):
            files.append(os.path.join(base, name))
            if len(files) >= max_files:
                return files
    return files


def _tree(ctx: dict, data: dict) -> tuple[bool, str, dict]:
    path = _resolve(ctx, data.get("path"))
    max_files = int(data.get("max_files", 80))
    if not os.path.isdir(path):
        return False, f"not a directory: {_rel(ctx, path)}", {}
    files = _walk_files(path, max_files=max_files)
    lines = [f"# Tree: {_rel(ctx, path)}", ""]
    for file in files:
        lines.append(f"- {_rel(ctx, file)}")
    if len(files) >= max_files:
        lines.append(f"- ... capped at {max_files} files")
    return True, "\n".join(lines), {"files": [_rel(ctx, f) for f in files]}


def _read(ctx: dict, data: dict) -> tuple[bool, str, dict]:
    path = _resolve(ctx, data.get("path"))
    max_bytes = int(data.get("max_bytes", MAX_READ_BYTES))
    if not os.path.isfile(path):
        return False, f"not a file: {_rel(ctx, path)}", {}
    with open(path, "rb") as fh:
        blob = fh.read(max_bytes + 1)
    truncated = len(blob) > max_bytes
    text = blob[:max_bytes].decode("utf-8", errors="replace")
    report = f"# Read: {_rel(ctx, path)}\n\n```text\n{text}\n```"
    if truncated:
        report += f"\n\n(truncated at {max_bytes} bytes)"
    return True, report, {"path": _rel(ctx, path), "truncated": truncated}


def _safe_regex(pattern: str) -> re.Pattern:
    if len(pattern) > MAX_PATTERN_LEN:
        raise ValueError(f"search pattern too long (max {MAX_PATTERN_LEN})")
    if any(marker in pattern for marker in REDOS_MARKERS):
        raise ValueError("search pattern rejected (nested repetition / ReDoS risk)")
    if re.search(r"\([^)]*[+*][^)]*\)\s*[+*?{]", pattern):
        raise ValueError("search pattern rejected (nested repetition / ReDoS risk)")
    return re.compile(pattern, re.IGNORECASE)


def _search(ctx: dict, data: dict) -> tuple[bool, str, dict]:
    path = _resolve(ctx, data.get("path"))
    pattern = str(data.get("pattern") or "").strip()
    if not pattern:
        return False, "missing search pattern", {}
    regex = _safe_regex(pattern)
    root = path if os.path.isdir(path) else os.path.dirname(path)
    files = _walk_files(root, max_files=1000) if os.path.isdir(path) else [path]
    matches: list[dict] = []
    for file in files:
        try:
            with open(file, "r", encoding="utf-8", errors="replace") as fh:
                for lineno, line in enumerate(fh, 1):
                    if regex.search(line):
                        matches.append({"file": _rel(ctx, file), "line": lineno, "text": line.rstrip()[:240]})
                        if len(matches) >= int(data.get("max_matches", MAX_MATCHES)):
                            raise StopIteration
        except (OSError, UnicodeError):
            continue
        except StopIteration:
            break
    lines = [f"# Search: {pattern}", ""]
    lines.extend(f"- {m['file']}:{m['line']}: {m['text']}" for m in matches)
    if not matches:
        lines.append("(no matches)")
    return True, "\n".join(lines), {"matches": matches}


def _py_compile(ctx: dict, data: dict) -> tuple[bool, str, dict]:
    path = _resolve(ctx, data.get("path"))
    files = [path] if os.path.isfile(path) else [
        f for f in _walk_files(path, max_files=1000) if fnmatch.fnmatch(f, "*.py")
    ]
    failures: list[str] = []
    for file in files:
        try:
            py_compile.compile(file, doraise=True)
        except py_compile.PyCompileError as exc:
            failures.append(f"{_rel(ctx, file)}: {exc.msg}")
    lines = [f"# py_compile: {_rel(ctx, path)}", ""]
    if failures:
        lines.extend(f"- FAIL {item}" for item in failures)
    else:
        lines.append(f"OK ({len(files)} Python file(s))")
    return not failures, "\n".join(lines), {"files": len(files), "failures": failures}


def _write_runner(ctx: dict) -> str:
    path = os.path.join(ctx["workdir"], "coding_mcp_test_runner.py")
    if os.path.exists(path):
        return path
    code = r'''
from __future__ import annotations
import os, runpy, sys
from pathlib import Path

ROOTS = [Path(p).resolve() for p in os.environ.get("KRAKEN_WRITE_ROOTS", "").split(os.pathsep) if p]

def inside(path):
    try:
        p = Path(path).resolve()
    except Exception:
        return True
    for root in ROOTS:
        try:
            p.relative_to(root)
            return True
        except ValueError:
            pass
    return False

def guard(event, args):
    if event == "open" and args:
        mode = args[1] if len(args) > 1 else None
        flags = args[2] if len(args) > 2 else None
        writing = (isinstance(mode, str) and any(ch in mode for ch in "wax+")) or (isinstance(flags, int) and flags & (os.O_WRONLY | os.O_RDWR | os.O_APPEND | os.O_CREAT | os.O_TRUNC))
        if writing and not inside(args[0]):
            raise PermissionError(f"write outside kraken root/workspace blocked: {args[0]}")
    elif event in {"subprocess.Popen", "socket.connect"}:
        raise PermissionError(f"{event} blocked during coding_mcp tests")

sys.addaudithook(guard)
runpy.run_path(os.environ["KRAKEN_TEST_PATH"], run_name="__main__")
'''
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(code.strip() + "\n")
    return path


def _run_tests(ctx: dict, data: dict) -> tuple[bool, str, dict]:
    path = _resolve(ctx, data.get("path"))
    test_files = [path] if os.path.isfile(path) else [
        f for f in _walk_files(path, max_files=1000)
        if os.path.basename(f).startswith("test_") and f.endswith(".py")
    ]
    if not test_files:
        return False, f"# run_tests: {_rel(ctx, path)}\n\nNo test_*.py files found.", {"tests": 0}
    runner = _write_runner(ctx)
    env = dict(os.environ)
    env["PYTHONPATH"] = os.pathsep.join([path if os.path.isdir(path) else os.path.dirname(path), ctx["root"], env.get("PYTHONPATH", "")])
    env["KRAKEN_WRITE_ROOTS"] = os.pathsep.join([str(Path(ctx["root"]).resolve()), str(Path(ctx.get("workspace") or os.path.join(ctx["root"], "workspace")).resolve())])
    failures: list[str] = []
    for test in test_files:
        env["KRAKEN_TEST_PATH"] = test
        proc = subprocess.run([sys.executable, runner], cwd=os.path.dirname(test), env=env, text=True, capture_output=True, timeout=45)
        if proc.returncode != 0:
            failures.append(f"{_rel(ctx, test)}\n{((proc.stdout or '') + (proc.stderr or ''))[-2000:]}")
    lines = [f"# run_tests: {_rel(ctx, path)}", ""]
    if failures:
        lines.extend(f"- FAIL {item}" for item in failures)
    else:
        lines.append(f"OK ({len(test_files)} test file(s))")
    return not failures, "\n".join(lines), {"tests": len(test_files), "failures": failures}


def _git_status(ctx: dict, data: dict) -> tuple[bool, str, dict]:
    path = _resolve(ctx, data.get("path"))
    cwd = path if os.path.isdir(path) else os.path.dirname(path)
    proc = subprocess.run(["git", "status", "--short"], cwd=cwd, text=True, capture_output=True, timeout=20)
    output = (proc.stdout or proc.stderr or "").strip()
    if proc.returncode != 0:
        return False, f"# git status: {_rel(ctx, cwd)}\n\n{output or 'git status failed'}", {}
    return True, f"# git status: {_rel(ctx, cwd)}\n\n```text\n{output or '(clean)'}\n```", {"clean": not bool(output)}


OPS = {
    "tree": _tree,
    "read": _read,
    "search": _search,
    "py_compile": _py_compile,
    "run_tests": _run_tests,
    "git_status": _git_status,
}


def execute(job: dict, ctx: dict) -> dict:
    try:
        data = _parse(job.get("input", ""))
        op = str(data.get("op") or "").strip()
        if op not in OPS:
            return {"ok": False, "notes": f"unknown coding_mcp op: {op}"}
        ok, report, meta = OPS[op](ctx, data)
    except Exception as exc:
        return {"ok": False, "notes": f"coding_mcp failed: {exc}"}

    out = os.path.join(ctx["workdir"], "coding_mcp_report.md")
    with open(out, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(report.rstrip() + "\n")
    json_out = os.path.join(ctx["workdir"], "coding_mcp_result.json")
    with open(json_out, "w", encoding="utf-8", newline="\n") as fh:
        json.dump({"ok": ok, "op": op, "meta": meta}, fh, ensure_ascii=False, indent=2)
    return {
        "ok": ok,
        "output": os.path.relpath(out, ctx["root"]),
        "notes": f"coding_mcp {op} {'passed' if ok else 'failed'}",
    }
