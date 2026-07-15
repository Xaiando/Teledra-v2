"""Continuous, off-air Court Synth composition laboratory.

It produces theory-audited candidates while Teledra is alive.  It never writes
current_score.json: promotion remains the court/human feedback system's job.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import time
import wave
from copy import deepcopy
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
os.chdir(ROOT)
sys.path.insert(0, str(ROOT))
from court_synthesizer import (  # noqa: E402
    SAMPLE_RATE, arrangement_interest_report, default_score, expand_score,
    render_score, validate_score,
)
from court_synth import harmony  # noqa: E402

LAB = ROOT / "court_synth" / "lab"
THEORY = ROOT / "knowledge" / "music_theory_foundation.md"
LESSONS = ROOT / "knowledge" / "music_theory_lessons.jsonl"

PROFILES = (
    ("gothic_baroque", "court_experimental", "solemnity into defiant radiance",
     ("kit.acoustic_hybrid", "fx.crack", "bass.restrained_reese", "keys.chapel_organ", "pluck.bell_tower", "ensemble.violin_air", "pad.dungeon_drone", "fx.reverse_bloom")),
    ("chamber_fantasy", "retro_adventure", "wonder, peril, and homecoming",
     ("kit.quest_8bit", "pluck.wood_marimba", "bass.upright_pluck", "keys.stage_grand", "pluck.celtic_harp", "ensemble.cello_section", "pad.aurora_choir", "fx.shimmer")),
    ("jazzhop_velvet", "spicy_lofi", "late-night ease growing into confidence",
     ("kit.velvet_lofi", "pluck.kalimba", "bass.upright_pluck", "keys.reed_electric", "pluck.steel_string", "lead.whisper_sine", "pad.tape_haze", "fx.tape_stop")),
    ("synthwave_noir", "court_experimental", "suspicion, pursuit, release",
     ("kit.mechanical_court", "fx.noise_sweep", "bass.reese_growl", "keys.tine_electric", "pluck.glass_current", "lead.prism_pulse", "pad.starfield", "fx.downlifter")),
    ("eight_bit_quest", "retro_adventure", "curiosity, danger, earned triumph",
     ("kit.quest_8bit", "pluck.wood_marimba", "bass.chip_quest", "keys.toy_chime", "pluck.frost_dulcimer", "lead.chip_hero", "pad.frost_shimmer", "fx.impact")),
    ("ambient_cinematic", "spicy_lofi", "stillness opening into awe",
     ("kit.lofi_dust", "fx.shimmer", "bass.sub_pluck", "ensemble.bowed_synth", "pluck.royal_harp", "lead.sine_dream", "pad.granular_rain", "fx.reverse_bloom")),
    ("electro_acid", "court_experimental", "restraint, pressure, ecstatic release",
     ("kit.brutal_impact", "fx.sweep_up", "bass.acid_mono", "keys.clavinet", "pluck.lute_pulse", "lead.saw_bite", "pad.sub_bass_pad", "fx.tape_stop")),
)
TRACKS = ("drums", "percussion", "bass", "harmony", "pluck", "lead", "atmos", "fx")


def _atomic(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(tmp, path)


def _theory_digest() -> dict[str, object]:
    text = THEORY.read_text(encoding="utf-8", errors="replace") if THEORY.exists() else ""
    lessons = LESSONS.read_text(encoding="utf-8", errors="replace").splitlines()[-12:] if LESSONS.exists() else []
    return {"foundation_sha256": hashlib.sha256(text.encode()).hexdigest(), "lesson_count_sampled": len(lessons),
            "principles": ["functional harmonic direction", "four-bar rhythmic hierarchy", "motif development", "register separation", "dynamic sectional contrast", "prepared loop return"]}


def build_candidate(cycle: int) -> dict:
    genre, style, emotion, patches = PROFILES[cycle % len(PROFILES)]
    score = deepcopy(default_score(style))
    score["project_id"] = f"living-lab-{cycle:06d}"
    score["title"] = f"{genre.replace('_', ' ').title()} Study {cycle + 1}"
    score["revision"] = cycle + 1
    score["seed"] = 730_000 + cycle * 7919
    score["genre"] = genre
    score["emotion_arc"] = emotion
    score["instrumentation"] = {
        track: {"patch_id": patch, "macros": {"tone": .48 + ((cycle + index) % 5) * .1,
                                                "character": .30 + ((cycle * 3 + index) % 6) * .09}}
        for index, (track, patch) in enumerate(zip(TRACKS, patches))
    }
    energies = (.24, .36, .50, .64, .82, .70, .55, .38)
    for index, section in enumerate(score["sections"]):
        # Every genre receives a legible dramatic arc and a quiet final return;
        # rotating the arc itself can erase the recognisable opening motif.
        section["energy"] = energies[index]
    return score


def run_cycle(cycle: int) -> dict:
    LAB.mkdir(parents=True, exist_ok=True)
    started = time.time()
    score = build_candidate(cycle)
    errors = validate_score(score)
    events, _tracks, sections = expand_score(score) if not errors else ([], [], [])
    interest = arrangement_interest_report(score, events, sections) if not errors else {"ok": False, "issues": errors}
    harmonic = harmony.grade_score(score, []) if not errors else {"ok": False, "issues": errors}
    audio, summary = render_score(score) if not errors and interest.get("ok") and harmonic.get("ok") else (None, {})
    report = {
        "cycle": cycle, "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "genre": score["genre"], "emotion_arc": score["emotion_arc"],
        "instrument_count": len({v["patch_id"] for v in score["instrumentation"].values()}),
        "theory": _theory_digest(), "schema_errors": errors, "harmony": harmonic,
        "arrangement": interest, "render_summary": summary,
    }
    report["ok"] = not errors and bool(interest.get("ok")) and bool(harmonic.get("ok")) and audio is not None
    if audio is not None:
        report["audio"] = {"peak": float(np.max(np.abs(audio))), "rms": float(np.sqrt(np.mean(audio ** 2))), "seconds": len(audio) / SAMPLE_RATE}
        wav = LAB / "latest.wav"
        with wave.open(str(wav), "wb") as handle:
            handle.setnchannels(2); handle.setsampwidth(2); handle.setframerate(SAMPLE_RATE)
            handle.writeframes((np.clip(audio, -1, 1) * 32767).astype("<i2").tobytes())
    if report["ok"]:
        _atomic(LAB / "latest_candidate.json", score)
    _atomic(LAB / "latest_report.json", report)
    _atomic(LAB / "state.json", {"alive": True, "cycle": cycle, "last_ok": report["ok"], "genre": score["genre"], "pid": os.getpid(), "cycle_seconds": round(time.time() - started, 3)})
    with (LAB / "history.jsonl").open("a", encoding="utf-8") as handle:
        handle.write(json.dumps({k: report[k] for k in ("cycle", "created_at", "genre", "emotion_arc", "instrument_count", "ok")}) + "\n")
    return report


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--interval", type=float, default=float(os.getenv("TELEDRA_MUSIC_LAB_INTERVAL", "180")))
    parser.add_argument("--once", action="store_true")
    args = parser.parse_args()
    state = json.loads((LAB / "state.json").read_text(encoding="utf-8")) if (LAB / "state.json").exists() else {}
    cycle = int(state.get("cycle", -1)) + 1
    while True:
        print(json.dumps(run_cycle(cycle)), flush=True)
        if args.once: break
        cycle += 1
        time.sleep(max(30.0, args.interval))


if __name__ == "__main__":
    main()
