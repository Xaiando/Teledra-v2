"""pixelfork skill — bring a published game from pixelfork.ai into Kraken for forking + verification.

The "Pixel fork harness" is largely covered by the existing game harnesses:
- browser_game_probe.py does real pixel sampling (getImageData, hash changes, input response).
- game_checks + verify_code for static + runtime.
- "beast" quality in code_forge triggers the full pixel + playability probe.

This skill acts as the entry point / importer.
"""

from __future__ import annotations
import os
import re
import json
from typing import Any

import requests  # safe inside Kraken context for fetch

def execute(job: dict, ctx: dict) -> dict:
    log = ctx.get("log", print)
    root = ctx["root"]
    workdir = ctx["workdir"]

    inp = job.get("input", "")
    if isinstance(inp, str):
        url = inp.strip()
        improve = True
    else:
        url = inp.get("url", "").strip()
        improve = inp.get("improve", True)

    if not url or "pixelfork.ai/publish/" not in url:
        return {
            "ok": False,
            "notes": "Input must be a pixelfork.ai/publish/<id> URL",
            "output": ""
        }

    # Extract slug
    m = re.search(r"/publish/([0-9a-zA-Z]+)", url)
    slug = m.group(1) if m else "unknown"

    log(f"[pixelfork] Processing published game {slug} from {url}")

    # Fetch page metadata (best effort)
    meta = {"slug": slug, "url": url, "title": "Published Game | Pixelfork"}
    try:
        resp = requests.get(url, timeout=15, headers={"User-Agent": "Kraken-PixelFork-Importer/1.0"})
        if resp.ok:
            text = resp.text
            # crude extraction
            title_m = re.search(r"<title>([^<]+)</title>", text, re.I)
            if title_m:
                meta["title"] = title_m.group(1).strip()
            desc_m = re.search(r'<meta name="description" content="([^"]+)"', text, re.I)
            if desc_m:
                meta["description"] = desc_m.group(1)
            meta["fetched"] = True
    except Exception as e:
        meta["fetch_error"] = str(e)
        log(f"[pixelfork] metadata fetch warning: {e}")

    # Create workspace target
    target_dir = os.path.join(root, "workspace", "games", f"pixelfork-{slug}")
    os.makedirs(target_dir, exist_ok=True)

    # Write a manifest + seed
    manifest = {
        "source": "pixelfork.ai",
        "publish_url": url,
        "slug": slug,
        "meta": meta,
        "notes": "Imported via Kraken pixelfork skill. Use code_forge with quality=beast for full pixel harness verification.",
        "harness_recommendation": "verify_code + browser_game_probe (pixel sampling)"
    }

    with open(os.path.join(target_dir, "pixelfork_manifest.json"), "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)

    # Write a README seed for the fork
    readme = f"""# PixelFork Game Fork: {slug}

Source: {url}
Title: {meta.get('title', 'Unknown')}
Description: {meta.get('description', 'AI-generated pixel game published on Pixelfork')}

## How to work with this in Kraken

1. This is a seed import.
2. Run code_forge on this directory to recreate or improve the game:
   kraken.py add code_forge '{{"task": "Fork and polish this PixelFork game. Make the pixel art and mechanics solid. Ensure canvas pixels change on input.", "dir": "workspace/games/pixelfork-{slug}", "quality": "beast"}}'
3. The beast quality + browser_game_probe will act as your "Pixel fork harness":
   - Samples live canvas pixels
   - Verifies animation (RAF + pixel hash change)
   - Tests input response
   - Checks for non-blank / working visuals

## Existing harnesses that apply
- browser_game_probe (pixel-level runtime checks)
- game_checks
- verify_code (with py tests if added)

Kraken lessons from previous games will be recalled automatically.

"""

    with open(os.path.join(target_dir, "README.md"), "w", encoding="utf-8") as f:
        f.write(readme)

    # Optionally spawn a child code_forge job for immediate improvement
    children = []
    if improve:
        children.append({
            "skill": "code_forge",
            "input": json.dumps({
                "task": f"Fork and improve the PixelFork published game at {url}. Focus on solid pixel-art mechanics, ensure the canvas shows changing pixels on input and Play. Make it robust and fun. Use the manifest in the target dir as reference.",
                "dir": f"workspace/games/pixelfork-{slug}",
                "quality": "beast",
                "verify_only": False
            })
        })
        log("[pixelfork] Spawned code_forge child for fork + pixel verification")

    output_path = os.path.join(target_dir, "pixelfork_manifest.json")

    return {
        "ok": True,
        "output": output_path,
        "notes": f"PixelFork game {slug} imported to {target_dir}. Pixel harness ready via beast quality probe.",
        "children": children,
        "metadata": meta
    }
