"""prod_digest — digest a folder of logs/notes into a daily brief."""

from __future__ import annotations

import json
import os
import re
from datetime import datetime

from kraken.kernel import paths as path_guard

ELIGIBLE_EXT = {".md", ".txt", ".jsonl", ".json", ".log"}
DEFAULT_MAX_LINES = 60
DEFAULT_MAX_CHARS = 8000
DEFAULT_MAX_FILES = 12
MANIFEST_NAME = "sources_manifest.json"


def _parse_input(raw: str) -> tuple[str, dict]:
    parts = raw.strip().split()
    if not parts:
        raise ValueError("input must be a folder path")
    folder = parts[0]
    opts: dict = {}
    for token in parts[1:]:
        if "=" in token:
            key, _, value = token.partition("=")
            opts[key.strip()] = value.strip()
    return folder, opts


def _list_files(folder: str, max_files: int) -> list[str]:
    found = []
    try:
        names = sorted(os.listdir(folder))
    except OSError as exc:
        raise ValueError(f"cannot list folder: {exc}") from exc
    for name in names:
        path = os.path.join(folder, name)
        if os.path.isfile(path) and os.path.splitext(name)[1].lower() in ELIGIBLE_EXT:
            found.append(path)
    return found[:max_files]


def _read_jsonl_tail(path: str, max_lines: int) -> tuple[str, int]:
    try:
        with open(path, "r", encoding="utf-8", errors="ignore") as fh:
            lines = fh.readlines()
    except OSError as exc:
        return f"(unreadable: {exc})", 0
    tail = lines[-max_lines:] if len(lines) > max_lines else lines
    return "".join(tail), len(tail)


def _read_text_bounded(path: str, max_chars: int) -> tuple[str, int]:
    try:
        with open(path, "r", encoding="utf-8", errors="ignore") as fh:
            text = fh.read(max_chars + 2000)
    except OSError as exc:
        return f"(unreadable: {exc})", 0
    if len(text) > max_chars:
        half = max_chars // 2
        text = text[:half] + "\n\n[... truncated ...]\n\n" + text[-half:]
    return text, min(len(text), max_chars)


def _read_source(path: str, max_lines: int, max_chars: int) -> dict:
    ext = os.path.splitext(path)[1].lower()
    if ext == ".jsonl":
        excerpt, units = _read_jsonl_tail(path, max_lines)
        mode = f"jsonl_tail_{units}_lines"
    else:
        excerpt, units = _read_text_bounded(path, max_chars)
        mode = f"text_{units}_chars"
    return {
        "path": path,
        "mode": mode,
        "excerpt": excerpt,
        "bytes": len(excerpt.encode("utf-8", errors="ignore")),
    }


def execute(job: dict, ctx: dict) -> dict:
    try:
        folder_raw, opts = _parse_input(job["input"])
    except ValueError as exc:
        return {"ok": False, "notes": str(exc)}

    folder, deny = path_guard.digest_allowed(folder_raw, ctx)
    if deny:
        ctx["log"](f"prod_digest denied: {deny}")
        report = (
            f"# Daily Brief — denied\n\n"
            f"**Reason:** {deny}\n\n"
            f"Allowed roots: kraken/, workspace/, D:\\Teledra\\logs, reflections.\n\n"
            f"## Sources\n\n(none)\n"
        )
        out_path = _write(ctx, job, report)
        # write an empty manifest so verify_digest sees an honest denial
        # (zero sources, zero waived) instead of a missing-manifest failure.
        os.makedirs(ctx["workdir"], exist_ok=True)
        with open(os.path.join(ctx["workdir"], MANIFEST_NAME), "w",
                  encoding="utf-8") as fh:
            json.dump({"folder": folder_raw, "max_files": 0, "sources": [],
                       "waived": [], "denied": deny}, fh)
        return {"ok": True, "output": out_path, "notes": f"denied: {deny}"}

    max_lines = int(opts.get("max_lines", DEFAULT_MAX_LINES))
    max_chars = int(opts.get("max_chars", DEFAULT_MAX_CHARS))
    max_files = int(opts.get("max_files", DEFAULT_MAX_FILES))

    try:
        files = _list_files(folder, max_files)
    except ValueError as exc:
        return {"ok": False, "notes": str(exc)}
    ctx["log"](f"prod_digest scanning {folder}: {len(files)} file(s)")

    manifest = {
        "folder": folder,
        "max_files": max_files,
        "max_lines": max_lines,
        "max_chars": max_chars,
        "generated": datetime.now().isoformat(timespec="seconds"),
        "sources": [],
        "waived": [],
    }

    excerpts = []
    for path in files:
        try:
            size = os.path.getsize(path)
        except OSError:
            size = 0
        if size == 0:
            manifest["waived"].append({"path": path, "reason": "empty file"})
            continue

        entry = _read_source(path, max_lines, max_chars)
        if not entry["excerpt"].strip():
            manifest["waived"].append({"path": path, "reason": "no readable content"})
            continue
        manifest["sources"].append({
            "path": path, "mode": entry["mode"], "bytes": entry["bytes"],
        })
        excerpts.append(f"=== {os.path.basename(path)} ({entry['mode']}) ===\n{entry['excerpt']}")

    workdir = ctx["workdir"]
    os.makedirs(workdir, exist_ok=True)
    manifest_path = os.path.join(workdir, MANIFEST_NAME)
    with open(manifest_path, "w", encoding="utf-8") as fh:
        json.dump(manifest, fh, indent=2)

    if not excerpts:
        report = (
            f"# Daily Brief — {os.path.basename(folder)}\n\n"
            f"No digestible content found in `{folder}`.\n\n"
            f"## Sources\n\n(none)\n\n## Waived\n\n"
            + "\n".join(f"- `{w['path']}` — {w['reason']}" for w in manifest["waived"])
            + "\n"
        )
        out_path = _write(ctx, job, report)
        return {"ok": True, "output": out_path, "notes": "empty folder digest"}

    llm = ctx["llm"]
    prompt = (
        "Write a concise daily productivity brief from these source excerpts.\n"
        "Use markdown with these sections exactly:\n"
        "## Summary (2-4 sentences)\n"
        "## Themes (bullet list)\n"
        "## Action Items (bullet list, concrete next steps)\n"
        "Be factual — only state what the sources support. Flag uncertainty.\n"
        "Do NOT include a Sources section; that will be appended.\n\n"
        f"FOLDER: {folder}\n"
        f"DATE: {datetime.now().strftime('%Y-%m-%d')}\n\n"
        + "\n\n".join(excerpts)
    )
    body = llm.generate(
        prompt,
        system="You are Kraken, the Teledra kingdom's silent productivity assistant. "
               "Terse, actionable, no fluff.",
    ).strip()

    source_lines = "\n".join(
        f"- `{s['path']}` ({s['mode']}, {s['bytes']} bytes)"
        for s in manifest["sources"]
    )
    waived_lines = ""
    if manifest["waived"]:
        waived_lines = "\n\n## Waived\n\n" + "\n".join(
            f"- `{w['path']}` — {w['reason']}" for w in manifest["waived"]
        )

    report = (
        f"# Daily Brief — {os.path.basename(folder)}\n\n"
        f"_Generated {datetime.now().strftime('%Y-%m-%d %H:%M')}_\n\n"
        f"{body}\n\n## Sources\n\n{source_lines}{waived_lines}\n"
    )
    out_path = _write(ctx, job, report)
    return {
        "ok": True,
        "output": out_path,
        "notes": f"{len(manifest['sources'])} sources, {len(manifest['waived'])} waived",
    }


def _write(ctx: dict, job: dict, report: str) -> str:
    vault = os.path.join(ctx["root"], "vault")
    os.makedirs(vault, exist_ok=True)
    path = os.path.join(vault, f"{job['id']}-digest.md")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(report)
    return os.path.relpath(path, ctx["root"])