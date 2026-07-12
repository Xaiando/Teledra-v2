#!/usr/bin/env python3
"""
One-hour soak / benchmark helper for Teledra kingdom (next-tier #3).

Runs a configurable number of cycles exercising:
- kingdom_dashboard --snapshot-json (missions, research, Fractus, TTS readiness, health)
- Fractus headless renders + validation for several families
- Basic file I/O and state snapshot reads (bounded)
- Optional light TTS dry (skipped by default to avoid heavy model load every cycle)

Records timings, success/failure counts, and produces a durable JSON report.

Usage:
  python -m soak_benchmark --cycles 20 --report coordination/soak_report.json
  (run from repo root with the .venv active, or invoke .venv/Scripts/python.exe directly)

Intended to be run while the main Teledra process is also active (or standalone).
Produces evidence for "soak mission" durability, latency, and recovery characteristics.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent
DASHBOARD = ROOT / "kingdom_dashboard.py"
FRACTUS_GUI = ROOT / "Fractus" / "fractus_gui.py"
VENV_PY = ROOT / ".venv" / "Scripts" / "python.exe"

DEFAULT_CYCLES = 12          # ~ a short soak; scale up for real 1h run
DEFAULT_REPORT = ROOT / "coordination" / "soak_report.json"
FAMILIES_TO_TEST = ["mandelbrot", "lotus_mandala", "guilloche", "newton", "lissajous", "reaction_diffusion"]


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def run_cmd(cmd: list[str], timeout: float = 60.0) -> dict[str, Any]:
    start = time.perf_counter()
    try:
        proc = subprocess.run(
            cmd,
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=timeout,
            creationflags=0x08000000 if os.name == "nt" else 0,  # CREATE_NO_WINDOW on win
        )
        dur = time.perf_counter() - start
        return {
            "ok": proc.returncode == 0,
            "duration_s": round(dur, 3),
            "returncode": proc.returncode,
            "stdout_tail": (proc.stdout or "")[-65_536:],
            "stderr_tail": (proc.stderr or "")[-16_384:],
        }
    except subprocess.TimeoutExpired as e:
        dur = time.perf_counter() - start
        return {
            "ok": False,
            "duration_s": round(dur, 3),
            "error": "timeout",
            "stdout_tail": (e.stdout or b"")[-65_536:].decode(errors="replace") if e.stdout else "",
            "stderr_tail": (e.stderr or b"")[-16_384:].decode(errors="replace") if e.stderr else "",
        }
    except Exception as e:
        dur = time.perf_counter() - start
        return {"ok": False, "duration_s": round(dur, 3), "error": str(e)}


def snapshot_dashboard() -> dict[str, Any]:
    if not VENV_PY.exists() or not DASHBOARD.exists():
        return {"ok": False, "error": "missing dashboard or venv"}
    cmd = [str(VENV_PY), str(DASHBOARD), "--snapshot-json", "--no-process-probe"]
    res = run_cmd(cmd, timeout=30.0)
    parsed = None
    if res["ok"] and res.get("stdout_tail"):
        try:
            parsed = json.loads(res["stdout_tail"])
        except Exception:
            pass
    res["parsed_health"] = parsed.get("health", {}) if parsed else None
    res["fractus_health"] = parsed.get("fractus", {}).get("health") if parsed else None
    return res


def fractus_smoke(family: str) -> dict[str, Any]:
    """Render and validate the family named by the benchmark step.

    The manifest is part of the acceptance gate. A stale file or a render of a
    different recipe cannot be counted as coverage for ``family``.
    """
    if not VENV_PY.exists() or not FRACTUS_GUI.exists():
        return {"ok": False, "error": "missing fractus"}
    out_dir = ROOT / "Fractus" / "output"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"soak_{family.replace('/', '_')}.png"
    started_ns = time.time_ns()
    seed = 80_000 + FAMILIES_TO_TEST.index(family)
    cmd = [
        str(VENV_PY), str(FRACTUS_GUI),
        "--headless",
        "--type", family,
        "--iterations", "120",
        "--palette", "twilight",
        "--seed", str(seed),
        "--width", "256",
        "--height", "256",
        "--output", str(out_path),
    ]
    res = run_cmd(cmd, timeout=45.0)
    res["family"] = family
    manifest_path = out_path.with_name(out_path.name + ".json")
    manifest = None
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        pass
    rendered_family = (
        manifest.get("scene", {}).get("layers", [{}])[0].get("family")
        if isinstance(manifest, dict)
        else None
    )
    fresh_output = (
        out_path.is_file()
        and manifest_path.is_file()
        and out_path.stat().st_mtime_ns >= started_ns
        and manifest_path.stat().st_mtime_ns >= started_ns
    )
    res["output_exists"] = out_path.is_file()
    res["fresh_output"] = fresh_output
    res["manifest_family"] = rendered_family
    res["recipe_hash"] = manifest.get("recipe_hash") if isinstance(manifest, dict) else None
    res["render_hash"] = manifest.get("render_hash") if isinstance(manifest, dict) else None

    val_cmd = [
        str(VENV_PY), str(FRACTUS_GUI),
        "--validate",
        "--type", family,
        "--iterations", "120",
        "--palette", "twilight",
        "--seed", str(seed),
        "--width", "256",
        "--height", "256",
    ]
    val = run_cmd(val_cmd, timeout=20.0)
    validated_family = None
    if val.get("ok"):
        try:
            validated = json.loads(val.get("stdout_tail", ""))
            validated_family = validated.get("layers", [{}])[0].get("family")
        except (json.JSONDecodeError, AttributeError, IndexError):
            pass
    res["validate_ok"] = val.get("ok", False) and validated_family == family
    res["ok"] = bool(
        res.get("ok")
        and fresh_output
        and rendered_family == family
        and res["validate_ok"]
        and res["recipe_hash"]
        and res["render_hash"]
    )
    return res


def light_tts_dry() -> dict[str, Any]:
    """Dry TTS probe: just checks the script is executable and usage works (avoids full model load in soak)."""
    if not VENV_PY.exists():
        return {"ok": False, "error": "no venv"}
    # Just invoke with bad args to hit usage path quickly (no model load)
    cmd = [str(VENV_PY), str(ROOT / "generate_voice.py")]
    res = run_cmd(cmd, timeout=10.0)
    # We expect the explicit usage error, not an arbitrary fast crash.
    res["note"] = "usage path only (no model load)"
    res["ok"] = bool(
        res.get("returncode") == 1
        and "Usage:" in res.get("stderr_tail", "")
        and res.get("duration_s", 99) < 10
    )
    return res


def main() -> None:
    parser = argparse.ArgumentParser(description="Teledra kingdom soak benchmark")
    parser.add_argument("--cycles", type=int, default=DEFAULT_CYCLES)
    parser.add_argument("--report", type=Path, default=DEFAULT_REPORT)
    parser.add_argument("--include-tts", action="store_true", help="Include light TTS dry probes (still no full synthesis)")
    args = parser.parse_args()

    args.report.parent.mkdir(parents=True, exist_ok=True)

    report = {
        "schema_version": 1,
        "started_at": now_iso(),
        "cycles": args.cycles,
        "include_tts": args.include_tts,
        "results": [],
        "summary": {},
    }

    total_dashboard = 0.0
    total_fractus = 0.0
    dashboard_ok = 0
    fractus_ok = 0
    errors = 0
    verified_families: set[str] = set()
    recipe_hashes: set[str] = set()

    print(f"[soak] Starting {args.cycles} cycles...")

    for i in range(args.cycles):
        cycle_start = time.perf_counter()
        cycle = {"cycle": i + 1, "started_at": now_iso(), "steps": []}

        # 1. Dashboard snapshot (exercises missions, research, fractus provenance, tts readiness, health)
        db = snapshot_dashboard()
        cycle["steps"].append({"name": "dashboard_snapshot", **db})
        if db.get("ok"):
            dashboard_ok += 1
            total_dashboard += db.get("duration_s", 0)
        else:
            errors += 1

        # 2. Fractus smokes for a few families (exercises registry, render, manifest, headless)
        for fam in FAMILIES_TO_TEST[:3]:  # keep short per cycle
            fr = fractus_smoke(fam)
            cycle["steps"].append({"name": f"fractus_{fam}", **fr})
            if fr.get("ok"):
                fractus_ok += 1
                total_fractus += fr.get("duration_s", 0)
                verified_families.add(fam)
                recipe_hashes.add(fr["recipe_hash"])
            else:
                errors += 1

        # 3. Optional light TTS probe
        if args.include_tts:
            tts = light_tts_dry()
            cycle["steps"].append({"name": "tts_dry", **tts})

        cycle["duration_s"] = round(time.perf_counter() - cycle_start, 3)
        report["results"].append(cycle)

        print(f"[soak] Cycle {i+1}/{args.cycles} done in {cycle['duration_s']}s (dash_ok={dashboard_ok}, fractus_ok={fractus_ok}, errs={errors})")

    ended = now_iso()
    report["ended_at"] = ended
    report["summary"] = {
        "total_cycles": args.cycles,
        "dashboard_success": dashboard_ok,
        "fractus_success": fractus_ok,
        "errors": errors,
        "avg_dashboard_s": round(total_dashboard / max(1, dashboard_ok), 3) if dashboard_ok else None,
        "avg_fractus_s": round(total_fractus / max(1, fractus_ok), 3) if fractus_ok else None,
        "total_duration_s": round(sum(r["duration_s"] for r in report["results"]), 3),
        "verified_families": sorted(verified_families),
        "unique_recipe_hashes": len(recipe_hashes),
    }

    with open(args.report, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2)

    print("\n[soak] Complete.")
    print(json.dumps(report["summary"], indent=2))
    print(f"Report written to: {args.report}")


if __name__ == "__main__":
    main()
