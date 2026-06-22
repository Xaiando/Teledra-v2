"""Headless, structured verification for generated Teledra compositions."""

from __future__ import annotations

import argparse
import json
import os
import runpy
import sys
import time
import traceback
from pathlib import Path
from typing import Any

import numpy as np


ROOT = Path(__file__).resolve().parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

ISSUE_INVALID_NOTE = "invalid_note"
ISSUE_SILENT_MIX = "silent_mix"
ISSUE_DEAD_LAYER = "dead_layer"
ISSUE_DEAD_SECTION = "dead_section"
ISSUE_CLIPPING = "clipping"
ISSUE_LOOP_SEAM = "loop_seam"
ISSUE_RUNTIME = "runtime_error"
ISSUE_TOO_SHORT = "too_short"
ISSUE_NONFINITE = "nonfinite_samples"


def _rms(wave: Any) -> float:
    array = np.asarray(wave, dtype=float).reshape(-1)
    if array.size == 0:
        return 0.0
    return float(np.sqrt(np.mean(np.square(array))))


def _issue(code: str, message: str, **details: Any) -> dict[str, Any]:
    return {"code": code, "message": message, **details}


def _wave_map(value: Any) -> dict[str, np.ndarray]:
    if not isinstance(value, dict):
        return {}
    mapped = {}
    for name, wave in value.items():
        try:
            mapped[str(name)] = np.asarray(wave, dtype=float).reshape(-1)
        except (TypeError, ValueError):
            continue
    return mapped


def verify_file(
    candidate: os.PathLike[str] | str,
    *,
    min_duration: float = 0.0,
    clip_threshold: float = 0.98,
    rms_floor: float = 1e-5,
    seam_threshold: float = 0.05,
) -> dict[str, Any]:
    candidate = str(candidate)
    captured: dict[str, Any] = {}
    notes: list[dict[str, Any]] = []
    issues: list[dict[str, Any]] = []
    namespace: dict[str, Any] = {}

    import teledra_synth

    original_play = teledra_synth.play_sound
    original_note_to_freq = teledra_synth.note_to_freq

    def capture_play(wave: Any, sr: int = 44100, loop: bool = False, **_kwargs: Any) -> None:
        captured["wave"] = np.asarray(wave, dtype=float).reshape(-1)
        captured["sr"] = int(sr)
        captured["loop"] = bool(loop)

    def tracing_note_to_freq(note: Any) -> float:
        freq = float(original_note_to_freq(note))
        if isinstance(note, str):
            notes.append({"note": note, "freq": freq})
        return freq

    teledra_synth.play_sound = capture_play
    teledra_synth.note_to_freq = tracing_note_to_freq
    try:
        try:
            import sounddevice as sd

            sd.play = lambda *_a, **_k: None
            sd.wait = lambda *_a, **_k: None
            sd.stop = lambda *_a, **_k: None
        except Exception:
            pass
        namespace = runpy.run_path(candidate, run_name="__main__")
    except SystemExit:
        pass
    except Exception as exc:
        issues.append(_issue(ISSUE_RUNTIME, f"{type(exc).__name__}: {exc}", traceback=traceback.format_exc(limit=8)))
    finally:
        teledra_synth.play_sound = original_play
        teledra_synth.note_to_freq = original_note_to_freq

    invalid_notes = sorted({entry["note"] for entry in notes if entry["freq"] <= 0.0})
    if invalid_notes:
        issues.append(_issue(ISSUE_INVALID_NOTE, "One or more note names resolve to silence.", notes=invalid_notes))

    wave = captured.get("wave")
    sr = int(captured.get("sr", 44100) or 44100)
    duration = 0.0
    peak = 0.0
    mix_rms = 0.0
    if wave is None and not any(x["code"] == ISSUE_RUNTIME for x in issues):
        issues.append(_issue(ISSUE_RUNTIME, "Composition did not call play_sound(full_track, loop=True)."))
    elif wave is not None:
        wave = np.asarray(wave, dtype=float).reshape(-1)
        duration = float(wave.size) / float(sr)
        if not np.all(np.isfinite(wave)):
            issues.append(_issue(ISSUE_NONFINITE, "Full mix contains NaN or Inf samples."))
        else:
            mix_rms = _rms(wave)
            peak = float(np.max(np.abs(wave))) if wave.size else 0.0
            if mix_rms < rms_floor:
                issues.append(_issue(ISSUE_SILENT_MIX, "Full mix is silent or below the energy floor.", rms=mix_rms))
            if peak >= clip_threshold:
                issues.append(_issue(ISSUE_CLIPPING, "Full mix reaches the clipping threshold.", peak=peak, threshold=clip_threshold))
            if wave.size >= 2:
                endpoint_delta = float(abs(wave[0] - wave[-1]))
                window = min(max(1, int(sr * 0.02)), wave.size // 2)
                edge_delta = float(abs(np.mean(wave[:window]) - np.mean(wave[-window:])))
                if max(endpoint_delta, edge_delta) > seam_threshold:
                    issues.append(_issue(ISSUE_LOOP_SEAM, "Loop endpoints are discontinuous.", endpoint_delta=endpoint_delta, edge_delta=edge_delta, threshold=seam_threshold))
        if duration < min_duration:
            issues.append(_issue(ISSUE_TOO_SHORT, "Composition is shorter than required.", duration=duration, minimum=min_duration))

    layers = _wave_map(namespace.get("TELEDRA_LAYERS"))
    sections = _wave_map(namespace.get("TELEDRA_SECTIONS"))
    per_layer_rms = {name: _rms(layer) for name, layer in layers.items()}
    per_section_rms = {name: _rms(section) for name, section in sections.items()}
    for name, value in per_layer_rms.items():
        if value < rms_floor:
            issues.append(_issue(ISSUE_DEAD_LAYER, f"Layer '{name}' is silent or below the energy floor.", layer=name, rms=value))
    for name, value in per_section_rms.items():
        if value < rms_floor:
            issues.append(_issue(ISSUE_DEAD_SECTION, f"Section '{name}' is silent or below the energy floor.", section=name, rms=value))

    return {
        "ok": not issues,
        "issues": issues,
        "per_layer_rms": per_layer_rms,
        "per_section_rms": per_section_rms,
        "peak": peak,
        "rms": mix_rms,
        "duration": duration,
        "sample_rate": sr,
        "loop": bool(captured.get("loop", False)),
        "notes_checked": len(notes),
        "layer_manifest": bool(layers),
    }


def append_lesson(report: dict[str, Any], candidate: str, lesson_path: os.PathLike[str] | str) -> None:
    if report.get("ok"):
        return
    path = Path(lesson_path)
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "timestamp": int(time.time()),
        "candidate": os.path.basename(candidate),
        "issues": report.get("issues", []),
        "lesson": "Fix every verifier issue before replay; preserve working layers and change the failing signal, not the whole identity.",
    }
    with path.open("a", encoding="utf-8", newline="\n") as handle:
        handle.write(json.dumps(payload, ensure_ascii=True) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("candidate")
    parser.add_argument("--min-duration", type=float, default=0.0)
    parser.add_argument("--lessons")
    parser.add_argument("--clip-threshold", type=float, default=0.98)
    args = parser.parse_args()
    report = verify_file(args.candidate, min_duration=args.min_duration, clip_threshold=args.clip_threshold)
    if args.lessons:
        append_lesson(report, args.candidate, args.lessons)
    print(json.dumps(report, ensure_ascii=True, sort_keys=True))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
