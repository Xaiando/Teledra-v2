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

from composer_harness import (
    audio_quality_metrics,
    evaluate_composer_events,
    evaluate_composer_plan,
    event_audio_alignment,
)


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
ISSUE_AUDIO_SHAPE = "invalid_audio_shape"
ISSUE_MISSING_SCORE = "missing_score"
ISSUE_MISSING_AUTOMATION = "missing_automation"
ISSUE_THIN_ARRANGEMENT = "thin_arrangement"
ISSUE_LAYER_ALIGNMENT = "layer_alignment"
ISSUE_MISSING_SECTIONS = "missing_sections"
ISSUE_FLAT_FORM = "flat_form"
ISSUE_DC_OFFSET = "dc_offset"
ISSUE_OVERCOMPRESSED = "overcompressed_mix"
ISSUE_HARSH_MIX = "harsh_mix"
ISSUE_UNDERPOWERED = "underpowered_mix"


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
            array = np.asarray(wave, dtype=float)
            if array.ndim == 1:
                mapped[str(name)] = array
            elif array.ndim == 2 and array.shape[1] in (1, 2):
                mapped[str(name)] = array
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
    composer_grade: bool = False,
) -> dict[str, Any]:
    candidate = str(candidate)
    captured: dict[str, Any] = {}
    notes: list[dict[str, Any]] = []
    issues: list[dict[str, Any]] = []
    namespace: dict[str, Any] = {}
    composer_advisories: list[dict[str, Any]] = []
    composer_metrics: dict[str, Any] = {}

    import teledra_synth

    original_play = teledra_synth.play_sound
    original_note_to_freq = teledra_synth.note_to_freq

    def capture_play(wave: Any, sr: int = 44100, loop: bool = False, **_kwargs: Any) -> None:
        captured["wave"] = np.asarray(wave, dtype=float)
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
        wave = np.asarray(wave, dtype=float)
        valid_shape = wave.ndim == 1 or (wave.ndim == 2 and wave.shape[1] in (1, 2))
        if not valid_shape:
            issues.append(_issue(ISSUE_AUDIO_SHAPE, "Full mix must be mono frames or frames x 1/2 channels.", shape=list(wave.shape)))
            wave = wave.reshape(-1)
        frame_count = int(wave.shape[0]) if wave.ndim else 0
        duration = float(frame_count) / float(sr)
        if not np.all(np.isfinite(wave)):
            issues.append(_issue(ISSUE_NONFINITE, "Full mix contains NaN or Inf samples."))
        else:
            mix_rms = _rms(wave)
            peak = float(np.max(np.abs(wave))) if wave.size else 0.0
            if mix_rms < rms_floor:
                issues.append(_issue(ISSUE_SILENT_MIX, "Full mix is silent or below the energy floor.", rms=mix_rms))
            if peak >= clip_threshold:
                issues.append(_issue(ISSUE_CLIPPING, "Full mix reaches the clipping threshold.", peak=peak, threshold=clip_threshold))
            if frame_count >= 2:
                endpoint_delta = float(np.max(np.abs(wave[0] - wave[-1])))
                window = min(max(1, int(sr * 0.02)), frame_count // 2)
                edge_delta = float(np.max(np.abs(np.mean(wave[:window], axis=0) - np.mean(wave[-window:], axis=0))))
                if max(endpoint_delta, edge_delta) > seam_threshold:
                    issues.append(_issue(ISSUE_LOOP_SEAM, "Loop endpoints are discontinuous.", endpoint_delta=endpoint_delta, edge_delta=edge_delta, threshold=seam_threshold))
        if duration < min_duration:
            issues.append(_issue(ISSUE_TOO_SHORT, "Composition is shorter than required.", duration=duration, minimum=min_duration))

    layers = _wave_map(namespace.get("TELEDRA_LAYERS"))
    sections = _wave_map(namespace.get("TELEDRA_SECTIONS"))
    score = namespace.get("TELEDRA_SCORE")
    automation = namespace.get("TELEDRA_AUTOMATION")
    per_layer_rms = {name: _rms(layer) for name, layer in layers.items()}
    per_section_rms = {name: _rms(section) for name, section in sections.items()}
    if len(layers) < 5:
        issues.append(_issue(
            ISSUE_THIN_ARRANGEMENT,
            "Expose at least five real audible layer buffers with distinct musical roles.",
            layers=list(layers),
            minimum=5,
        ))
    if wave is not None:
        full_frames = int(np.asarray(wave).shape[0])
        misaligned = {name: int(layer.shape[0]) for name, layer in layers.items() if layer.shape[0] != full_frames}
        if misaligned:
            issues.append(_issue(
                ISSUE_LAYER_ALIGNMENT,
                "Every TELEDRA_LAYERS buffer must align to the final mix length.",
                full_frames=full_frames,
                layer_frames=misaligned,
            ))
    if len(sections) < 4:
        issues.append(_issue(
            ISSUE_MISSING_SECTIONS,
            "Expose at least four real section slices from the arranged mix.",
            sections=list(sections),
            minimum=4,
        ))
    if not isinstance(score, dict):
        issues.append(_issue(ISSUE_MISSING_SCORE, "Declare TELEDRA_SCORE with motif, section plan, and depth roles."))
    else:
        score_sections = score.get("sections")
        depth_roles = score.get("depth_roles")
        if not isinstance(score_sections, (list, tuple)) or len(score_sections) < 4:
            issues.append(_issue(ISSUE_MISSING_SCORE, "TELEDRA_SCORE.sections must plan at least four sections."))
        if not isinstance(depth_roles, dict) or len(depth_roles) < 3:
            issues.append(_issue(ISSUE_MISSING_SCORE, "TELEDRA_SCORE.depth_roles must map foreground, midground, and background roles."))
    if not isinstance(automation, dict) or len(automation) < 3:
        issues.append(_issue(
            ISSUE_MISSING_AUTOMATION,
            "Declare at least three form-serving movements in TELEDRA_AUTOMATION and apply them to the audio.",
        ))
    for name, value in per_layer_rms.items():
        if value < rms_floor:
            issues.append(_issue(ISSUE_DEAD_LAYER, f"Layer '{name}' is silent or below the energy floor.", layer=name, rms=value))
    for name, value in per_section_rms.items():
        if value < rms_floor:
            issues.append(_issue(ISSUE_DEAD_SECTION, f"Section '{name}' is silent or below the energy floor.", section=name, rms=value))
    live_section_energy = [value for value in per_section_rms.values() if value >= rms_floor]
    if len(live_section_energy) >= 4:
        floor = max(min(live_section_energy), rms_floor)
        energy_ratio = max(live_section_energy) / floor
        source_text = Path(candidate).read_text(encoding="utf-8", errors="replace").lower()
        ambient = any(marker in source_text for marker in ("ambient", "ambience", "soundscape", "drone", "atmosphere"))
        minimum_ratio = 1.035 if ambient else 1.12
        if energy_ratio < minimum_ratio:
            issues.append(_issue(
                ISSUE_FLAT_FORM,
                "Section energy is too flat; create an audible arrival, development, peak, and release.",
                energy_ratio=energy_ratio,
                minimum_ratio=minimum_ratio,
            ))

    composer_plan = namespace.get("TELEDRA_COMPOSER")
    composer_events = namespace.get("TELEDRA_EVENTS")
    if composer_grade:
        plan_report = evaluate_composer_plan(
            composer_plan,
            bpm=namespace.get("BPM"),
            section_count=len(sections),
        )
        issues.extend(plan_report["issues"])
        composer_advisories.extend(plan_report["advisories"])
        composer_metrics.update(plan_report["metrics"])
        score_sections = score.get("sections") if isinstance(score, dict) else list(sections)
        beats_per_bar: Any = namespace.get("BEATS_PER_BAR", 4)
        if isinstance(score, dict):
            meter = score.get("meter")
            if isinstance(meter, (list, tuple)) and meter:
                beats_per_bar = meter[0]
            elif isinstance(meter, (int, float)):
                beats_per_bar = meter
        event_report = evaluate_composer_events(
            composer_events,
            composer_plan,
            bpm=namespace.get("BPM"),
            bars=namespace.get("BARS"),
            beats_per_bar=beats_per_bar,
            section_names=score_sections,
            layer_names=list(layers),
            audio_duration=duration,
        )
        normalized_events = event_report.pop("normalized_events", [])
        issues.extend(event_report["issues"])
        composer_advisories.extend(event_report["advisories"])
        composer_metrics.update(event_report["metrics"])
        if normalized_events and layers:
            alignment_report = event_audio_alignment(
                normalized_events,
                layers,
                bpm=namespace.get("BPM"),
                sample_rate=sr,
            )
            issues.extend(alignment_report["issues"])
            composer_advisories.extend(alignment_report["advisories"])
            composer_metrics.update(alignment_report["metrics"])
        if wave is not None and np.all(np.isfinite(np.asarray(wave, dtype=float))):
            mix_metrics = audio_quality_metrics(wave, sr)
            composer_metrics.update(mix_metrics)
            dc_offset = mix_metrics.get("dc_offset", 0.0)
            crest = mix_metrics.get("crest_factor", 0.0)
            high_ratio = mix_metrics.get("high_frequency_ratio", 0.0)
            measured_peak = mix_metrics.get("mix_peak", peak)
            measured_rms = mix_metrics.get("mix_rms", mix_rms)
            if dc_offset > 0.02:
                issues.append(_issue(
                    ISSUE_DC_OFFSET,
                    "Mix has excessive DC offset; center oscillators/noise before mastering.",
                    value=dc_offset,
                ))
            if crest and crest < 1.22:
                issues.append(_issue(
                    ISSUE_OVERCOMPRESSED,
                    "Mix has almost no transient contrast; rebalance before limiting.",
                    crest_factor=crest,
                ))
            elif crest > 16.0:
                composer_advisories.append(_issue(
                    "spiky_mix",
                    "Mix is unusually transient-heavy; check isolated clicks and percussion peaks.",
                    crest_factor=crest,
                ))
            if high_ratio > 0.28:
                issues.append(_issue(
                    ISSUE_HARSH_MIX,
                    "Too much energy sits above 8 kHz; tame noise, hats, or bright waveforms.",
                    high_frequency_ratio=high_ratio,
                ))
            if measured_peak < 0.06 or measured_rms < 0.004:
                issues.append(_issue(
                    ISSUE_UNDERPOWERED,
                    "Mix is too weak for unnormalised playback; raise authored track gains before the limiter.",
                    peak=measured_peak,
                    rms=measured_rms,
                    minimum_peak=0.06,
                    minimum_rms=0.004,
                ))
            elif measured_peak < 0.22 or measured_rms < 0.018:
                composer_advisories.append(_issue(
                    ISSUE_UNDERPOWERED,
                    "Mix may play quietly now that playback preserves authored headroom; consider gain staging before limiting.",
                    peak=measured_peak,
                    rms=measured_rms,
                    target_peak=0.22,
                    target_rms=0.018,
                ))

    quality_score = max(0, 100 - 10 * len(issues) - 2 * len(composer_advisories))

    return {
        "ok": not issues,
        "issues": issues,
        "per_layer_rms": per_layer_rms,
        "per_section_rms": per_section_rms,
        "peak": peak,
        "rms": mix_rms,
        "duration": duration,
        "sample_rate": sr,
        "channels": 0 if wave is None else (1 if np.asarray(wave).ndim == 1 else int(np.asarray(wave).shape[1])),
        "loop": bool(captured.get("loop", False)),
        "notes_checked": len(notes),
        "layer_manifest": bool(layers),
        "section_manifest": bool(sections),
        "score_manifest": isinstance(score, dict),
        "automation_manifest": isinstance(automation, dict),
        "composer_grade": bool(composer_grade),
        "composer_plan": isinstance(composer_plan, dict),
        "composer_events": isinstance(composer_events, list) and bool(composer_events),
        "composer_metrics": composer_metrics,
        "composer_advisories": composer_advisories,
        "quality_score": quality_score,
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
    parser.add_argument("--composer-grade", action="store_true")
    args = parser.parse_args()
    report = verify_file(
        args.candidate,
        min_duration=args.min_duration,
        clip_threshold=args.clip_threshold,
        composer_grade=args.composer_grade,
    )
    if args.lessons:
        append_lesson(report, args.candidate, args.lessons)
    print(json.dumps(report, ensure_ascii=True, sort_keys=True))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
