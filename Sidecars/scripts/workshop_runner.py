"""Execute one workshop experiment with audit-hook capability barriers."""

from __future__ import annotations

import os
import py_compile
import runpy
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) < 2:
        raise SystemExit("workshop candidate is required")
    candidate = Path(sys.argv[1]).resolve(strict=True)
    root = candidate.parent.resolve(strict=True)
    if candidate.parent != root or candidate.suffix.lower() != ".py":
        raise SystemExit("candidate must be one local Python file")

    def audit(event: str, args: tuple[object, ...]) -> None:
        if event.startswith(("socket.", "subprocess.", "ctypes.", "winreg.")):
            raise PermissionError(f"workshop capability denied: {event}")
        if event in {"os.system", "os.remove", "os.rmdir", "os.rename", "os.replace"}:
            raise PermissionError(f"workshop capability denied: {event}")
        if event == "open" and args:
            path = args[0]
            mode = str(args[1]) if len(args) > 1 else "r"
            if isinstance(path, (str, bytes, os.PathLike)) and any(flag in mode for flag in "wax+"):
                resolved = Path(path).resolve()
                if resolved != root and root not in resolved.parents:
                    raise PermissionError("workshop writes must remain inside the experiment directory")

    sys.addaudithook(audit)
    os.chdir(root)
    if "--compile-only" in sys.argv[2:]:
        py_compile.compile(str(candidate), doraise=True)
        print("Python syntax and capability scan passed")
    else:
        runpy.run_path(str(candidate), run_name="__main__")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
