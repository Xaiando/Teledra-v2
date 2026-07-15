"""prod_vault — distill a vault report into an evergreen note."""

from __future__ import annotations

import json
import os
import re
from datetime import datetime

MANIFEST_NAME = "sources_manifest.json"
READ_LIMIT = 24_000


def _resolve_source(raw: str, root: str) -> str:
    raw = raw.strip().strip('"')
    if os.path.isabs(raw):
        path = raw
    else:
        path = os.path.join(root, raw.replace("/", os.sep))
    path = os.path.normpath(path)
    vault = os.path.normpath(os.path.join(root, "vault"))
    if not path.startswith(vault):
        raise ValueError(f"source must live under vault/: {raw}")
    if not os.path.isfile(path):
        raise ValueError(f"source report not found: {path}")
    return path


def execute(job: dict, ctx: dict) -> dict:
    root = ctx["root"]
    source = _resolve_source(job["input"], root)
    ctx["log"](f"prod_vault distilling {source}")

    with open(source, "r", encoding="utf-8", errors="ignore") as fh:
        text = fh.read(READ_LIMIT)

    workdir = ctx["workdir"]
    os.makedirs(workdir, exist_ok=True)
    manifest = {
        "skill": "prod_vault",
        "generated": datetime.now().isoformat(timespec="seconds"),
        "sources": [{"path": source, "mode": "full_report", "bytes": len(text)}],
        "waived": [],
    }
    with open(os.path.join(workdir, MANIFEST_NAME), "w", encoding="utf-8") as fh:
        json.dump(manifest, fh, indent=2)

    llm = ctx["llm"]
    prompt = (
        "Distill this report into a compact evergreen knowledge note.\n"
        "Use markdown with these sections exactly:\n"
        "## Evergreen (1-2 sentence essence)\n"
        "## Key Facts (bullet list of durable facts only)\n"
        "## References (bullet list of cited paths, URLs, or named sources from the report)\n"
        "Drop ephemeral timestamps and session noise. Keep only what should still matter "
        "in six months. Do NOT include a Source section.\n\n"
        f"REPORT ({os.path.basename(source)}):\n\n{text}"
    )
    body = llm.generate(
        prompt,
        system="You are Kraken, the Teledra kingdom's silent archivist. "
               "Precise, durable, no invented facts.",
    ).strip()

    stem = os.path.splitext(os.path.basename(source))[0]
    stem = re.sub(r"-evergreen$", "", stem)
    report = (
        f"# Evergreen — {stem}\n\n"
        f"_Distilled {datetime.now().strftime('%Y-%m-%d %H:%M')}_\n\n"
        f"{body}\n\n## Source\n\n- `{source}`\n"
    )

    vault = os.path.join(root, "vault")
    os.makedirs(vault, exist_ok=True)
    out_name = f"{stem}-evergreen.md"
    out_path = os.path.join(vault, out_name)
    with open(out_path, "w", encoding="utf-8") as fh:
        fh.write(report)

    rel = os.path.relpath(out_path, root)
    return {"ok": True, "output": rel, "notes": f"distilled from {os.path.basename(source)}"}