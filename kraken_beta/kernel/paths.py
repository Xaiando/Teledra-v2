"""Path containment helpers — keep skills inside their designated zones."""

from __future__ import annotations

import os

# Folders prod_digest may read (operator productivity surfaces only).
DIGEST_EXTRA = (
    r"D:\Teledra\logs",
    r"D:\Teledra\reflections",
)


def _norm(path: str) -> str:
    return os.path.normcase(os.path.normpath(os.path.abspath(path)))


def is_under(path: str, root: str) -> bool:
    path_n = _norm(path)
    root_n = _norm(root)
    return path_n == root_n or path_n.startswith(root_n + os.sep)


def digest_allowed(folder: str, ctx: dict) -> tuple[str | None, str | None]:
    """Resolve folder for prod_digest. Returns (resolved_path, error_reason)."""
    raw = folder.strip()
    root = ctx["root"]
    workspace = ctx.get("workspace") or os.path.join(root, "workspace")

    if os.path.isabs(raw):
        candidate = os.path.abspath(raw)
    else:
        candidate = os.path.abspath(os.path.join(root, raw))

    candidate = os.path.normpath(candidate)
    allowed_roots = [_norm(root), _norm(workspace)]
    for extra in DIGEST_EXTRA:
        if os.path.isdir(extra):
            allowed_roots.append(_norm(extra))

    if not any(is_under(candidate, ar) for ar in allowed_roots):
        return None, f"folder outside digest allowlist: {raw}"

    if not os.path.isdir(candidate):
        return None, f"not a directory: {candidate}"

    return candidate, None