from __future__ import annotations

import os
import runpy
import sys
from pathlib import Path


def _roots() -> list[Path]:
    raw = os.environ.get("KRAKEN_WRITE_ROOTS", "")
    return [Path(item).resolve() for item in raw.split(os.pathsep) if item]


ALLOWED_ROOTS = _roots()


def _inside_allowed(path: object) -> bool:
    try:
        candidate = Path(path).resolve()
    except Exception:
        return True
    for root in ALLOWED_ROOTS:
        try:
            candidate.relative_to(root)
            return True
        except ValueError:
            pass
    return False


def _write_intent(mode: object = None, flags: object = None) -> bool:
    if isinstance(mode, str) and any(ch in mode for ch in "wax+"):
        return True
    if isinstance(flags, int):
        return bool(flags & (os.O_WRONLY | os.O_RDWR | os.O_APPEND | os.O_CREAT | os.O_TRUNC))
    return False


def _guard(event: str, args: tuple) -> None:
    if event == "open" and args:
        mode = args[1] if len(args) > 1 else None
        flags = args[2] if len(args) > 2 else None
        if _write_intent(mode, flags) and not _inside_allowed(args[0]):
            raise PermissionError(f"write outside kraken root/workspace blocked: {args[0]}")
    elif event in {"os.remove", "os.rmdir", "os.mkdir", "os.rename"} and args:
        for candidate in args[:2]:
            if not _inside_allowed(candidate):
                raise PermissionError(f"{event} outside kraken root/workspace blocked: {candidate}")
    elif event in {"subprocess.Popen", "socket.connect"}:
        raise PermissionError(f"{event} blocked during verify_code tests")


sys.addaudithook(_guard)
runpy.run_path(os.environ["KRAKEN_TEST_PATH"], run_name="__main__")
