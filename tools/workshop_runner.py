from __future__ import annotations

import builtins
import json
import os
from pathlib import Path
import runpy
import shutil
import socket
import subprocess
import sys


ROOT = Path(__file__).resolve().parent
ALLOWED_EXT = {".py", ".json", ".md", ".txt"}


class WorkshopViolation(RuntimeError):
    pass


def resolve_inside_tools(path: object) -> Path:
    candidate = Path(path)
    if candidate.is_absolute():
        resolved = candidate.resolve()
    else:
        resolved = (ROOT / candidate).resolve()
    try:
        resolved.relative_to(ROOT)
    except ValueError as exc:
        raise WorkshopViolation(f"path outside workshop denied: {path}") from exc
    return resolved


def deny_network(*_args, **_kwargs):
    raise WorkshopViolation("network access is disabled in the workshop")


def deny_shell(*_args, **_kwargs):
    raise WorkshopViolation("shell/subprocess access is disabled in the workshop")


def install_guards() -> None:
    original_open = builtins.open

    def guarded_open(file, mode="r", *args, **kwargs):
        # File descriptors (ints) can't be path-resolved; pass them through so
        # stdlib internals (tempfile, gzip-on-fd, etc.) don't crash with TypeError.
        if isinstance(file, int):
            return original_open(file, mode, *args, **kwargs)
        return original_open(resolve_inside_tools(file), mode, *args, **kwargs)

    builtins.open = guarded_open
    socket.socket = deny_network
    socket.create_connection = deny_network
    subprocess.Popen = deny_shell
    subprocess.run = deny_shell
    subprocess.call = deny_shell
    subprocess.check_call = deny_shell
    subprocess.check_output = deny_shell
    os.system = deny_shell
    os.popen = deny_shell

    original_rmtree = shutil.rmtree

    def guarded_rmtree(path, *args, **kwargs):
        target = resolve_inside_tools(path)
        return original_rmtree(target, *args, **kwargs)

    shutil.rmtree = guarded_rmtree

    original_unlink = Path.unlink
    original_open_path = Path.open
    original_read_text = Path.read_text
    original_write_text = Path.write_text
    original_mkdir = Path.mkdir

    def guarded_path_open(self, *args, **kwargs):
        return original_open_path(resolve_inside_tools(self), *args, **kwargs)

    def guarded_read_text(self, *args, **kwargs):
        return original_read_text(resolve_inside_tools(self), *args, **kwargs)

    def guarded_write_text(self, *args, **kwargs):
        return original_write_text(resolve_inside_tools(self), *args, **kwargs)

    def guarded_mkdir(self, *args, **kwargs):
        return original_mkdir(resolve_inside_tools(self), *args, **kwargs)

    def guarded_unlink(self, *args, **kwargs):
        return original_unlink(resolve_inside_tools(self), *args, **kwargs)

    Path.open = guarded_path_open
    Path.read_text = guarded_read_text
    Path.write_text = guarded_write_text
    Path.mkdir = guarded_mkdir
    Path.unlink = guarded_unlink


def static_scan(path: Path) -> None:
    text = path.read_text(encoding="utf-8", errors="replace") if path.suffix == ".py" else ""
    lower = text.lower()
    forbidden = [
        "import socket",
        "from socket",
        "import requests",
        "from requests",
        "import urllib",
        "from urllib",
        "import httpx",
        "from httpx",
        "import subprocess",
        "from subprocess",
        "os.system",
        "popen(",
        "../",
        "..\\",
    ]
    for needle in forbidden:
        if needle in lower:
            raise WorkshopViolation(f"forbidden capability in source: {needle}")


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: python workshop_runner.py experiments/<file>", file=sys.stderr)
        return 2

    target = resolve_inside_tools(sys.argv[1])
    if target.suffix not in ALLOWED_EXT:
        print("denied: unsupported extension", file=sys.stderr)
        return 2
    if not target.exists() or not target.is_file():
        print(f"denied: missing experiment file {target}", file=sys.stderr)
        return 2

    static_scan(target)
    os.chdir(ROOT)
    sys.path = [str(ROOT), str(ROOT / "experiments")] + [
        p for p in sys.path if "site-packages" in p or "python" in p.lower()
    ]
    install_guards()

    if target.suffix == ".py":
        runpy.run_path(str(target), run_name="__main__")
        print(f"python experiment completed: {target.name}")
    elif target.suffix == ".json":
        with open(target, "r", encoding="utf-8") as handle:
            json.load(handle)
        print(f"json experiment parsed: {target.name}")
    else:
        text = target.read_text(encoding="utf-8", errors="replace")
        print(f"text experiment readable: {target.name} ({len(text)} chars)")

    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except WorkshopViolation as exc:
        print(f"workshop violation: {exc}", file=sys.stderr)
        raise SystemExit(3)
