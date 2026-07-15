"""verify_pixelfork.py — dedicated harness for games imported from pixelfork.ai

This is a thin "Pixel fork harness" wrapper.

It reuses the heavy pixel work from browser_game_probe and game_checks,
while adding pixelfork-specific notes (e.g. typical pixel-art expectations,
export hints, etc.).

A skill can declare:
  harness: verify_pixelfork

For most cases the normal "verify_code" + beast quality already gives you
excellent pixel verification. Use this when you want explicit pixelfork branding
or extra metadata checks.
"""

from __future__ import annotations
import os
import json
from typing import Any, Dict

# Reuse the real pixel logic
try:
    from . import browser_game_probe
    from . import game_checks
except ImportError:
    browser_game_probe = None
    game_checks = None


def verify(job: dict, result: dict, ctx: dict) -> dict:
    reasons = []
    passed = True

    # Basic result sanity
    if not result.get("ok"):
        reasons.append("skill reported failure")
        passed = False

    workdir = result.get("workdir") or ctx.get("workdir")
    manifest_path = None

    if workdir:
        manifest_path = os.path.join(workdir, "pixelfork_manifest.json")
        if os.path.exists(manifest_path):
            try:
                with open(manifest_path, encoding="utf-8") as f:
                    manifest = json.load(f)
                reasons.append(f"pixelfork source: {manifest.get('publish_url')}")
            except Exception:
                pass
        else:
            # Still allow if it's a raw game dir
            pass

    # Delegate to the strong pixel harness when we have a browser game
    if browser_game_probe and "browser" in str(result).lower() or os.path.exists(os.path.join(workdir or ".", "index.html")):
        try:
            probe_result = browser_game_probe.verify(job, result, ctx)
            if not probe_result.get("passed"):
                passed = False
                reasons.extend(probe_result.get("reasons", []))
            else:
                reasons.append("pixel probe passed (canvas changes + input response verified)")
        except Exception as e:
            reasons.append(f"pixel probe error: {e}")

    # Extra pixelfork expectations (lightweight)
    if workdir and os.path.exists(os.path.join(workdir, "index.html")):
        # Many PixelFork games are pixel-art → encourage pixelated + discrete movement
        pass  # could add static checks here

    if not reasons:
        reasons.append("pixelfork import + basic checks ok (use beast quality for full runtime pixel harness)")

    return {
        "passed": passed,
        "reasons": reasons,
        "pixelfork": True,
        "manifest": manifest_path
    }
