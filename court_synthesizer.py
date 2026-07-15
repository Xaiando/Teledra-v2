"""Court Synth — Teledra's single native, score-driven composition surface.

This replaces code-first Python music and Strudel authoring with a declarative
CourtScore project.  It is deliberately local: Tkinter for the DAW, NumPy for
audio, sounddevice for playback, and JSON for the court-facing artifact.

The court writes musical intent (form, key, motif, harmony, style, energy),
not arbitrary DSP programs.  The compiler turns that intent into an editable
eight-track arrangement and renders it through one deterministic engine.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import re
import sys
import threading
import time
import wave
from collections import Counter
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Any, Iterable

import numpy as np

try:
    import sounddevice as sd
except Exception:  # Rendering/export remains useful on a machine without audio.
    sd = None

# CourtScore v2 remains supported, but loading a v1 project never migrates it
# implicitly.  Schema changes are explicit project operations, not a side effect
# of opening the workstation.
from court_synth import schema as court_schema
from court_synth import project_store as court_store
from court_synth import instruments as court_instruments
from court_synth import harmony as court_harmony
from court_synth import feedback as court_feedback
from court_synth import workshop as court_workshop

try:
    import tkinter as tk
    from tkinter import filedialog, messagebox
except Exception:
    tk = None
    filedialog = None
    messagebox = None


ROOT = Path(__file__).resolve().parent
PROJECT_DIR = ROOT / "court_synth"
DEFAULT_SCORE_PATH = PROJECT_DIR / "current_score.json"
STATE_PATH = PROJECT_DIR / "state.json"
RENDER_DIR = PROJECT_DIR / "renders"
AUDIO_CONFIG_PATH = PROJECT_DIR / "audio_device.json"
SAMPLE_RATE = 22_050
MIN_COMPOSITION_SECONDS = 120.0
RENDER_TAIL_SECONDS = 1.5

NOTE_RE = re.compile(r"^([A-Ga-g])([#b]?)(-?\d+)$")
CHORD_RE = court_harmony.CHORD_RE
NOTE_PC = {
    "C": 0, "C#": 1, "Db": 1, "D": 2, "D#": 3, "Eb": 3,
    "E": 4, "F": 5, "F#": 6, "Gb": 6, "G": 7, "G#": 8,
    "Ab": 8, "A": 9, "A#": 10, "Bb": 10, "B": 11,
}
PC_NAME = ("C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B")
MODES = {
    "major": (0, 2, 4, 5, 7, 9, 11),
    "natural_minor": (0, 2, 3, 5, 7, 8, 10),
    "dorian": (0, 2, 3, 5, 7, 9, 10),
    "mixolydian": (0, 2, 4, 5, 7, 9, 10),
    "phrygian": (0, 1, 3, 5, 7, 8, 10),
    "harmonic_minor": (0, 2, 3, 5, 7, 8, 11),
}
STYLES = {"retro_adventure", "spicy_lofi", "court_experimental"}
STYLE_TEMPO_RANGES = {
    "retro_adventure": (100, 128),
    "spicy_lofi": (72, 98),
    "court_experimental": (88, 116),
}

TRACKS: tuple[dict[str, Any], ...] = (
    {"id": "drums", "name": "Drums", "role": "pulse", "wave": "drum", "volume": 0.80, "pan": 0.0, "color": "#ff5c7c"},
    {"id": "percussion", "name": "Percussion", "role": "motion", "wave": "noise", "volume": 0.34, "pan": 0.28, "color": "#ffae57"},
    {"id": "bass", "name": "Sub Bass", "role": "foundation", "wave": "saw", "volume": 0.56, "pan": -0.05, "color": "#69a9ff"},
    {"id": "harmony", "name": "Harmony", "role": "middle", "wave": "triangle", "volume": 0.31, "pan": -0.20, "color": "#b796ff"},
    {"id": "pluck", "name": "Glass Pluck", "role": "rhythm", "wave": "square", "volume": 0.28, "pan": 0.18, "color": "#58e2ca"},
    {"id": "lead", "name": "Prism Lead", "role": "motif", "wave": "pulse", "volume": 0.36, "pan": 0.12, "color": "#ff82df"},
    {"id": "atmos", "name": "Atmos Pad", "role": "air", "wave": "sine", "volume": 0.19, "pan": -0.36, "color": "#64d4ff"},
    {"id": "fx", "name": "Transitions", "role": "transition", "wave": "noise", "volume": 0.21, "pan": 0.38, "color": "#d9ee55"},
)
TRACK_BY_ID = {track["id"]: track for track in TRACKS}
V1_PATCH_FAMILIES = {
    "drums": {"drums"}, "percussion": {"drums", "fx", "pluck"},
    "bass": {"bass", "pads"}, "harmony": {"keys", "ensemble", "pads"},
    "pluck": {"pluck", "keys"}, "lead": {"leads", "ensemble", "pluck"},
    "atmos": {"pads", "ensemble"}, "fx": {"fx"},
}


@dataclass(frozen=True)
class NoteEvent:
    track: str
    start: float
    duration: float
    midi: int | None
    velocity: float
    wave: str
    kind: str = "note"
    authored: bool = False


def _atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(tmp, path)


def _clamp(value: float, low: float, high: float) -> float:
    return max(low, min(high, value))


def _db_to_gain(db: float) -> float:
    return 10.0 ** (float(db) / 20.0)


def _v2_track(score: dict[str, Any], track_id: str) -> dict[str, Any] | None:
    ver = str(score.get("schema_version"))
    if ver not in ("2", "3.0"):
        return None
    return next((item for item in score.get("tracks", []) if item.get("id") == track_id), None)


def resolved_tracks(score: dict[str, Any]) -> list[dict[str, Any]]:
    """Return the eight live render tracks with real project mixer state applied."""
    result: list[dict[str, Any]] = []
    v1_mix = score.get("track_mix", {}) if score.get("schema_version") == 1 else {}
    any_solo = False
    for base in TRACKS:
        track = dict(base)
        source = _v2_track(score, base["id"])
        if source:
            mixer = source.get("mixer", {})
            instrument = source.get("instrument", source.get("instrument_bindings", {}))
            track.update({
                "name": source.get("name", track["name"]),
                "role": source.get("role", track["role"]),
                "color": source.get("color", track["color"]),
                "volume": track["volume"] * _db_to_gain(float(mixer.get("gain_db", 0.0))),
                "pan": _clamp(float(mixer.get("pan", track["pan"])), -1.0, 1.0),
                "mute": bool(mixer.get("mute", False)),
                "solo": bool(mixer.get("solo", False)),
                "arm": bool(mixer.get("arm", False)),
                "patch_id": instrument.get("patch_id"),
                "macros": dict(instrument.get("macros", {})),
                "reverb_send": _clamp(float(mixer.get("sends", {}).get("reverb", 0.0)), 0.0, 1.0) if str(score.get("schema_version")) == "2" else _clamp(float(mixer.get("reverb_send", 0.0)), 0.0, 1.0),
                "delay_send": _clamp(float(mixer.get("sends", {}).get("delay", 0.0)), 0.0, 1.0) if str(score.get("schema_version")) == "2" else _clamp(float(mixer.get("delay_send", 0.0)), 0.0, 1.0),
            })
        else:
            mixer = v1_mix.get(base["id"], {}) if isinstance(v1_mix, dict) else {}
            orchestration = score.get("instrumentation", {})
            instrument = orchestration.get(base["id"], {}) if isinstance(orchestration, dict) else {}
            patch_id = instrument.get("patch_id", court_schema.V1_TO_V2_DEFAULT_PATCHES.get(base["id"]))
            patch = court_instruments.REGISTRY.get(str(patch_id))
            macros = dict(patch.default_macros) if patch else {}
            if isinstance(instrument.get("macros"), dict):
                macros.update(instrument["macros"])
            if isinstance(mixer.get("macros"), dict):
                macros.update(mixer["macros"])
            track.update({
                "volume": _clamp(float(mixer.get("volume", track["volume"])), 0.0, 1.25),
                "pan": _clamp(float(mixer.get("pan", track["pan"])), -1.0, 1.0),
                "mute": bool(mixer.get("mute", False)),
                "solo": bool(mixer.get("solo", False)),
                "arm": bool(mixer.get("arm", False)),
                "patch_id": patch_id,
                "macros": macros,
                "reverb_send": 0.0,
                "delay_send": 0.0,
            })
        any_solo = any_solo or track["solo"]
        result.append(track)
    for track in result:
        track["audible"] = not track["mute"] and (not any_solo or track["solo"])
    return result


def manual_notes(score: dict[str, Any]) -> list[dict[str, Any]]:
    """Flatten human-authored notes without duplicating v1/v2 representations."""
    if score.get("schema_version") == 1:
        return list(score.get("manual_notes", []))
    notes: list[dict[str, Any]] = []
    for track in score.get("tracks", []):
        for clip in track.get("clips", []):
            clip_start = float(clip.get("start_beat", 0.0))
            for note in clip.get("notes", []):
                item = dict(note)
                item["track"] = track.get("id", "lead")
                item["beat"] = clip_start + float(note.get("beat", 0.0))
                notes.append(item)
    return notes


def add_manual_note_to_score(score: dict[str, Any], note: dict[str, Any]) -> None:
    if score.get("schema_version") == 1:
        score.setdefault("manual_notes", []).append(dict(note))
        return
    track_id = str(note.get("track", "lead"))
    track = _v2_track(score, track_id)
    if track is None:
        raise ValueError(f"unknown track {track_id!r}")
    clips = track.setdefault("clips", [])
    clip = next((item for item in clips if item.get("id") == f"clip-manual-{track_id}"), None)
    if clip is None:
        clip = {
            "id": f"clip-manual-{track_id}",
            "start_beat": 0.0,
            "length_beats": float(score["transport"]["bars"] * 4),
            "notes": [],
        }
        clips.append(clip)
    existing_ids = {
        str(existing.get("id"))
        for item in clips for existing in item.get("notes", [])
    }
    serial = sum(len(item.get("notes", [])) for item in clips) + 1
    while f"manual-{track_id}-{serial}" in existing_ids:
        serial += 1
    clip["notes"].append({
        "id": f"manual-{track_id}-{serial}",
        "beat": float(note["beat"]),
        "duration": float(note.get("duration", 0.5)),
        "pitch": str(note["pitch"]),
        "velocity": float(note.get("velocity", 0.7)),
        "probability": 1.0,
    })


def remove_manual_note_from_score(score: dict[str, Any], track_id: str, beat: float, pitch: str) -> bool:
    if score.get("schema_version") == 1:
        notes = score.get("manual_notes", [])
        target = next((index for index, note in enumerate(notes)
                       if note.get("track") == track_id
                       and abs(float(note.get("beat", -99)) - beat) < .51
                       and note.get("pitch") == pitch), None)
        if target is None:
            return False
        notes.pop(target)
        return True
    track = _v2_track(score, track_id)
    if track is None:
        return False
    for clip in track.get("clips", []):
        clip_start = float(clip.get("start_beat", 0.0))
        notes = clip.get("notes", [])
        target = next((index for index, note in enumerate(notes)
                       if abs(clip_start + float(note.get("beat", -99)) - beat) < .51
                       and note.get("pitch") == pitch), None)
        if target is not None:
            notes.pop(target)
            return True
    return False


def remove_manual_notes_at_onset(score: dict[str, Any], track_id: str, beat: float) -> int:
    """Remove pitched editor notes at one onset (used by monophonic roles)."""
    removed = 0
    if score.get("schema_version") == 1:
        notes = score.get("manual_notes", [])
        kept = [
            note for note in notes
            if not (
                note.get("track") == track_id
                and abs(float(note.get("beat", -99)) - beat) <= 1e-6
            )
        ]
        removed = len(notes) - len(kept)
        score["manual_notes"] = kept
        return removed
    track = _v2_track(score, track_id)
    if track is None:
        return 0
    for clip in track.get("clips", []):
        clip_start = float(clip.get("start_beat", 0.0))
        notes = clip.get("notes", [])
        kept = [
            note for note in notes
            if abs(clip_start + float(note.get("beat", -99)) - beat) > 1e-6
        ]
        removed += len(notes) - len(kept)
        clip["notes"] = kept
    return removed


def note_to_midi(note: str) -> int:
    match = NOTE_RE.match(str(note).strip())
    if not match:
        raise ValueError(f"invalid note {note!r}; use names such as D4 or Bb3")
    name = match.group(1).upper() + match.group(2)
    if name not in NOTE_PC:
        raise ValueError(f"unsupported note spelling {note!r}")
    return (int(match.group(3)) + 1) * 12 + NOTE_PC[name]


def midi_to_note(midi: int) -> str:
    return f"{PC_NAME[midi % 12]}{midi // 12 - 1}"


def root_pc(text: str) -> int:
    root, _quality, _intervals = court_harmony.parse_chord(text)
    return root


def chord_midis(chord: str, octave: int) -> list[int]:
    root_pc_value, _quality, intervals = court_harmony.parse_chord(chord)
    root = (octave + 1) * 12 + root_pc_value
    return [root + interval for interval in intervals]


def _voice_chord_near(chord: str, previous: list[int] | None = None) -> list[int]:
    """Choose a smooth, spread three-note upper structure.

    Legal chord tones are not automatically pleasant when held as an adjacent
    semitone cluster.  Bass already supplies the root, so colored chords use
    guide-tone shells and the optimizer rejects gaps smaller than a minor
    third whenever a spread inversion exists.
    """
    import itertools

    root, _quality, _full_intervals = court_harmony.parse_chord(chord)
    intervals = court_harmony.chord_guide_intervals(chord)
    wanted = {(root + interval) % 12 for interval in intervals}
    available = [midi for midi in range(52, 77) if midi % 12 in wanted]
    choices = [
        list(combo) for combo in itertools.combinations(available, 3)
        if {midi % 12 for midi in combo} == wanted
    ]
    if not choices:
        return chord_midis(chord, 3)[:3]
    spread = [
        combo for combo in choices
        if min(right - left for left, right in zip(combo, combo[1:])) >= 3
        and combo[-1] - combo[0] <= 17
    ]
    if spread:
        choices = spread
    target = previous if previous and len(previous) == 3 else [55, 60, 64]
    return min(
        choices,
        key=lambda combo: (
            sum(abs(combo[index] - target[index]) for index in range(3)),
            max(abs(combo[index] - target[index]) for index in range(3)),
            max(0, combo[-1] - combo[0] - 16) * 3,
            abs(sum(combo) / 3.0 - 61.0),
            combo,
        ),
    )


def style_defaults(style: str) -> dict[str, Any]:
    if style == "spicy_lofi":
        return {
            "tempo": 88, "tonic": "E", "mode": "natural_minor",
            "motif": ["B4", "E5", "G5", "B5", "A5", "G5", "E5", "B4"],
            "harmony": ["Em9", "Cmaj9", "Am9", "B7", "Em9", "Am9", "B7", "Em"], "swing": 0.10,
            "sections": [
                {"name": "dusk", "bars": 8, "energy": .32, "transform": "fragment"},
                {"name": "pocket", "bars": 8, "energy": .50, "transform": "forward"},
                {"name": "spice", "bars": 8, "energy": .70, "transform": "call_response"},
                {"name": "breath", "bars": 8, "energy": .30, "transform": "reverse"},
                {"name": "midnight_walk", "bars": 8, "energy": .58, "transform": "sequence"},
                {"name": "ember_bloom", "bars": 8, "energy": .82, "transform": "call_response"},
                {"name": "lantern_return", "bars": 8, "energy": .60, "transform": "recombine"},
                {"name": "afterglow", "bars": 8, "energy": .38, "transform": "fragment"},
            ],
            "mix": {"master_gain": .78, "width": .78, "reverb": .28, "delay": .22},
        }
    if style == "court_experimental":
        return {
            "tempo": 104, "tonic": "D", "mode": "dorian",
            "motif": ["D5", "F5", "A5", "E5", "C5", "D5", "G5", "F5"],
            "harmony": ["Dm", "G", "Cmaj7", "A7", "Dm", "G", "A7", "Dm"], "swing": 0.03,
            "sections": [
                {"name": "seed", "bars": 8, "energy": .25, "transform": "fragment"},
                {"name": "lattice", "bars": 8, "energy": .52, "transform": "sequence"},
                {"name": "fracture", "bars": 8, "energy": .74, "transform": "reverse"},
                {"name": "void", "bars": 8, "energy": .22, "transform": "fragment"},
                {"name": "bloom", "bars": 8, "energy": 1.0, "transform": "call_response"},
                {"name": "orbit", "bars": 8, "energy": .62, "transform": "forward"},
                {"name": "convergence", "bars": 8, "energy": .84, "transform": "sequence"},
                {"name": "residue", "bars": 8, "energy": .42, "transform": "recombine"},
            ],
            "mix": {"master_gain": .80, "width": .90, "reverb": .26, "delay": .24},
        }
    return {
        "tempo": 112, "tonic": "D", "mode": "natural_minor",
        "motif": ["A4", "F4", "E4", "D4", "A3", "D4", "F4", "A4"],
        "harmony": ["Dm", "Bb", "Gm", "A7", "Dm", "Bb", "A7", "Dm"], "swing": 0.025,
        "sections": [
            {"name": "prologue", "bars": 8, "energy": .22, "transform": "fragment"},
            {"name": "road", "bars": 8, "energy": .44, "transform": "forward"},
            {"name": "encounter", "bars": 8, "energy": .64, "transform": "call_response"},
            {"name": "hush", "bars": 8, "energy": .28, "transform": "fragment"},
            {"name": "rally", "bars": 8, "energy": .60, "transform": "reverse"},
            {"name": "ascent", "bars": 8, "energy": .96, "transform": "sequence"},
            {"name": "home", "bars": 8, "energy": .50, "transform": "recombine"},
            {"name": "epilogue", "bars": 8, "energy": .34, "transform": "fragment"},
        ],
        "mix": {"master_gain": .82, "width": .68, "reverb": .15, "delay": .12},
    }


def default_score(style: str = "retro_adventure") -> dict[str, Any]:
    base = style_defaults(style)
    bars = sum(int(section["bars"]) for section in base["sections"])
    return {
        "schema_version": 1,
        "project_id": "court-synth-default",
        "revision": 1,
        "title": "Vaultlight Procession",
        "style": style,
        "seed": 48358,
        "transport": {"bpm": base["tempo"], "meter": [4, 4], "bars": bars, "swing": base["swing"], "loop": True},
        "harmony": {"tonic": base["tonic"], "mode": base["mode"], "chords": base["harmony"]},
        "motif": base["motif"],
        "sections": base["sections"],
        "mix": base["mix"],
        "manual_notes": [],
        "lineage": {"source": "court_synth_default", "preserve": ["tonal_center", "main_motif"]},
    }


def ensure_score_file(path: Path = DEFAULT_SCORE_PATH) -> Path:
    if not path.exists():
        _atomic_json(path, default_score())
    return path


def read_score(path: Path) -> dict[str, Any]:
    ensure_score_file(path)
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        raise ValueError(f"invalid JSON in {path.name}: {exc}") from exc
    if not isinstance(value, dict):
        raise ValueError("CourtScore root must be a JSON object")
    if value.get("schema_version") not in {1, 2}:
        raise ValueError(f"unsupported CourtScore schema {value.get('schema_version')!r}")
    return value


def score_duration_seconds(score: dict[str, Any]) -> float:
    """Return the declared musical form length, excluding export-only tails."""
    transport = score.get("transport", {})
    bpm = transport.get("bpm")
    bars = transport.get("bars")
    meter = transport.get("meter", [4, 4])
    if (
        not isinstance(bpm, (int, float))
        or isinstance(bpm, bool)
        or float(bpm) <= 0
        or not isinstance(bars, int)
        or isinstance(bars, bool)
        or bars <= 0
        or not isinstance(meter, list)
        or len(meter) != 2
        or not isinstance(meter[0], int)
        or meter[0] <= 0
    ):
        return 0.0
    return float(bars) * float(meter[0]) * 60.0 / float(bpm)


def _duration_errors(score: dict[str, Any]) -> list[str]:
    duration = score_duration_seconds(score)
    if duration and duration + 1e-6 < MIN_COMPOSITION_SECONDS:
        return [
            "CourtScore form must last at least "
            f"{MIN_COMPOSITION_SECONDS:.0f} seconds at its declared tempo "
            f"(currently {duration:.3f}s)"
        ]
    return []


def validate_score(score: dict[str, Any]) -> list[str]:
    ver = score.get("schema_version")
    if str(ver) == "3.0":
        errors = court_schema.validate_v3(score)
        if not errors:
            errors.extend(court_harmony.grade_score(score, manual_notes(score))["issues"])
            errors.extend(_duration_errors(score))
        return list(dict.fromkeys(errors))
    if ver == 2:
        # Connect to new schema module for v2 validation
        errors = court_schema.validate_v2(score)
        if not errors:
            errors.extend(court_harmony.grade_score(score, manual_notes(score))["issues"])
            errors.extend(_duration_errors(score))
        return list(dict.fromkeys(errors))
    # v1 legacy path (kept during transition)
    errors: list[str] = []
    if ver != 1:
        errors.append("schema_version must be 1 (or 2 via new modules)")
    if not isinstance(score.get("project_id"), str) or len(score["project_id"].strip()) < 3:
        errors.append("project_id must be a non-empty identifier")
    if not isinstance(score.get("title"), str) or not score["title"].strip():
        errors.append("title is required")
    if score.get("style") not in STYLES:
        errors.append(f"style must be one of {', '.join(sorted(STYLES))}")
    transport = score.get("transport")
    if not isinstance(transport, dict):
        errors.append("transport object is required")
        transport = {}
    bpm = transport.get("bpm")
    if not isinstance(bpm, (int, float)) or not 55 <= bpm <= 170:
        errors.append("transport.bpm must be 55..170")
    elif score.get("style") in STYLE_TEMPO_RANGES:
        low_bpm, high_bpm = STYLE_TEMPO_RANGES[score["style"]]
        if not low_bpm <= bpm <= high_bpm:
            errors.append(
                f"{score['style']} tempo must stay inside its {low_bpm}..{high_bpm} BPM pocket"
            )
    bars = transport.get("bars")
    if not isinstance(bars, int) or not 16 <= bars <= 128:
        errors.append("transport.bars must be an integer from 16..128")
    elif bars % 4:
        errors.append("transport.bars must align to the four-bar phrase clock")
    if transport.get("meter") != [4, 4]:
        errors.append("transport.meter must currently be [4, 4]")
    if not isinstance(transport.get("swing", 0), (int, float)) or not 0 <= transport.get("swing", 0) <= 0.20:
        errors.append("transport.swing must be 0..0.20")
    harmony = score.get("harmony")
    if not isinstance(harmony, dict):
        errors.append("harmony object is required")
        harmony = {}
    tonic = harmony.get("tonic")
    if tonic not in NOTE_PC:
        errors.append("harmony.tonic must be a pitch class such as D or Bb")
    if harmony.get("mode") not in MODES:
        errors.append(f"harmony.mode must be one of {', '.join(sorted(MODES))}")
    chords = harmony.get("chords")
    if not isinstance(chords, list) or len(chords) not in {4, 8}:
        errors.append("harmony.chords must contain one 4-bar phrase or 8-bar period")
    else:
        for chord in chords:
            try:
                chord_midis(str(chord), 3)
            except ValueError as exc:
                errors.append(str(exc))
    motif = score.get("motif")
    if not isinstance(motif, list) or not 4 <= len(motif) <= 12:
        errors.append("motif must contain 4..12 note names")
    else:
        for note in motif:
            try:
                note_to_midi(str(note))
            except ValueError as exc:
                errors.append(str(exc))
    sections = score.get("sections")
    if not isinstance(sections, list) or not 4 <= len(sections) <= 8:
        errors.append("sections must contain 4..8 named movements")
    else:
        names = set()
        total = 0
        for section in sections:
            if not isinstance(section, dict):
                errors.append("every section must be an object")
                continue
            name = str(section.get("name", "")).strip().lower()
            if not name or name in names:
                errors.append("section names must be non-empty and distinct")
            names.add(name)
            count = section.get("bars")
            if not isinstance(count, int) or count < 4 or count % 4:
                errors.append("every section must contain aligned four/eight-bar phrases")
            else:
                total += count
            energy = section.get("energy")
            if not isinstance(energy, (int, float)) or not 0.05 <= energy <= 1.0:
                errors.append("section energy must be 0.05..1.0")
            if section.get("transform") not in {"fragment", "forward", "reverse", "sequence", "recombine", "call_response"}:
                errors.append("section transform must be fragment, forward, reverse, sequence, recombine, or call_response")
        if isinstance(bars, int) and total != bars:
            errors.append(f"section bars ({total}) must equal transport.bars ({bars})")
    mix = score.get("mix", {})
    if not isinstance(mix, dict):
        errors.append("mix must be an object")
    elif not all(isinstance(mix.get(key, default), (int, float)) and 0 <= mix.get(key, default) <= 1
                 for key, default in (("master_gain", .82), ("width", .72), ("reverb", .22), ("delay", .18))):
        errors.append("mix values must be 0..1")
    track_mix = score.get("track_mix", {})
    if not isinstance(track_mix, dict):
        errors.append("track_mix must be an object")
    else:
        for track_id, values in track_mix.items():
            if track_id not in TRACK_BY_ID or not isinstance(values, dict):
                errors.append("track_mix entries require a known track and object value")
                continue
            if not isinstance(values.get("volume", TRACK_BY_ID[track_id]["volume"]), (int, float)) or not 0 <= float(values.get("volume", 0)) <= 1.25:
                errors.append(f"track_mix.{track_id}.volume must be 0..1.25")
            if not isinstance(values.get("pan", TRACK_BY_ID[track_id]["pan"]), (int, float)) or not -1 <= float(values.get("pan", 0)) <= 1:
                errors.append(f"track_mix.{track_id}.pan must be -1..1")
            for flag in ("mute", "solo", "arm"):
                if flag in values and not isinstance(values[flag], bool):
                    errors.append(f"track_mix.{track_id}.{flag} must be boolean")
    instrumentation = score.get("instrumentation", {})
    if not isinstance(instrumentation, dict):
        errors.append("instrumentation must be an object")
    else:
        for track_id, spec in instrumentation.items():
            if track_id not in TRACK_BY_ID or not isinstance(spec, dict):
                errors.append("instrumentation entries require a known track and object value")
                continue
            patch_id = spec.get("patch_id")
            patch = court_instruments.REGISTRY.get(str(patch_id))
            if patch is None:
                errors.append(f"instrumentation.{track_id}.patch_id is not registered")
            elif patch.family not in V1_PATCH_FAMILIES[track_id]:
                errors.append(f"instrumentation.{track_id} cannot use the {patch.family} family")
            macros = spec.get("macros", {})
            if not isinstance(macros, dict) or any(
                isinstance(value, bool) or not isinstance(value, (int, float)) or not 0 <= value <= 1
                for value in macros.values()
            ):
                errors.append(f"instrumentation.{track_id}.macros must contain numeric 0..1 values")
    manual = score.get("manual_notes", [])
    if not isinstance(manual, list):
        errors.append("manual_notes must be a list")
    else:
        for item in manual[:512]:
            if not isinstance(item, dict) or item.get("track") not in TRACK_BY_ID:
                errors.append("manual notes require a known track")
                continue
            try:
                note_to_midi(str(item.get("pitch", "")))
            except ValueError as exc:
                errors.append(str(exc))
            if not isinstance(item.get("beat"), (int, float)) or item["beat"] < 0:
                errors.append("manual note beat must be non-negative")
            if not isinstance(item.get("duration"), (int, float)) or not 0.05 <= item["duration"] <= 16:
                errors.append("manual note duration must be 0.05..16 beats")
    if not errors:
        errors.extend(court_harmony.grade_score(score, manual)["issues"])
    if (
        isinstance(bars, int)
        and not isinstance(bars, bool)
        and bars > 0
        and isinstance(bpm, (int, float))
        and not isinstance(bpm, bool)
        and bpm > 0
    ):
        errors.extend(_duration_errors(score))
    return list(dict.fromkeys(errors))


def _section_offsets(sections: Iterable[dict[str, Any]]) -> Iterable[tuple[dict[str, Any], int]]:
    bar = 0
    for section in sections:
        yield section, bar
        bar += int(section["bars"])


def _motif_for_transform(motif: list[int], transform: str, phrase: int, scale: list[int]) -> list[int]:
    if transform == "fragment":
        return motif[: max(4, len(motif) // 2)]
    if transform == "reverse":
        return list(reversed(motif))
    if transform == "sequence":
        transposed = [note + (2 if phrase % 2 == 0 else -3) for note in motif]
        return [_nearest_scale(note, scale) for note in transposed]
    if transform == "recombine":
        half = len(motif) // 2
        return list(reversed(motif[:half])) + motif[half:]
    if transform == "call_response":
        half = len(motif) // 2
        return motif[:half] + [_nearest_scale(note - 5, scale) for note in reversed(motif[half:])]
    return motif


def _nearest_scale(midi: int, scale: list[int]) -> int:
    choices = [value for value in scale if abs(value - midi) <= 12]
    return min(choices or scale, key=lambda value: abs(value - midi))


def _scale_notes(tonic: str, mode: str, low: int, high: int) -> list[int]:
    pcs = {(NOTE_PC[tonic] + interval) % 12 for interval in MODES[mode]}
    return [midi for midi in range(low, high + 1) if midi % 12 in pcs]


PHRASE_BAR_ROLES = ("statement", "answer", "variation", "cadence")

# These cells are deliberately different musical grammars, not cosmetic
# presets.  The score remains compact (form, harmony, motif, energy), while the
# arranger expands that intent into idiomatic four-bar cause-and-effect.
DRUM_CELLS: dict[str, dict[str, dict[str, tuple[float, ...]]]] = {
    "retro_adventure": {
        "statement": {"kick": (0.0, 2.0), "snare": (1.0, 3.0), "hat": (.5, 1.5, 2.5, 3.5)},
        "answer": {"kick": (0.0, 2.0), "snare": (1.0, 3.0), "hat": (.5, 1.5, 2.5, 3.5)},
        "variation": {"kick": (0.0, 2.0), "snare": (1.0, 3.0), "hat": (.5, 1.5, 2.5, 3.5)},
        "cadence": {"kick": (0.0, 2.0, 3.5), "snare": (1.0, 3.0), "hat": (.5, 1.5, 2.5, 3.5, 3.75)},
    },
    "spicy_lofi": {
        "statement": {"kick": (0.0, 2.5), "snare": (1.06, 3.06), "hat": (.5, 1.5, 2.5, 3.5)},
        "answer": {"kick": (0.0, 2.5), "snare": (1.06, 3.06), "hat": (.5, 1.5, 2.5, 3.5)},
        "variation": {"kick": (0.0, 2.5), "snare": (1.06, 3.06), "hat": (.5, 1.5, 2.5, 3.5)},
        "cadence": {"kick": (0.0, 2.5, 3.5), "snare": (1.06, 3.06), "hat": (.5, 1.5, 2.5, 3.5)},
    },
    "court_experimental": {
        "statement": {"kick": (0.0, 2.0), "snare": (1.0, 3.0), "hat": (.5, 1.5, 2.5, 3.5)},
        "answer": {"kick": (0.0, 2.0), "snare": (1.0, 3.0), "hat": (.5, 1.5, 2.5, 3.5)},
        "variation": {"kick": (0.0, 2.0), "snare": (1.0, 3.0), "hat": (.5, 1.5, 2.5, 3.5)},
        "cadence": {"kick": (0.0, 2.0, 3.25), "snare": (1.0, 3.0, 3.5), "hat": (.5, 1.5, 2.5, 3.5)},
    },
}

BASS_CELLS: dict[str, dict[str, tuple[tuple[float, int, float, float], ...]]] = {
    "retro_adventure": {
        "statement": ((0.0, 0, .88, 1.0), (2.0, 7, .68, .84)),
        "answer": ((0.0, 0, .88, 1.0), (2.0, 7, .68, .84)),
        "variation": ((0.0, 0, .88, 1.0), (2.0, 7, .68, .84)),
        "cadence": ((0.0, 0, 1.08, 1.0), (2.0, 7, .62, .82), (3.5, 12, .34, .72)),
    },
    "spicy_lofi": {
        "statement": ((.04, 0, 1.08, .96), (2.54, 7, .66, .76)),
        "answer": ((.04, 0, 1.08, .96), (2.54, 7, .66, .76)),
        "variation": ((.04, 0, 1.08, .96), (2.54, 7, .66, .76)),
        "cadence": ((.04, 0, 1.18, .96), (2.54, 7, .58, .74), (3.5, 12, .30, .68)),
    },
    "court_experimental": {
        "statement": ((0.0, 0, .86, 1.0), (2.0, 7, .64, .78)),
        "answer": ((0.0, 0, .86, 1.0), (2.0, 7, .64, .78)),
        "variation": ((0.0, 0, .86, 1.0), (2.0, 7, .64, .78)),
        "cadence": ((0.0, 0, 1.04, 1.0), (2.0, 7, .58, .76), (3.25, 12, .38, .72)),
    },
}

COMP_CELLS: dict[str, dict[str, tuple[tuple[float, float, float], ...]]] = {
    "retro_adventure": {
        "statement": ((0.0, 1.52, .94), (2.0, 1.42, .82)),
        "answer": ((0.0, 1.52, .94), (2.0, 1.42, .82)),
        "variation": ((0.0, 1.52, .94), (2.0, 1.42, .82)),
        "cadence": ((0.0, 3.42, .96),),
    },
    "spicy_lofi": {
        "statement": ((0.0, 1.34, .88), (2.5, 1.02, .78)),
        "answer": ((0.0, 1.34, .88), (2.5, 1.02, .78)),
        "variation": ((0.0, 1.34, .88), (2.5, 1.02, .78)),
        "cadence": ((0.0, 3.42, .90),),
    },
    "court_experimental": {
        "statement": ((0.0, 1.18, .90), (2.0, 1.18, .78)),
        "answer": ((0.0, 1.18, .90), (2.0, 1.18, .78)),
        "variation": ((0.0, 1.18, .90), (2.0, 1.18, .78)),
        "cadence": ((0.0, 3.32, .92),),
    },
}

PLUCK_RHYTHMS: dict[str, dict[str, tuple[float, ...]]] = {
    "retro_adventure": {
        "statement": (.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5),
        "answer": (.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5),
        "variation": (.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5),
        "cadence": (.5, 1.0, 1.5, 2.5, 3.0),
    },
    "spicy_lofi": {
        "statement": (.5, 1.5, 2.5, 3.5),
        "answer": (.5, 1.5, 2.5, 3.5),
        "variation": (.5, 1.5, 2.5, 3.5),
        "cadence": (.5, 1.5, 2.5),
    },
    "court_experimental": {
        "statement": (.5, 1.25, 2.0, 2.75, 3.5),
        "answer": (.5, 1.25, 2.0, 2.75, 3.5),
        "variation": (.5, 1.25, 2.0, 2.75, 3.5),
        "cadence": (.5, 1.5, 2.5, 3.5),
    },
}

LEAD_RHYTHMS: dict[str, dict[str, tuple[tuple[float, float], ...]]] = {
    "retro_adventure": {
        "statement": ((.5, .32), (1.0, .32), (1.5, .32), (2.5, .32), (3.0, .58)),
        "answer": ((.5, .28), (1.0, .32), (1.5, .45), (2.5, .32), (3.0, .50)),
        "variation": ((.5, .24), (1.0, .30), (1.5, .38), (2.5, .34), (3.0, .46)),
        "cadence": ((.5, .30), (1.0, .30), (1.5, .42), (2.5, .38), (3.0, .68)),
    },
    "spicy_lofi": {
        "statement": ((.5, .55), (1.5, .40), (2.5, .55), (3.5, .25)),
        "answer": ((.5, .40), (1.5, .60), (2.5, .40), (3.5, .25)),
        "variation": ((.5, .30), (1.5, .45), (2.5, .65), (3.5, .25)),
        "cadence": ((.5, .45), (1.5, .55), (2.5, .45), (3.25, .60)),
    },
    "court_experimental": {
        "statement": ((.5, .28), (1.25, .42), (2.0, .30), (2.75, .52), (3.5, .28)),
        "answer": ((.5, .30), (1.25, .48), (2.0, .28), (2.75, .40), (3.5, .28)),
        "variation": ((.5, .24), (1.25, .34), (2.0, .52), (2.75, .28), (3.5, .28)),
        "cadence": ((.5, .32), (1.25, .36), (2.0, .50), (2.75, .44), (3.5, .28)),
    },
}


def _phrase_bar_role(local_bar: int, section_bars: int) -> str:
    del section_bars
    return PHRASE_BAR_ROLES[local_bar % len(PHRASE_BAR_ROLES)]


def _swung_offset(offset: float, swing: float, style: str) -> float:
    """Delay only off-eighth subdivisions; never shift the whole part."""
    eighth = int(round(offset * 2.0))
    if abs(offset * 2.0 - eighth) <= .04 and eighth % 2 == 1:
        depth = 1.0 if style == "spicy_lofi" else .55
        return offset + swing * depth
    return offset


def _developed_motif(
    motif: list[int], transform: str, phrase_index: int, role: str, scale: list[int]
) -> list[int]:
    base = _motif_for_transform(motif, transform, phrase_index, scale)
    if not base:
        return []
    if role == "answer":
        pivot = max(1, len(base) // 2)
        answer = base[pivot:] + base[:pivot]
        return [_nearest_scale(note - 5, scale) for note in reversed(answer)]
    if role == "variation":
        rotation = (phrase_index + 2) % len(base)
        varied = base[rotation:] + base[:rotation]
        peak = max(range(len(varied)), key=lambda index: varied[index])
        varied[peak] = _nearest_scale(varied[peak] + 12, scale)
        return varied
    if role == "cadence":
        tail = list(reversed(base[-max(2, len(base) // 2):]))
        return base[:2] + tail
    return base


def _voice_melody_phrase(
    score: dict[str, Any],
    bar_beat: float,
    phrase: list[int],
    rhythm: list[tuple[float, float]],
    scale: list[int],
    *,
    previous_note: int | None = None,
    previous_interval: int | None = None,
) -> list[int]:
    """Fit a motif phrase to harmony without destroying its contour.

    Weak-beat color tones are permitted only as prepared and resolved
    stepwise neighbors.  A tiny deterministic dynamic program then balances
    motif identity, register, bounded intervals, and post-leap reversal.  This
    prevents the former ``+12 then generic snap`` path from turning phrase
    peaks into unrelated octave-plus jumps.
    """
    if not phrase or not rhythm:
        return []
    low, high = court_harmony.track_pitch_range("lead")
    chord = court_harmony.chord_at_beat(score, bar_beat)
    chord_pcs = court_harmony.chord_pitch_classes(chord) if chord else set()
    scale_pcs = {note % 12 for note in scale}
    chord_notes = [note for note in range(low, high + 1) if note % 12 in chord_pcs]
    color_notes = [
        note for note in range(low, high + 1)
        if note % 12 in scale_pcs | chord_pcs
    ]
    targets = [phrase[index % len(phrase)] for index in range(len(rhythm))]

    def target_cost(candidate: int, target: int) -> float:
        equivalents = [
            target + octave * 12 for octave in range(-5, 6)
            if low <= target + octave * 12 <= high
        ]
        distance = min((abs(candidate - value) for value in equivalents), default=abs(candidate - target))
        pc_distance = abs((candidate - target) % 12)
        pc_distance = min(pc_distance, 12 - pc_distance)
        return distance * .34 + pc_distance * .62 + abs(candidate - 70) * .018

    prior_previous = (
        previous_note - previous_interval
        if previous_note is not None and previous_interval is not None
        else None
    )
    states: dict[tuple[int | None, int | None], tuple[float, list[int]]] = {
        (prior_previous, previous_note): (0.0, [])
    }
    for index, (offset, duration) in enumerate(rhythm):
        is_anchor = (
            index in {0, len(rhythm) - 1}
            or duration >= .50
            or (
                abs(offset - round(offset)) <= .06
                and int(round(offset)) % 2 == 0
            )
        )
        candidates = chord_notes if is_anchor else color_notes
        next_states: dict[tuple[int | None, int], tuple[float, list[int]]] = {}
        for (before_previous, previous), (cost, path) in states.items():
            for candidate in candidates:
                interval = candidate - previous if previous is not None else 0
                if previous is not None and abs(interval) > 8:
                    continue
                previous_is_chord = previous is None or previous % 12 in chord_pcs
                candidate_is_chord = candidate % 12 in chord_pcs
                # A color tone is a neighbor, not a destination or a second
                # consecutive outside note.  The next DP step enforces its
                # resolution with the same hard rule.
                if previous is not None and not previous_is_chord:
                    if not candidate_is_chord or abs(interval) > 3:
                        continue
                if not candidate_is_chord:
                    if previous is None or not previous_is_chord or abs(interval) > 3:
                        continue

                candidate_cost = cost + target_cost(candidate, targets[index])
                if previous is not None:
                    candidate_cost += abs(interval) * .11
                    if interval == 0 and index and targets[index] != targets[index - 1]:
                        candidate_cost += .9
                    target_motion = targets[index] - targets[index - 1] if index else 0
                    if target_motion and interval and (target_motion > 0) != (interval > 0):
                        candidate_cost += 1.25
                if before_previous is not None and previous is not None:
                    previous_motion = previous - before_previous
                    if abs(previous_motion) >= 5:
                        if not interval or previous_motion * interval >= 0:
                            candidate_cost += 24.0
                        if abs(interval) > 4:
                            candidate_cost += 12.0 + 2.0 * (abs(interval) - 4)
                key = (previous, candidate)
                incumbent = next_states.get(key)
                value = (candidate_cost, path + [candidate])
                if incumbent is None or value[0] < incumbent[0]:
                    next_states[key] = value
        if not next_states:
            # Defensive fallback: a legal chord-tone line is always preferable
            # to emitting an unbounded or unresolved target.
            previous = min(
                (state[1] for state in states if state[1] is not None),
                key=lambda note: abs(note - targets[index]),
                default=None,
            )
            candidate = min(
                chord_notes,
                key=lambda note: (
                    abs(note - previous) if previous is not None else 0,
                    target_cost(note, targets[index]),
                ),
            )
            best_cost, best_path = min(states.values(), key=lambda item: item[0])
            next_states[(previous, candidate)] = (best_cost + target_cost(candidate, targets[index]), best_path + [candidate])
        states = next_states
    return min(states.values(), key=lambda item: (item[0], item[1]))[1]


def _revoice_generated_lead_around_authored(
    score: dict[str, Any],
    events: list[NoteEvent],
    *,
    lock_authored: bool = True,
) -> list[NoteEvent]:
    """Octave-fold generated melody around immutable piano-roll anchors.

    Manual notes replace generated mono spans later in compilation.  Without a
    second pass, removing one generated note can expose an octave jump between
    its authored replacement and the next surviving generated note.  This DP
    keeps authored pitches fixed, preserves every generated pitch class, and
    chooses only register-equivalent generated notes that retain the melodic
    close-succession and leap-recovery contract.
    """
    ordered = sorted(
        [event for event in events if event.track == "lead" and event.midi is not None],
        key=lambda event: (event.start, not event.authored),
    )
    if len(ordered) < 2 or not any(event.authored for event in ordered):
        return events

    low, high = court_harmony.track_pitch_range("lead")

    def candidates(event: NoteEvent) -> list[int]:
        original = int(event.midi)
        if event.authored and lock_authored:
            return [original]
        if event.authored and not lock_authored:
            active_chord = court_harmony.chord_at_beat(score, event.start)
            chord_pcs = (
                court_harmony.chord_pitch_classes(active_chord)
                if active_chord else {original % 12}
            )
            return [midi for midi in range(low, high + 1) if midi % 12 in chord_pcs]
        return [midi for midi in range(low, high + 1) if midi % 12 == original % 12]

    def register_cost(event: NoteEvent, midi: int) -> float:
        weight = 2.4 if event.authored else 1.0
        pitch_class_change = (
            .65 if event.authored and midi % 12 != int(event.midi) % 12 else 0.0
        )
        return weight * abs(midi - int(event.midi)) / 12.0 + pitch_class_change

    def solve(chain: list[NoteEvent], *, require_recovery: bool) -> list[int] | None:
        first = chain[0]
        states: dict[tuple[int, int | None], tuple[float, list[int]]] = {
            (midi, None): (register_cost(first, midi), [midi])
            for midi in candidates(first)
        }
        for event in chain[1:]:
            next_states: dict[tuple[int, int], tuple[float, list[int]]] = {}
            original = int(event.midi)
            for midi in candidates(event):
                for (previous, previous_interval), (cost, path) in states.items():
                    interval = midi - previous
                    if abs(interval) > 8:
                        continue
                    if (
                        require_recovery
                        and previous_interval is not None
                        and abs(previous_interval) >= 5
                        and not (previous_interval * interval < 0 and abs(interval) <= 4)
                    ):
                        continue
                    candidate_cost = (
                        cost
                        + register_cost(event, midi)
                        + abs(interval) * .055
                        + (1.2 if abs(interval) >= 7 else 0.0)
                    )
                    key = (midi, interval)
                    incumbent = next_states.get(key)
                    value = (candidate_cost, path + [midi])
                    if incumbent is None or value < incumbent:
                        next_states[key] = value
            if not next_states:
                return None
            states = next_states
        return min(states.values(), key=lambda item: (item[0], item[1]))[1]

    chains: list[list[NoteEvent]] = []
    for event in ordered:
        if not chains or event.start - chains[-1][-1].start > 1.75:
            chains.append([event])
        else:
            chains[-1].append(event)

    replacements: dict[int, int] = {}
    for chain in chains:
        if not any(event.authored for event in chain):
            continue
        voiced = solve(chain, require_recovery=True)
        if voiced is None:
            voiced = solve(chain, require_recovery=False)
        if voiced is None:
            # Impossible fixed anchors remain audible and the combined-lead
            # quality gate reports the exact failure; protected edits are not
            # silently moved or deleted.
            continue
        for event, midi in zip(chain, voiced):
            if not event.authored or not lock_authored:
                replacements[id(event)] = midi
    return [
        replace(event, midi=replacements[id(event)])
        if id(event) in replacements else event
        for event in events
    ]


def _repair_generated_colors_around_authored(
    score: dict[str, Any],
    events: list[NoteEvent],
) -> list[NoteEvent]:
    """Repair a passing tone whose preparation was displaced by an edit.

    The melody generator deliberately permits prepared and resolved color
    tones.  Replacing its neighbor with an authored piano-roll note can remove
    that preparation even after the register pass succeeds.  Only an
    *unresolved generated* note directly beside an authored anchor is eligible
    here; authored pitches and valid generated colors remain untouched.
    """
    result = list(events)
    low, high = court_harmony.track_pitch_range("lead")

    def lead_chain() -> list[tuple[int, NoteEvent]]:
        return sorted(
            [
                (index, event)
                for index, event in enumerate(result)
                if event.track == "lead" and event.midi is not None
            ],
            key=lambda item: item[1].start,
        )

    def unresolved_at(chain: list[tuple[int, NoteEvent]], index: int) -> bool:
        event = chain[index][1]
        chord = court_harmony.chord_at_beat(score, event.start)
        chord_pcs = court_harmony.chord_pitch_classes(chord) if chord else set()
        if int(event.midi) % 12 in chord_pcs:
            return False
        previous = chain[index - 1][1] if index else None
        following = chain[index + 1][1] if index + 1 < len(chain) else None
        prepared = (
            previous is not None
            and event.start - previous.start <= 1.75
            and int(previous.midi) % 12 in chord_pcs
            and abs(int(event.midi) - int(previous.midi)) <= 3
        )
        following_chord = (
            court_harmony.chord_at_beat(score, following.start)
            if following is not None else None
        )
        following_pcs = (
            court_harmony.chord_pitch_classes(following_chord)
            if following_chord else set()
        )
        resolved = (
            following is not None
            and following.start - event.start <= 1.75
            and int(following.midi) % 12 in following_pcs
            and abs(int(following.midi) - int(event.midi)) <= 3
        )
        return not (prepared and resolved)

    eligible_limit = sum(
        event.track == "lead" and not event.authored and event.midi is not None
        for event in result
    )
    for _pass in range(eligible_limit):
        chain = lead_chain()
        changed = False
        for chain_index, (event_index, event) in enumerate(chain):
            if event.authored or not unresolved_at(chain, chain_index):
                continue
            previous = chain[chain_index - 1][1] if chain_index else None
            following = (
                chain[chain_index + 1][1]
                if chain_index + 1 < len(chain) else None
            )
            adjacent_authored = any(
                neighbor is not None
                and neighbor.authored
                and abs(neighbor.start - event.start) <= 1.75
                for neighbor in (previous, following)
            )
            if not adjacent_authored:
                continue
            chord = court_harmony.chord_at_beat(score, event.start)
            chord_pcs = court_harmony.chord_pitch_classes(chord) if chord else set()
            candidates = [
                midi for midi in range(low, high + 1)
                if midi % 12 in chord_pcs
                and midi % 12 in court_harmony.allowed_pitch_classes(
                    score, "lead", event.start
                )
            ]
            linked_neighbors = [
                neighbor for neighbor in (previous, following)
                if neighbor is not None
                and abs(neighbor.start - event.start) <= 1.75
            ]
            candidates = [
                midi for midi in candidates
                if all(abs(midi - int(neighbor.midi)) <= 8 for neighbor in linked_neighbors)
            ]
            if not candidates:
                continue

            local_indices = range(
                max(0, chain_index - 1),
                min(len(chain), chain_index + 2),
            )
            before_unresolved = sum(
                unresolved_at(chain, index) for index in local_indices
            )
            choices: list[tuple[tuple[float, ...], int]] = []
            for midi in candidates:
                trial = list(chain)
                trial[chain_index] = (event_index, replace(event, midi=midi))
                after_unresolved = sum(
                    unresolved_at(trial, index) for index in local_indices
                )
                if after_unresolved >= before_unresolved:
                    continue
                neighbor_motion = sum(
                    abs(midi - int(neighbor.midi)) for neighbor in linked_neighbors
                )
                choices.append((
                    (
                        float(after_unresolved),
                        float(neighbor_motion),
                        float(abs(midi - int(event.midi))),
                        float(midi),
                    ),
                    midi,
                ))
            if not choices:
                continue
            replacement_midi = min(choices)[1]
            result[event_index] = replace(event, midi=replacement_midi)
            changed = True
            break
        if not changed:
            break
    return result


def _leave_authored_foreground_space(events: list[NoteEvent]) -> list[NoteEvent]:
    """Carve brief gaps in near-unison accompaniment under authored lead.

    This keeps protected lead notes intelligible and prevents a held pad or
    chord voice from doubling them as an accidental 1-2 semitone cluster.
    """
    anchors = [
        event for event in events
        if event.track == "lead" and event.authored and event.midi is not None
    ]
    if not anchors:
        return events
    result: list[NoteEvent] = []
    for event in events:
        if (
            event.authored
            or event.track not in {"harmony", "pluck", "atmos"}
            or event.midi is None
        ):
            result.append(event)
            continue
        blocks = [
            (anchor.start - .025, anchor.start + anchor.duration + .025)
            for anchor in anchors
            if abs(int(event.midi) - int(anchor.midi)) <= 2
            and min(event.start + event.duration, anchor.start + anchor.duration)
            - max(event.start, anchor.start) > .02
        ]
        segments = [(event.start, event.start + event.duration)]
        for block_start, block_end in blocks:
            carved: list[tuple[float, float]] = []
            for start, end in segments:
                if block_end <= start or block_start >= end:
                    carved.append((start, end))
                    continue
                if block_start - start >= .06:
                    carved.append((start, block_start))
                if end - block_end >= .06:
                    carved.append((block_end, end))
            segments = carved
        result.extend(
            replace(event, start=start, duration=end - start)
            for start, end in segments
        )
    return result


def arrangement_interest_report(
    score: dict[str, Any],
    events: list[NoteEvent],
    sections: list[tuple[int, dict[str, Any]]],
) -> dict[str, Any]:
    """Positive musicality evidence beside the negative sour-note gate."""
    bars = int(score["transport"]["bars"])

    def signatures(track: str, *, pitched: bool = False) -> list[tuple[Any, ...]]:
        result: list[tuple[Any, ...]] = []
        for bar in range(bars):
            bar_events = [
                event for event in events
                if event.track == track and not event.authored and int(event.start // 4) == bar
            ]
            first_midi = next((event.midi for event in bar_events if event.midi is not None), None)
            cell: list[tuple[Any, ...]] = []
            for event in bar_events:
                offset = round((event.start - bar * 4.0) * 8.0) / 8.0
                duration = round(event.duration * 8.0) / 8.0
                identity: Any = event.kind
                if pitched and event.midi is not None and first_midi is not None:
                    identity = (event.midi - first_midi) % 12
                cell.append((offset, duration, identity))
            result.append(tuple(cell))
        return result

    def longest_streak(cells: list[tuple[Any, ...]]) -> int:
        best = run = 0
        previous: tuple[Any, ...] | None = None
        for cell in cells:
            if cell and cell == previous:
                run += 1
            else:
                run = 1 if cell else 0
            best = max(best, run)
            previous = cell
        return best

    drum_cells = signatures("drums")
    bass_cells = signatures("bass", pitched=True)
    lead_cells = signatures("lead", pitched=True)

    drum_backbones: list[tuple[tuple[str, float], ...]] = []
    downbeat_kick_hits = 0
    backbeat_hits = 0
    downbeat_kick_velocities: list[float] = []
    bass_events_total = 0
    bass_kick_hits = 0
    bass_root_downbeat_hits = 0
    core_timing_deviations: list[float] = []
    for bar in range(bars):
        bar_start = bar * 4.0
        drums = [
            event for event in events
            if event.track == "drums"
            and not event.authored
            and bar_start <= event.start < bar_start + 4.0
        ]
        bass = [
            event for event in events
            if event.track == "bass"
            and not event.authored
            and bar_start <= event.start < bar_start + 4.0
        ]
        kicks = [event for event in drums if event.kind == "kick"]
        snares = [event for event in drums if event.kind == "snare"]
        kick_offsets = [event.start - bar_start for event in kicks]
        backbone = tuple(sorted(
            (event.kind, round((event.start - bar_start) * 8.0) / 8.0)
            for event in drums
            if event.kind in {"kick", "snare"}
        ))
        drum_backbones.append(backbone)
        downbeat = next(
            (event for event in kicks if abs(event.start - bar_start) <= .10),
            None,
        )
        if downbeat is not None:
            downbeat_kick_hits += 1
            downbeat_kick_velocities.append(float(downbeat.velocity))
        if any(
            min(abs((event.start - bar_start) - 1.0), abs((event.start - bar_start) - 3.0))
            <= .10
            for event in snares
        ):
            backbeat_hits += 1
        for event in drums + bass:
            offset = event.start - bar_start
            core_timing_deviations.append(abs(offset - round(offset * 4.0) / 4.0))
        chord = court_harmony.chord_at_beat(score, bar_start)
        chord_root = court_harmony.parse_chord(chord)[0] if chord else None
        if any(
            abs(event.start - bar_start) <= .10
            and chord_root is not None
            and int(event.midi) % 12 == chord_root
            for event in bass
            if event.midi is not None
        ):
            bass_root_downbeat_hits += 1
        for event in bass:
            bass_events_total += 1
            offset = event.start - bar_start
            if any(abs(offset - kick_offset) <= .10 for kick_offset in kick_offsets):
                bass_kick_hits += 1

    backbone_counts = Counter(cell for cell in drum_backbones if cell)
    home_groove_coverage = (
        max(backbone_counts.values(), default=0) / max(1, bars)
    )
    groove_return_pairs = [
        (drum_backbones[index - 4], drum_backbones[index])
        for index in range(4, len(drum_backbones))
        if drum_backbones[index - 4] and drum_backbones[index]
    ]
    groove_return_rate = sum(left == right for left, right in groove_return_pairs) / max(
        1, len(groove_return_pairs)
    )
    kick_mean = float(np.mean(downbeat_kick_velocities)) if downbeat_kick_velocities else 0.0
    kick_cv = (
        float(np.std(downbeat_kick_velocities)) / kick_mean
        if kick_mean > 1e-9 else 0.0
    )
    section_phrase_alignment = all(
        section_bar % 4 == 0 and int(section["bars"]) % 4 == 0
        for section_bar, section in sections
    )
    section_density: list[float] = []
    orchestration: list[tuple[str, ...]] = []
    for section_bar, section in sections:
        start = section_bar * 4.0
        end = (section_bar + int(section["bars"])) * 4.0
        within = [event for event in events if start <= event.start < end]
        section_density.append(len(within) / max(1, int(section["bars"])))
        orchestration.append(tuple(sorted({event.track for event in within})))
    density_floor = max(.001, min(section_density, default=.001))
    density_contrast = max(section_density, default=0.0) / density_floor
    boundaries = [bar * 4.0 for bar, _section in sections[1:]]
    covered = sum(
        1 for boundary in boundaries
        if any(event.track == "fx" and boundary - 1.5 <= event.start <= boundary + .25 for event in events)
    )
    generated_lead = sorted(
        [event for event in events if event.track == "lead" and not event.authored and event.midi is not None],
        key=lambda event: event.start,
    )
    audible_lead = sorted(
        [event for event in events if event.track == "lead" and event.midi is not None],
        key=lambda event: event.start,
    )

    def motion_metrics(lead: list[NoteEvent]) -> dict[str, Any]:
        links = [
            (left, right)
            for left, right in zip(lead, lead[1:])
            if right.start - left.start <= 1.75
        ]
        intervals = [int(right.midi) - int(left.midi) for left, right in links]
        recovered = 0
        large = 0
        opportunities = 0
        for link_index, interval in enumerate(intervals):
            if abs(interval) < 5:
                continue
            large += 1
            if link_index + 1 < len(intervals) and links[link_index][1] is links[link_index + 1][0]:
                opportunities += 1
                answer = intervals[link_index + 1]
                if interval * answer < 0 and abs(answer) <= 4:
                    recovered += 1
        return {
            "max_step": max((abs(interval) for interval in intervals), default=0),
            "large_leaps": large,
            "recovery_opportunities": opportunities,
            "leap_recovery": round(recovered / max(1, opportunities), 3),
        }

    def color_metrics(lead: list[NoteEvent]) -> tuple[int, int]:
        unresolved = 0
        colors = 0
        for lead_index, event in enumerate(lead):
            chord = court_harmony.chord_at_beat(score, event.start)
            chord_pcs = court_harmony.chord_pitch_classes(chord) if chord else set()
            if int(event.midi) % 12 in chord_pcs:
                continue
            colors += 1
            previous = lead[lead_index - 1] if lead_index else None
            following = lead[lead_index + 1] if lead_index + 1 < len(lead) else None
            prepared = (
                previous is not None
                and event.start - previous.start <= 1.75
                and int(previous.midi) % 12 in chord_pcs
                and abs(int(event.midi) - int(previous.midi)) <= 3
            )
            following_chord = (
                court_harmony.chord_at_beat(score, following.start)
                if following is not None else None
            )
            following_pcs = (
                court_harmony.chord_pitch_classes(following_chord)
                if following_chord else set()
            )
            resolved = (
                following is not None
                and following.start - event.start <= 1.75
                and int(following.midi) % 12 in following_pcs
                and abs(int(following.midi) - int(event.midi)) <= 3
            )
            if not (prepared and resolved):
                unresolved += 1
        return colors, unresolved

    generated_motion = motion_metrics(generated_lead)
    audible_motion = motion_metrics(audible_lead)
    generated_colors, generated_unresolved = color_metrics(generated_lead)
    audible_colors, audible_unresolved = color_metrics(audible_lead)

    bar_roles: dict[int, str] = {}
    for section_bar, section in sections:
        for local_bar in range(int(section["bars"])):
            bar_roles[section_bar + local_bar] = _phrase_bar_role(local_bar, int(section["bars"]))
    lead_by_bar: dict[int, list[int]] = {}
    for event in audible_lead:
        lead_by_bar.setdefault(int(event.start // 4), []).append(int(event.midi))
    statement_bars = [
        bar for bar in sorted(lead_by_bar)
        if bar_roles.get(bar) == "statement" and len(lead_by_bar[bar]) >= 3
    ]
    final_section_bar = sections[-1][0] if sections else bars
    opening_bar = next((bar for bar in statement_bars if bar < final_section_bar), None)
    return_bar = next((bar for bar in statement_bars if bar >= final_section_bar), None)
    motif_return_similarity = 0.0
    if opening_bar is not None and return_bar is not None:
        opening = lead_by_bar[opening_bar]
        returning = lead_by_bar[return_bar]

        def lcs_f1(left: list[int], right: list[int]) -> float:
            if not left or not right:
                return 0.0
            row = [0] * (len(right) + 1)
            for left_value in left:
                previous = 0
                for index, right_value in enumerate(right, 1):
                    saved = row[index]
                    if left_value == right_value:
                        row[index] = previous + 1
                    else:
                        row[index] = max(row[index], row[index - 1])
                    previous = saved
            return 2.0 * row[-1] / (len(left) + len(right))

        opening_pc = [note % 12 for note in opening]
        returning_pc = [note % 12 for note in returning]
        pitch_similarity = max(
            lcs_f1([(pitch + shift) % 12 for pitch in opening_pc], returning_pc)
            for shift in range(12)
        )
        opening_intervals = [
            (right - left) % 12 for left, right in zip(opening, opening[1:])
        ]
        returning_intervals = [
            (right - left) % 12 for left, right in zip(returning, returning[1:])
        ]
        interval_similarity = lcs_f1(opening_intervals, returning_intervals)
        motif_return_similarity = .72 * pitch_similarity + .28 * interval_similarity
    metrics = {
        "drum_bar_patterns": len({cell for cell in drum_cells if cell}),
        "bass_bar_patterns": len({cell for cell in bass_cells if cell}),
        "lead_bar_patterns": len({cell for cell in lead_cells if cell}),
        "longest_repeated_drum_cell": longest_streak(drum_cells),
        "section_density_events_per_bar": [round(value, 3) for value in section_density],
        "section_density_contrast": round(density_contrast, 3),
        "orchestration_profiles": len(set(orchestration)),
        "transition_coverage": round(covered / max(1, len(boundaries)), 3),
        "generated_lead_max_step": generated_motion["max_step"],
        "generated_lead_large_leaps": generated_motion["large_leaps"],
        "generated_lead_recovery_opportunities": generated_motion["recovery_opportunities"],
        "generated_lead_leap_recovery": generated_motion["leap_recovery"],
        "generated_lead_color_tones": generated_colors,
        "generated_lead_unresolved_colors": generated_unresolved,
        "audible_lead_notes": len(audible_lead),
        "authored_lead_notes": sum(event.authored for event in audible_lead),
        "audible_lead_max_step": audible_motion["max_step"],
        "audible_lead_large_leaps": audible_motion["large_leaps"],
        "audible_lead_recovery_opportunities": audible_motion["recovery_opportunities"],
        "audible_lead_leap_recovery": audible_motion["leap_recovery"],
        "audible_lead_color_tones": audible_colors,
        "audible_lead_unresolved_colors": audible_unresolved,
        "motif_return_similarity": round(motif_return_similarity, 3),
        "home_groove_coverage": round(home_groove_coverage, 3),
        "groove_four_bar_return_rate": round(groove_return_rate, 3),
        "downbeat_kick_coverage": round(downbeat_kick_hits / max(1, bars), 3),
        "backbeat_coverage": round(backbeat_hits / max(1, bars), 3),
        "bass_kick_lock": round(bass_kick_hits / max(1, bass_events_total), 3),
        "bass_root_downbeat_lock": round(bass_root_downbeat_hits / max(1, bars), 3),
        "core_timing_max_deviation": round(max(core_timing_deviations, default=0.0), 4),
        "downbeat_kick_velocity_cv": round(kick_cv, 3),
        "section_phrase_alignment": section_phrase_alignment,
    }
    issues: list[str] = []
    if metrics["drum_bar_patterns"] < 2:
        issues.append("drum grammar lacks a home groove and phrase-tail fill")
    if metrics["drum_bar_patterns"] > 5:
        issues.append("drum grammar changes too often to establish a home groove")
    if metrics["bass_bar_patterns"] < 2:
        issues.append("bass grammar lacks a stable foundation and cadence")
    if metrics["bass_bar_patterns"] > 4:
        issues.append("bass rhythm changes too often to reinforce the groove")
    if metrics["lead_bar_patterns"] < 3:
        issues.append("motif rhythm lacks statement/development/cadence contrast")
    if metrics["home_groove_coverage"] < .60:
        issues.append("no recurring drum backbone owns most of the arrangement")
    if metrics["groove_four_bar_return_rate"] < .75:
        issues.append("the home groove does not return reliably on the phrase clock")
    if metrics["downbeat_kick_coverage"] < .90:
        issues.append("kick does not establish beat one consistently")
    if metrics["backbeat_coverage"] < .85:
        issues.append("snare backbeat disappears too often to project the declared tempo")
    if metrics["bass_kick_lock"] < .60:
        issues.append("bass foundation is not locked to the kick pattern")
    if metrics["bass_root_downbeat_lock"] < .85:
        issues.append("bass does not establish the active chord root with the downbeat")
    if metrics["core_timing_max_deviation"] > .08:
        issues.append("core kick/snare/bass timing drifts outside the shared subdivision grid")
    if metrics["downbeat_kick_velocity_cv"] > .18:
        issues.append("core kick dynamics vary too sharply to sustain one pocket")
    if not metrics["section_phrase_alignment"]:
        issues.append("section boundaries split the global four-bar phrase clock")
    if metrics["section_density_contrast"] < 1.18:
        issues.append("sections do not create enough density contrast")
    if metrics["transition_coverage"] < .75:
        issues.append("section boundaries lack prepared gestures")
    if metrics["audible_lead_max_step"] > 8:
        issues.append("melody contains an unbounded close-succession leap")
    if metrics["audible_lead_unresolved_colors"]:
        issues.append("melody contains unprepared or unresolved color tones")
    if metrics["audible_lead_recovery_opportunities"] and metrics["audible_lead_leap_recovery"] < .70:
        issues.append("melodic leaps do not recover by step in the opposite direction")
    if metrics["motif_return_similarity"] < .55:
        issues.append("final section does not recall the opening motif recognizably")
    return {"ok": not issues, "issues": issues, "metrics": metrics}


def expand_score(score: dict[str, Any]) -> tuple[list[NoteEvent], list[dict[str, Any]], list[tuple[int, dict[str, Any]]]]:
    """Compile one declarative score into the exact notes the DAW displays/renders.
    Now handles v2 (from schema migration) by normalizing motif/harmony/chords.
    """
    errors = validate_score(score)
    if errors:
        raise ValueError("; ".join(errors))
    transport = score["transport"]
    harmony = score["harmony"]
    seeds = score.get("seeds", {})
    if not seeds:
        rng_humanization = np.random.default_rng(int(score.get("seed", 0)))
    else:
        rng_humanization = np.random.default_rng(seeds["humanization"])
    style = score["style"]
    swing = float(transport.get("swing", 0.0))
    scale = _scale_notes(harmony["tonic"], harmony["mode"], 24, 96)
    total_beats = int(transport["bars"]) * 4
    authored_lead_spans = [
        (float(note.get("beat", 0.0)), float(note.get("duration", .5)))
        for note in manual_notes(score)
        if str(note.get("track", "lead")) == "lead"
        and float(note.get("beat", 0.0)) < total_beats
    ]

    # Support v1 "motif" or v2 "motifs"
    raw_motif = score.get("motif") or (score.get("motifs") or [{}])[0].get("notes", [])
    motif = [note_to_midi(note) for note in raw_motif]

    events: list[NoteEvent] = []
    sections: list[tuple[int, dict[str, Any]]] = []
    previous_chord_mid: list[int] | None = None
    last_generated_lead_note: int | None = None
    last_generated_lead_interval: int | None = None
    last_generated_lead_start = -999.0
    opening_motif_voice: list[int] | None = None
    arrangement_gain = .92

    def add(
        track: str,
        start: float,
        duration: float,
        midi: int | None,
        velocity: float,
        kind: str = "note",
        *,
        authored: bool = False,
    ) -> None:
        jitter = 0.0
        if kind == "note" and track in {"pluck", "percussion"}:
            jitter = float(rng_humanization.uniform(-0.012, 0.012))
        if midi is not None and track in court_harmony.TRACK_RANGES:
            midi = court_harmony.nearest_allowed_midi(score, track, start, midi)
        # The rhythm section is the listener's ruler: its own accent formula
        # may breathe, but the large-scale arrangement gain must not make the
        # kick and bass feel as though the tempo/pocket changed at a section
        # boundary.
        layer_gain = .96 if track in {"drums", "percussion", "bass"} and not authored else arrangement_gain
        events.append(NoteEvent(
            track,
            max(0.0, start + jitter),
            duration,
            midi,
            _clamp(velocity * (1.0 if authored else layer_gain), 0.02, 1.0),
            TRACK_BY_ID[track]["wave"],
            kind,
            authored,
        ))

    score_sections = list(score["sections"])
    climax_index = max(
        range(len(score_sections)),
        key=lambda item: float(score_sections[item]["energy"]),
    )
    for index, (section, section_bar) in enumerate(_section_offsets(score_sections)):
        sections.append((section_bar, section))
        energy = float(section["energy"])
        section_bars = int(section["bars"])
        transform = str(section["transform"])
        is_final_section = index == len(score_sections) - 1
        if index:
            boundary = section_bar * 4.0
            previous_energy = float(score_sections[index - 1]["energy"])
            if energy >= previous_energy:
                add("fx", boundary - 1.25, 1.18, None, .12 + .18 * energy, "riser")
                add("fx", boundary, .52, None, .16 + .16 * energy, "impact")
            else:
                add("fx", boundary, 1.35, None, .14 + .14 * previous_energy, "downlifter")
        for local_bar in range(section_bars):
            bar = section_bar + local_bar
            beat = bar * 4.0
            role = _phrase_bar_role(local_bar, section_bars)
            phrase_index = bar // 4
            progress = local_bar / max(1, section_bars - 1)
            if is_final_section:
                target_energy = energy * (1.0 - .16 * progress)
            elif index <= climax_index:
                target_energy = energy * (.94 + .06 * progress)
            else:
                target_energy = energy * (.98 - .06 * progress)
            if index and local_bar < 2:
                previous_energy = float(score_sections[index - 1]["energy"])
                blend = (local_bar + 1) / 2.0
                blend = blend * blend * (3.0 - 2.0 * blend)
                bar_energy = previous_energy + (target_energy - previous_energy) * blend
            else:
                bar_energy = target_energy
            phrase_accent = (1.0, .98, 1.01, 1.04)[bar % 4]
            bar_energy = _clamp(bar_energy * phrase_accent, .08, 1.0)
            # Density and orchestration carry the large-scale energy curve.
            # Harmonic/melodic layers receive this bounded gain exactly once;
            # the pulse keeps its own much narrower accent envelope.
            arrangement_gain = .55 + .60 * bar_energy
            pre_climax_break = index == climax_index - 1 and role == "cadence"
            pre_return_break = index == len(score_sections) - 2 and local_bar == section_bars - 1
            chord = court_harmony.chord_at_beat(score, beat)
            if not chord:
                raise ValueError(f"harmony has no chord covering bar {bar + 1}")
            chord_low = chord_midis(chord, 2)
            chord_mid = _voice_chord_near(chord, previous_chord_mid)
            previous_chord_mid = chord_mid
            chord_high = [note + 12 for note in chord_mid]
            root = chord_low[0]

            # Publish the foreground plan before accompaniment is placed.  The
            # other roles can now leave complementary gaps instead of every
            # layer reacting independently to the same energy scalar.
            lead_active = (
                bar_energy > .20
                and not pre_climax_break
                and (
                    role == "cadence"
                    or bar_energy >= .42
                    or local_bar % 2 == 0
                )
                and not (is_final_section and role == "answer")
                and not pre_return_break
            )
            lead_rhythm = list(LEAD_RHYTHMS[style][role]) if lead_active else []
            if lead_active and bar_energy < .34 and not (index == 0 and local_bar == 0):
                lead_rhythm = lead_rhythm[::2]
            lead_rhythm = [
                (offset, duration)
                for offset, duration in lead_rhythm
                if not any(
                    min(
                        beat + _swung_offset(offset, swing, style) + duration,
                        anchor_start + anchor_duration,
                    )
                    - max(beat + _swung_offset(offset, swing, style), anchor_start) > .02
                    for anchor_start, anchor_duration in authored_lead_spans
                )
            ]
            lead_active = bool(lead_rhythm)
            lead_offsets = [
                _swung_offset(offset, swing, style)
                for offset, _duration in lead_rhythm
            ]
            foreground_offsets = sorted(
                lead_offsets
                + [
                    max(0.0, anchor_start - beat)
                    for anchor_start, anchor_duration in authored_lead_spans
                    if min(beat + 4.0, anchor_start + anchor_duration)
                    - max(beat, anchor_start) > .02
                ]
            )

            # Pulse grammar: three bars repeat one home pocket and only the
            # phrase tail receives a fill.  Energy changes orchestration around
            # this clock instead of replacing the clock every bar.
            drum_cell = DRUM_CELLS[style][role]
            kicks = list(drum_cell["kick"])
            snares = list(drum_cell["snare"])
            hats = list(drum_cell["hat"])
            if bar_energy < .24:
                hats = hats[::2]
            if pre_climax_break:
                kicks, snares, hats = kicks[:1], snares[:1], hats[:2]
            for hit_no, offset in enumerate(kicks):
                add("drums", beat + offset, .40 if hit_no == 0 else .25, None,
                    (.50 + .16 * bar_energy) * (1.0 if hit_no == 0 else .86), "kick")
            for hit_no, offset in enumerate(snares):
                add("drums", beat + offset, .30 if hit_no == 0 else .18, None,
                    (.42 + .12 * bar_energy) * (1.0 if hit_no == 0 else .90), "snare")
            if bar_energy > .15:
                for hat_no, offset in enumerate(hats):
                    moved = _swung_offset(offset, swing, style)
                    kind = "hat_open" if role == "cadence" and hat_no == len(hats) - 1 and bar_energy > .62 else "hat"
                    accent = 1.0 if abs(offset - round(offset)) < .06 else .72
                    add("percussion", beat + moved, .13 if kind == "hat_open" else .08, None,
                        (.10 + .08 * bar_energy) * accent, kind)

            # Bass repeats its home root/fifth pocket for three bars, then may
            # add one controlled cadence pickup.  Kick lock and a downbeat root
            # remain the foundation rather than optional variety.
            bass_cell = list(BASS_CELLS[style][role])
            if pre_climax_break:
                bass_cell = bass_cell[:1]
            for offset, interval, duration, accent in bass_cell:
                add("bass", beat + offset, duration, root + interval,
                    (.38 + .15 * bar_energy) * accent)

            # Comping changes from holds to syncopated stabs and cadential
            # releases.  A chord is still present on every downbeat so the
            # piano roll and harmony policy expose a legible progression.
            comp_cell = list(COMP_CELLS[style][role])
            if bar_energy < .34:
                comp_cell = [(0.0, 3.42, 1.0)]
            elif pre_climax_break:
                comp_cell = [(0.0, 1.32, .82)]
            elif is_final_section and role == "answer":
                comp_cell = [(0.0, 2.70, .82)]
            if foreground_offsets and bar_energy >= .48 and len(comp_cell) > 1:
                comp_cell = [
                    cell for cell in comp_cell
                    if cell[0] == 0.0
                    or all(abs(cell[0] - lead_offset) > .30 for lead_offset in foreground_offsets)
                ]
            for offset, duration, accent in comp_cell:
                for note in chord_mid[:3]:
                    add("harmony", beat + offset, duration, note,
                        .18 * accent)

            # Arpeggio direction, rhythm, and density follow phrase role and
            # style instead of repeating one five-note cell for all genres.
            if bar_energy > .34 and not pre_climax_break and not is_final_section:
                arp = chord_high[:3] + list(reversed(chord_high[:2]))
                rotation = phrase_index % len(arp)
                arp = arp[rotation:] + arp[:rotation]
                if role in {"answer", "cadence"}:
                    arp = list(reversed(arp))
                pluck_offsets = list(PLUCK_RHYTHMS[style][role])
                if bar_energy < .56:
                    pluck_offsets = pluck_offsets[::2]
                if foreground_offsets:
                    pluck_offsets = [
                        offset for offset in pluck_offsets
                        if all(
                            abs(_swung_offset(offset, swing, style) - lead_offset) > .20
                            for lead_offset in foreground_offsets
                        )
                    ]
                for step_no, offset in enumerate(pluck_offsets):
                    moved = _swung_offset(offset, swing, style)
                    add("pluck", beat + moved, .20 if style != "spicy_lofi" else .28,
                        arp[step_no % len(arp)], .16 * (1.0 if step_no == 0 else .82))

            # Pads enter and leave in phrases.  The afterglow restores them as
            # the rhythm section recedes, producing orchestration contrast.
            pad_active = (
                (bar_energy < .38 and local_bar % 2 == 0)
                or (is_final_section and role in {"statement", "variation", "cadence"})
                or (.38 <= bar_energy < .72 and role in {"statement", "cadence"})
                or (bar_energy >= .72 and role == "variation")
            )
            if pad_active and not pre_climax_break:
                pad_notes = chord_high[:2] if foreground_offsets else chord_high[:3]
                for note in pad_notes:
                    add("atmos", beat + .08, 3.66, note, .10)

            # Melody receives independent rhythmic development, rests, a
            # phrase peak, and a chord-root cadence.  It is no longer squeezed
            # uniformly into every second bar.
            if lead_active:
                # The final section begins with a recognizable A-prime return;
                # register, energy and accompaniment may change, but the seed
                # contour itself is deliberately recalled before development.
                if is_final_section and local_bar == 0:
                    phrase = list(motif)
                else:
                    phrase = _developed_motif(motif, transform, phrase_index, role, scale)
                phrase_targets = [
                    phrase[step_no % len(phrase)]
                    for step_no in range(len(lead_rhythm))
                ]
                if role == "cadence" and phrase_targets:
                    phrase_targets[-1] = root + 24
                first_start = beat + lead_offsets[0] if lead_offsets else beat
                prior_note = (
                    last_generated_lead_note
                    if first_start - last_generated_lead_start <= 1.75
                    else None
                )
                prior_interval = last_generated_lead_interval if prior_note is not None else None
                if (
                    is_final_section
                    and local_bar == 0
                    and opening_motif_voice is not None
                    and len(opening_motif_voice) == len(lead_rhythm)
                ):
                    voiced_phrase = list(opening_motif_voice)
                else:
                    voiced_phrase = _voice_melody_phrase(
                        score,
                        beat,
                        phrase_targets,
                        lead_rhythm,
                        scale,
                        previous_note=prior_note,
                        previous_interval=prior_interval,
                    )
                if opening_motif_voice is None and role == "statement" and not is_final_section:
                    opening_motif_voice = list(voiced_phrase)
                for step_no, ((offset, duration), note) in enumerate(zip(lead_rhythm, voiced_phrase)):
                    moved = _swung_offset(offset, swing, style)
                    accent = 1.0 if step_no in {0, len(lead_rhythm) - 1} else .82
                    add("lead", beat + moved, duration, note,
                        .27 * accent)
                if voiced_phrase:
                    if len(voiced_phrase) >= 2:
                        last_generated_lead_interval = voiced_phrase[-1] - voiced_phrase[-2]
                    elif prior_note is not None:
                        last_generated_lead_interval = voiced_phrase[-1] - prior_note
                    else:
                        last_generated_lead_interval = None
                    last_generated_lead_note = voiced_phrase[-1]
                    last_generated_lead_start = beat + lead_offsets[-1]
        if is_final_section:
            ending = (section_bar + section_bars) * 4.0
            add("fx", ending - .82, .72, None, .10 + .10 * energy, "downlifter")

    for manual in manual_notes(score):
        start = float(manual.get("beat", 0))
        if start >= total_beats:
            continue
        add(
            str(manual.get("track", "lead")), start, float(manual.get("duration", 0.5)),
            note_to_midi(str(manual.get("pitch", "C4"))), float(manual.get("velocity", .65)),
            authored=True,
        )
    authored_by_track = {
        track: [event for event in events if event.authored and event.track == track]
        for track in court_harmony.MONOPHONIC_TRACKS
    }
    # Manual piano-roll phrasing replaces the generated mono voice over its
    # exact time span. Summing both was a second source of sour lead/bass notes.
    events = [
        event for event in events
        if event.authored
        or event.track not in court_harmony.MONOPHONIC_TRACKS
        or not any(
            min(event.start + event.duration, manual.start + manual.duration)
            - max(event.start, manual.start) > 0.02
            for manual in authored_by_track.get(event.track, [])
        )
    ]
    events = _revoice_generated_lead_around_authored(score, events)
    events = _repair_generated_colors_around_authored(score, events)
    events = _leave_authored_foreground_space(events)
    events.sort(key=lambda event: (event.start, event.track, event.midi or -1))
    event_grade = court_harmony.grade_events(score, events)
    if not event_grade["ok"]:
        raise ValueError("; ".join(event_grade["issues"]))
    return events, resolved_tracks(score), sections


def _arrangement_pitch_hints(
    score: dict[str, Any], notes: list[dict[str, Any]]
) -> dict[tuple[Any, ...], int]:
    """Map protected note timings to the target arranger's local voice.

    Hints are only used after ordinary scale-degree reharmonization cannot pass
    the final audible-line gate.  Pitch-specific keys keep polyphonic manual
    chords from collapsing onto one hinted voice.
    """
    baseline = json.loads(json.dumps(score))
    baseline["manual_notes"] = []
    baseline_events, _tracks, _sections = expand_score(baseline)
    hints: dict[tuple[Any, ...], int] = {}
    for note in notes:
        track = str(note.get("track", "lead"))
        start = float(note.get("beat", 0.0))
        duration = float(note.get("duration", .5))
        try:
            original_midi = note_to_midi(str(note.get("pitch", "")))
        except ValueError:
            continue
        candidates = [
            event for event in baseline_events
            if event.track == track and event.midi is not None and not event.authored
        ]
        overlapping = [
            event for event in candidates
            if min(event.start + event.duration, start + duration)
            - max(event.start, start) > .02
        ]
        if overlapping:
            best = max(
                overlapping,
                key=lambda event: (
                    min(event.start + event.duration, start + duration)
                    - max(event.start, start),
                    -abs(int(event.midi) - original_midi),
                    -abs(event.start - start),
                ),
            )
        else:
            nearby = [event for event in candidates if abs(event.start - start) <= 1.75]
            if not nearby:
                hints[(track, round(start, 6), original_midi)] = original_midi
                continue
            best = min(
                nearby,
                key=lambda event: (
                    abs(event.start - start),
                    abs(int(event.midi) - original_midi),
                ),
            )
        hints[(track, round(start, 6), original_midi)] = int(best.midi)
    return hints


def _arrangement_issues(score: dict[str, Any]) -> list[str]:
    structural = validate_score(score)
    if structural:
        return structural
    try:
        events, _tracks, sections = expand_score(score)
    except ValueError as exc:
        return [str(exc)]
    return arrangement_interest_report(score, events, sections)["issues"]


def normalize_score_harmony(
    score: dict[str, Any],
    *,
    source_score: dict[str, Any] | None = None,
) -> tuple[dict[str, Any], dict[str, int]]:
    """Reharmonize protected v1 notes and prove the final audible result.

    The first pass preserves scale-degree identity.  If a new harmony/style
    makes that fixed overlay fail the audible melody gate, a bounded second
    pass keeps every authored timing/duration/velocity but fits its pitch to
    the target arranger's local voice.  This prevents autonomous recovery from
    endlessly queuing scores that can never launch.
    """
    if score.get("schema_version") != 1:
        raise ValueError("automatic harmony normalization is enabled for live schema v1 only")
    original_notes = manual_notes(score)
    hints = _arrangement_pitch_hints(score, original_notes) if original_notes else {}

    def attempt(*, contextual: bool) -> tuple[dict[str, Any], dict[str, int], list[str]]:
        normalized, stats = court_harmony.normalize_manual_notes(
            score,
            original_notes,
            source_score=source_score,
            pitch_hints=hints if contextual or source_score is None else None,
            force_pitch_hints=contextual,
        )
        candidate = json.loads(json.dumps(score))
        candidate["manual_notes"] = normalized
        if contextual and normalized:
            try:
                compiled, _tracks, _sections = expand_score(candidate)
                fitted = _revoice_generated_lead_around_authored(
                    candidate,
                    compiled,
                    lock_authored=False,
                )
                fitted_by_start = {
                    round(event.start, 6): int(event.midi)
                    for event in fitted
                    if event.track == "lead" and event.authored and event.midi is not None
                }
                register_repairs = 0
                for note in candidate["manual_notes"]:
                    if str(note.get("track", "lead")) != "lead":
                        continue
                    fitted_midi = fitted_by_start.get(round(float(note.get("beat", 0.0)), 6))
                    if fitted_midi is None:
                        continue
                    old_pitch = str(note.get("pitch", ""))
                    new_pitch = midi_to_note(fitted_midi)
                    if new_pitch != old_pitch:
                        note["pitch"] = new_pitch
                        register_repairs += 1
                stats["authored_register_repairs"] = register_repairs
            except ValueError:
                # The issue list below remains authoritative.  Never move or
                # drop an authored note unless a complete fitted line exists.
                pass
        issues = _arrangement_issues(candidate)
        return candidate, stats, issues

    candidate, stats, issues = attempt(contextual=False)
    if issues and source_score is not None and hints:
        candidate, stats, issues = attempt(contextual=True)
        stats["arrangement_context_fallback"] = 1
    else:
        stats["arrangement_context_fallback"] = 0
    if issues:
        raise ValueError("; ".join(issues))
    candidate.setdefault("lineage", {})["harmony_normalization"] = stats
    return candidate, stats


def _oscillator(freq: float, samples: int, wave_name: str, rng: np.random.Generator) -> np.ndarray:
    t = np.arange(samples, dtype=np.float32) / SAMPLE_RATE
    phase = (t * freq) % 1.0
    if wave_name == "sine":
        return np.sin(2 * np.pi * phase)
    if wave_name == "triangle":
        return 4 * np.abs(phase - .5) - 1
    if wave_name == "square":
        return np.where(phase < .5, 1.0, -1.0).astype(np.float32)
    if wave_name == "pulse":
        return np.where(phase < .32, 1.0, -0.70).astype(np.float32)
    if wave_name == "saw":
        return (2 * phase - 1).astype(np.float32)
    return rng.normal(0, .25, samples).astype(np.float32)


def _envelope(samples: int, duration: float, wave_name: str) -> np.ndarray:
    if samples <= 0:
        return np.zeros(0, dtype=np.float32)
    attack = min(.025 if wave_name in {"pluck", "pulse", "square"} else .08, duration * .24)
    release = min(.12 if wave_name in {"pluck", "pulse", "square"} else .45, duration * .38)
    a = max(1, int(attack * SAMPLE_RATE))
    r = max(1, int(release * SAMPLE_RATE))
    env = np.ones(samples, dtype=np.float32)
    env[:a] = np.linspace(0, 1, a, dtype=np.float32)
    env[-r:] *= np.linspace(1, 0, r, dtype=np.float32)
    return env


def _drum(kind: str, duration: float, rng: np.random.Generator,
          patch_id: str | None = None, macros: dict[str, float] | None = None) -> np.ndarray:
    n = max(8, int(duration * SAMPLE_RATE))
    t = np.arange(n, dtype=np.float32) / SAMPLE_RATE
    macros = macros or {}
    if kind == "kick":
        freq = 132 * np.exp(-t * 18) + 46
        body = np.sin(2 * np.pi * np.cumsum(freq) / SAMPLE_RATE) * np.exp(-t * 12)
        if patch_id == "kit.mechanical_court":
            body += np.sin(2 * np.pi * 1750 * t) * np.exp(-t * 70) * .08
        if patch_id == "kit.brutal_impact":
            body *= 1.3
            body += rng.normal(0, .4, n).astype(np.float32) * np.exp(-t * 8) * .4
        return body.astype(np.float32)
    if kind in {"snare", "clap"}:
        decay = 18 if kind == "snare" else 27
        noise = rng.normal(0, 1, n).astype(np.float32)
        if patch_id == "kit.velvet_lofi":
            noise = np.convolve(noise, np.ones(7, dtype=np.float32) / 7, mode="same")
            decay *= .82
        if patch_id == "kit.lofi_dust":
            noise = np.convolve(noise, np.ones(9, dtype=np.float32) / 9, mode="same")
            noise += rng.normal(0, .15, n).astype(np.float32) * np.exp(-t * 12) * .3
        return noise * np.exp(-t * decay)
    if kind == "riser":
        return rng.normal(0, .7, n).astype(np.float32) * np.linspace(.05, 1, n, dtype=np.float32) * np.sin(2 * np.pi * (180 + t * 600) * t)
    if kind == "impact":
        pitch = np.sin(2 * np.pi * np.cumsum(78 * np.exp(-t * 10) + 34) / SAMPLE_RATE)
        grit = rng.normal(0, .55, n).astype(np.float32) * np.exp(-t * 11)
        return (pitch * np.exp(-t * 7) * .82 + grit * .28).astype(np.float32)
    if kind == "downlifter":
        sweep = np.linspace(1.0, .04, n, dtype=np.float32)
        noise = rng.normal(0, .52, n).astype(np.float32)
        tone = np.sin(2 * np.pi * (620 - np.minimum(t, .8) * 520) * t)
        return (noise * .72 + tone * .28) * sweep
    # hats
    hat = rng.normal(0, .45, n).astype(np.float32)
    if patch_id == "kit.velvet_lofi":
        hat = np.convolve(hat, np.ones(5, dtype=np.float32) / 5, mode="same")
        hat *= .82 + float(macros.get("dust", .18)) * .12
    if patch_id == "kit.lofi_dust":
        hat = np.convolve(hat, np.ones(7, dtype=np.float32) / 7, mode="same")
        hat *= .7
    decay = 14 if kind == "hat_open" else (32 if patch_id == "kit.velvet_lofi" else 40)
    return hat * np.exp(-t * decay)


def _stereo(mono: np.ndarray, pan: float) -> np.ndarray:
    angle = (_clamp(pan, -1, 1) + 1) * math.pi / 4
    return np.column_stack((mono * math.cos(angle), mono * math.sin(angle))).astype(np.float32)


def _tonal_voice(freq: float, samples: int, duration: float, wave_name: str,
                 patch_id: str | None, macros: dict[str, float],
                 rng: np.random.Generator) -> np.ndarray:
    """Render the live tonal voice; registered proof patches have distinct topology."""
    envelope = _envelope(samples, duration, wave_name)
    t = np.arange(samples, dtype=np.float32) / SAMPLE_RATE
    if patch_id == "keys.nocturne_felt":
        body = .58 * _oscillator(freq, samples, "triangle", rng)
        body += .32 * _oscillator(freq * 2.0, samples, "sine", rng)
        hammer = rng.normal(0, .08, samples).astype(np.float32) * np.exp(-t * 48)
        voice = (body + hammer) * _envelope(samples, duration, "triangle")
    elif patch_id == "pluck.glass_current":
        partial = .58 * _oscillator(freq, samples, "sine", rng)
        partial += .27 * _oscillator(freq * 2.01, samples, "triangle", rng)
        partial += .15 * _oscillator(freq * 3.98, samples, "sine", rng)
        damping = 3.0 + 5.0 * float(macros.get("damping", .55))
        voice = partial * np.exp(-t * damping) * _envelope(samples, duration, "square")
    elif patch_id == "bass.substructure":
        sub = _oscillator(freq, samples, "sine", rng) * .78
        edge = _oscillator(freq, samples, "saw", rng) * .22
        edge = np.convolve(edge, np.ones(9, dtype=np.float32) / 9, mode="same")
        voice = (sub + edge) * _envelope(samples, duration, "sine")
    elif patch_id == "lead.ember_superwave":
        vibrato = 1.0 + np.sin(2 * np.pi * 5.3 * t) * (.001 + .004 * float(macros.get("vibrato", .25)))
        phase = np.cumsum(freq * vibrato / SAMPLE_RATE, dtype=np.float64) % 1.0
        main = (2 * phase - 1).astype(np.float32)
        detuned = _oscillator(freq * 1.008, samples, "pulse", rng)
        voice = (main * .62 + detuned * .38) * _envelope(samples, duration, "pulse")
    elif patch_id == "pad.aurora_choir":
        fundamental = _oscillator(freq, samples, "sine", rng) * .62
        choir = _oscillator(freq * 2.0, samples, "sine", rng) * .23
        undertone = _oscillator(freq * .5, samples, "triangle", rng) * .15
        attack = max(1, min(samples, int(min(.38, duration * .30) * SAMPLE_RATE)))
        release = max(1, min(samples, int(min(.55, duration * .36) * SAMPLE_RATE)))
        pad_envelope = np.ones(samples, dtype=np.float32)
        pad_envelope[:attack] = np.linspace(0, 1, attack, dtype=np.float32)
        pad_envelope[-release:] *= np.linspace(1, 0, release, dtype=np.float32)
        voice = (fundamental + choir + undertone) * pad_envelope
    elif patch_id == "keys.stage_grand":
        body = .72 * _oscillator(freq, samples, "saw", rng)
        body += .18 * _oscillator(freq * 2.0, samples, "sine", rng)
        hammer = rng.normal(0, .06, samples).astype(np.float32) * np.exp(-t * 35)
        voice = (body + hammer) * _envelope(samples, duration, "saw")
    elif patch_id == "keys.tine_electric":
        # FM-style tine
        mod = np.sin(2 * np.pi * freq * 1.8 * t) * (0.9 + 0.6 * float(macros.get("character", 0.72)))
        carrier = np.sin(2 * np.pi * freq * t + mod)
        voice = carrier * _envelope(samples, duration, "triangle")
    elif patch_id == "bass.saw_foundation":
        body = _oscillator(freq, samples, "saw", rng) * .82
        sub = _oscillator(freq * 0.5, samples, "sine", rng) * .18
        body = np.convolve(body, np.ones(5, dtype=np.float32) / 5, mode="same")
        voice = (body + sub) * _envelope(samples, duration, "saw")
    elif patch_id == "pad.velvet_dusk":
        base = _oscillator(freq, samples, "triangle", rng) * .55
        warm = _oscillator(freq * 1.5, samples, "sine", rng) * .25
        air = _oscillator(freq * 2.02, samples, "sine", rng) * .20
        slow_env = np.ones(samples, dtype=np.float32)
        slow = max(1, int(0.6 * samples))
        slow_env[:slow] = np.linspace(0, 1, slow, dtype=np.float32)
        voice = (base + warm + air) * slow_env
    elif patch_id == "lead.saw_bite":
        body = _oscillator(freq, samples, "saw", rng) * .78
        body = np.tanh(body * 1.4) * .9
        voice = body * _envelope(samples, duration, "saw")
    elif patch_id == "pad.ether":
        base = _oscillator(freq, samples, "sine", rng) * .7
        shimmer = _oscillator(freq * 2.01, samples, "sine", rng) * .2
        air = _oscillator(freq * 3.02, samples, "sine", rng) * .1
        very_slow = np.ones(samples, dtype=np.float32)
        vs = max(1, int(0.9 * samples))
        very_slow[:vs] = np.linspace(0, 1, vs, dtype=np.float32)
        voice = (base + shimmer + air) * very_slow
    else:
        patch = court_instruments.REGISTRY.get(str(patch_id))
        engine = patch.engine if patch else "subtractive"
        digest = hashlib.sha256(str(patch_id).encode("utf-8")).digest()
        variant = int.from_bytes(digest[:2], "big") / 65535.0
        if engine == "fm":
            ratio = 1.25 + variant * 2.75
            index = .5 + 2.4 * float(macros.get("character", .5))
            voice = np.sin(2 * np.pi * freq * t + index * np.sin(2 * np.pi * freq * ratio * t)) * envelope
        elif engine == "additive":
            voice = sum((1.0 / n) * np.sin(2 * np.pi * freq * n * t + variant * n) for n in range(1, 6))
            voice = np.asarray(voice, dtype=np.float32) * envelope * .58
        elif engine == "wavetable":
            sine = _oscillator(freq, samples, "sine", rng)
            saw = _oscillator(freq * (1.0 + (variant - .5) * .01), samples, "saw", rng)
            voice = (sine * (1.0 - variant) + saw * variant) * envelope
        elif engine == "granular":
            carrier = _oscillator(freq, samples, "triangle", rng)
            grain = .35 + .65 * np.square(np.sin(2 * np.pi * (5 + variant * 13) * t))
            voice = (carrier * grain + rng.normal(0, .035, samples)) * envelope
        elif engine == "noise":
            noise = rng.normal(0, .35, samples).astype(np.float32)
            voice = np.convolve(noise, np.ones(5, dtype=np.float32) / 5, mode="same") * envelope
        elif engine == "pluck":
            body = .65 * _oscillator(freq, samples, "triangle", rng) + .35 * _oscillator(freq * 2.01, samples, "sine", rng)
            voice = body * np.exp(-t * (2.5 + 6 * float(macros.get("damping", .5)))) * envelope
        else:
            detune = 1.002 + variant * .008
            voice = (.68 * _oscillator(freq, samples, wave_name, rng) + .32 * _oscillator(freq * detune, samples, "saw", rng)) * envelope

    tone = _clamp(float(macros.get("tone", .65)), 0.0, 1.0)
    smoothing = max(1, int(round(1 + (1.0 - tone) * 10)))
    if smoothing > 1:
        voice = np.convolve(voice, np.ones(smoothing, dtype=np.float32) / smoothing, mode="same")
    character = _clamp(float(macros.get("character", .35)), 0.0, 1.0)
    if character > .02:
        voice = np.tanh(voice * (1.0 + character * 1.25)) / (1.0 + character * .22)
    return voice.astype(np.float32)


def _tempo_delay_frames(seconds_per_beat: float) -> int:
    """Return an exact one-beat delay without introducing a second clock."""
    return max(1, int(round(float(seconds_per_beat) * SAMPLE_RATE)))


def _fold_tail_into_loop(audio: np.ndarray, loop_samples: int) -> np.ndarray:
    """Fold effect tails into bar one and make the stored cycle sample-continuous."""
    if loop_samples <= 0 or len(audio) < loop_samples:
        raise ValueError("loop sample count must fit inside the rendered audio")
    loop = np.array(audio[:loop_samples], dtype=np.float32, copy=True)
    tail = audio[loop_samples:]
    if len(tail):
        # The render tail is intentionally much shorter than a valid two-minute
        # form.  Keep the modulo form so this remains correct for small fixtures.
        cursor = 0
        while cursor < len(tail):
            chunk = min(loop_samples, len(tail) - cursor)
            loop[:chunk] += tail[cursor:cursor + chunk]
            cursor += chunk
    # Correct the final 25 ms toward the first sample with a raised-cosine
    # curve.  This preserves level through the seam (unlike fading both ends to
    # silence) and makes the final and first PCM samples exactly continuous.
    seam = min(max(2, int(.025 * SAMPLE_RATE)), len(loop))
    delta = loop[0] - loop[-1]
    ramp = .5 - .5 * np.cos(np.linspace(0.0, math.pi, seam, dtype=np.float32))
    loop[-seam:] += ramp[:, None] * delta[None, :]
    return loop


def render_score(score: dict[str, Any]) -> tuple[np.ndarray, dict[str, Any]]:
    events, tracks, sections = expand_score(score)
    transport = score["transport"]
    seconds_per_beat = 60.0 / float(transport["bpm"])
    total_seconds = int(transport["bars"]) * 4 * seconds_per_beat
    loop_samples = int(round(total_seconds * SAMPLE_RATE))
    total_samples = loop_samples + int(RENDER_TAIL_SECONDS * SAMPLE_RATE)
    audio = np.zeros((total_samples, 2), dtype=np.float32)
    reverb_bus = np.zeros_like(audio)
    delay_bus = np.zeros_like(audio)
    
    seeds = score.get("seeds", {})
    if not seeds:
        global_rng = np.random.default_rng(int(score.get("seed", 0)))
        rngs = {track["id"]: global_rng for track in tracks}
    else:
        seed_mapping = {
            "drums": "drums", "percussion": "percussion", "bass": "sub_bass",
            "harmony": "harmony", "pluck": "glass_pluck", "lead": "prism_lead",
            "atmos": "atmos_pad", "fx": "transitions"
        }
        rngs = {track["id"]: np.random.default_rng(seeds[seed_mapping.get(track["id"], "arrangement")]) for track in tracks}

    track_map = {track["id"]: track for track in tracks}

    ver = str(score.get("schema_version"))
    for event in events:
        track = track_map[event.track]
        if not track.get("audible", True):
            continue
        start = int(event.start * seconds_per_beat * SAMPLE_RATE)
        duration = max(.045, event.duration * seconds_per_beat)
        n = min(int(duration * SAMPLE_RATE), total_samples - start)
        if n <= 0:
            continue
        track_rng = rngs[event.track]
        if event.kind == "note" and event.midi is not None:
            freq = 440.0 * (2 ** ((event.midi - 69) / 12))
            mono = _tonal_voice(
                freq, n, duration, event.wave, track.get("patch_id"),
                dict(track.get("macros", {})), track_rng,
            )
        else:
            mono = _drum(
                event.kind, duration, track_rng, track.get("patch_id"),
                dict(track.get("macros", {})),
            )[:n]
        gain = float(track["volume"]) * event.velocity
        voice = _stereo(mono * gain, float(track["pan"]))
        audio[start:start + n] += voice
        if ver in ("2", "3.0"):
            reverb_bus[start:start + n] += voice * float(track.get("reverb_send", 0.0))
            delay_bus[start:start + n] += voice * float(track.get("delay_send", 0.0))

    mix = score.get("mix", {}) if score.get("schema_version") == 1 else {}
    if score.get("schema_version") == 1:
        delay_bus = audio * float(mix.get("delay", .18))
        reverb_bus = audio * float(mix.get("reverb", .22))
    # Delay is part of the rhythmic grid.  The former fixed 375 ms tap imposed
    # an audible 160-BPM pulse on every score and could overpower the declared
    # tempo.  One quarter-note keeps the echo musical at every style BPM.
    delay = _tempo_delay_frames(seconds_per_beat)
    if delay < len(audio):
        audio[delay:] += delay_bus[:-delay]
    for offset, level in ((.087, .20), (.149, .13), (.271, .08)):
        frames = int(offset * SAMPLE_RATE)
        if frames < len(audio):
            audio[frames:] += reverb_bus[:-frames] * level

    is_loop = bool(transport.get("loop", True))
    if is_loop:
        # A loop export contains one exact musical cycle.  Reverb and delay
        # energy beyond the last bar wraps into bar one instead of creating a
        # silent 1.5-second appendage on every repeat.
        audio = _fold_tail_into_loop(audio, loop_samples)

    master = score.get("master", {}) if score.get("schema_version") == 2 else {}
    width = float(master.get("width", mix.get("width", .72)))
    mid = (audio[:, 0] + audio[:, 1]) * .5
    side = (audio[:, 0] - audio[:, 1]) * .5 * width
    audio[:, 0], audio[:, 1] = mid + side, mid - side
    master_gain = _db_to_gain(float(master.get("gain_db", 0.0))) if master else float(mix.get("master_gain", .82))
    audio *= master_gain
    peak = float(np.max(np.abs(audio))) if audio.size else 1.0
    ceiling = _db_to_gain(-.8) if master else .82
    if peak > ceiling:
        audio *= ceiling / peak
    audio = np.tanh(audio * 1.08).astype(np.float32)
    # One-shot exports get conventional fades. Loop exports retain full level;
    # their circular seam was repaired before mastering above.
    fade = min(int(.025 * SAMPLE_RATE), len(audio) // 4)
    if fade and not is_loop:
        ramp = np.linspace(0, 1, fade, dtype=np.float32)
        audio[:fade] *= ramp[:, None]
        audio[-fade:] *= ramp[::-1, None]
    summary = {
        "title": score["title"], "style": score["style"], "revision": score.get("revision", 1),
        "bpm": transport["bpm"], "bars": transport["bars"], "seconds": round(total_seconds, 3),
        "render_seconds": round(len(audio) / SAMPLE_RATE, 3), "loop": is_loop,
        "events": len(events), "tracks": len(tracks),
        "sections": [{"bar": bar, **section} for bar, section in sections],
        "peak": round(float(np.max(np.abs(audio))), 5),
        "score_hash": hashlib.sha256(json.dumps(score, sort_keys=True).encode("utf-8")).hexdigest()[:16],
        "harmony_grade": court_harmony.grade_score(score, manual_notes(score))["metrics"],
        "compiled_harmony_grade": court_harmony.grade_events(score, events)["metrics"],
        "arrangement_grade": arrangement_interest_report(score, events, sections),
    }
    return audio, summary


def save_wav(path: Path, audio: np.ndarray) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    pcm = np.clip(audio, -1, 1)
    data = (pcm * 32767).astype("<i2")
    with wave.open(str(path), "wb") as handle:
        handle.setnchannels(2)
        handle.setsampwidth(2)
        handle.setframerate(SAMPLE_RATE)
        handle.writeframes(data.tobytes())


def render_to_path(
    score_path: Path,
    out_path: Path | None = None,
    *,
    state_out_path: Path | None = None,
) -> tuple[Path, dict[str, Any], np.ndarray]:
    """Render a score and publish state only to the requested destination.

    Existing workstation callers retain the canonical default.  Workshop and
    other isolated callers pass ``state_out_path`` so this path never touches
    the canonical renderer state.
    """
    score = read_score(score_path)
    errors = validate_score(score)
    if errors:
        raise ValueError("\n".join(errors))
    audio, summary = render_score(score)
    if out_path is None:
        out_path = RENDER_DIR / "current.wav"
    save_wav(out_path, audio)
    summary["render_path"] = str(out_path)
    summary["rendered_at"] = time.strftime("%Y-%m-%dT%H:%M:%S")
    _atomic_json(state_out_path if state_out_path is not None else STATE_PATH, summary)
    return out_path, summary, audio


class AudioTransport:
    def __init__(self) -> None:
        self.audio: np.ndarray | None = None
        self.stream: Any | None = None
        self.looping = False
        self.loop_cursor = 0
        self.output_devices: list[tuple[int, str]] = []
        self.output_device: int | None = None
        self.device_warning = ""
        self.refresh_devices()

    @property
    def device_name(self) -> str:
        for index, name in self.output_devices:
            if index == self.output_device:
                return name
        return "Windows default" if sd is not None else "Audio unavailable"

    def refresh_devices(self) -> None:
        self.output_devices = []
        self.output_device = None
        if sd is None:
            self.device_warning = "sounddevice is unavailable"
            return
        try:
            devices = sd.query_devices()
            self.output_devices = [
                (index, str(device["name"]))
                for index, device in enumerate(devices)
                if int(device["max_output_channels"]) > 0
            ]
            preferred_name = ""
            if AUDIO_CONFIG_PATH.exists():
                configured = json.loads(AUDIO_CONFIG_PATH.read_text(encoding="utf-8"))
                preferred_name = str(configured.get("output_device_name", ""))
            env_preference = os.environ.get("TELEDRA_MUSIC_OUTPUT_DEVICE", "").strip()
            if env_preference:
                preferred_name = env_preference
            if preferred_name:
                match = next((item for item in self.output_devices
                              if preferred_name.casefold() in item[1].casefold()
                              or preferred_name == str(item[0])), None)
                if match:
                    self.output_device = match[0]
            if self.output_device is None:
                current = sd.default.device
                default_output = current[1] if isinstance(current, (list, tuple)) else current
                if isinstance(default_output, int) and any(index == default_output for index, _ in self.output_devices):
                    self.output_device = default_output
            self.device_warning = "" if self.output_devices else "No output devices found"
        except Exception as exc:
            self.device_warning = str(exc)

    def select_device(self, index: int | None) -> None:
        if index is not None and not any(device_index == index for device_index, _ in self.output_devices):
            raise ValueError(f"audio output {index} is unavailable")
        self.stop()
        self.output_device = index
        name = self.device_name
        _atomic_json(AUDIO_CONFIG_PATH, {
            "output_device_index": index,
            "output_device_name": "" if index is None else name,
            "updated_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        })

    def _fill_loop_output(self, outdata: np.ndarray) -> None:
        """Fill one device callback buffer from an immutable circular PCM cycle."""
        if self.audio is None or not len(self.audio):
            outdata.fill(0)
            return
        written = 0
        frames = len(outdata)
        while written < frames:
            available = len(self.audio) - self.loop_cursor
            chunk = min(frames - written, available)
            outdata[written:written + chunk] = self.audio[
                self.loop_cursor:self.loop_cursor + chunk
            ]
            written += chunk
            self.loop_cursor = (self.loop_cursor + chunk) % len(self.audio)

    def _loop_callback(
        self,
        outdata: np.ndarray,
        _frames: int,
        _time_info: Any,
        _status: Any,
    ) -> None:
        self._fill_loop_output(outdata)

    def play(self, audio: np.ndarray, loop: bool = False) -> None:
        if sd is None:
            raise RuntimeError("sounddevice is unavailable; render/export still works")
        self.stop()
        self.audio = np.ascontiguousarray(audio, dtype=np.float32)
        self.looping = bool(loop)
        self.loop_cursor = 0
        if self.looping:
            self.stream = sd.OutputStream(
                samplerate=SAMPLE_RATE,
                device=self.output_device,
                channels=int(self.audio.shape[1]),
                dtype="float32",
                callback=self._loop_callback,
            )
            self.stream.start()
        else:
            sd.play(self.audio, SAMPLE_RATE, device=self.output_device, blocking=False)

    def stop(self) -> None:
        if self.stream is not None:
            stream = self.stream
            self.stream = None
            try:
                stream.stop()
            except Exception:
                pass
            try:
                stream.close()
            except Exception:
                pass
        if sd is not None:
            try:
                sd.stop()
            except Exception:
                pass
        self.looping = False
        self.loop_cursor = 0


class CourtSynthUI:
    """Stable native workstation: truthful controls, safe revisions, focused roll."""

    BG = "#090d13"
    PANEL = "#101722"
    PANEL_2 = "#141e2a"
    GRID = "#1c2a38"
    TEXT = "#eef3f7"
    MUTED = "#7f95a8"
    LIME = "#b6ef4b"

    def __init__(self, score_path: Path, geometry: str | None = None, autoplay: bool = False) -> None:
        if tk is None:
            raise RuntimeError("Tkinter is unavailable on this Python installation")
        self.root = tk.Tk()
        self.root.title("Court Synth // native composition workstation")
        self.root.configure(bg=self.BG)
        self.root.geometry(geometry or "1480x900+36+36")
        self.root.minsize(1120, 720)
        self.root.protocol("WM_DELETE_WINDOW", self.close)
        self.root.grid_columnconfigure(0, weight=1)
        self.root.grid_rowconfigure(1, weight=1)

        self.score_path = score_path
        self.score = read_score(score_path)
        errors = validate_score(self.score)
        if errors:
            raise ValueError("; ".join(errors))
        self.loaded_revision = int(self.score.get("revision", 1))
        self.last_mtime = score_path.stat().st_mtime
        self.transport = AudioTransport()
        self.cached_audio: np.ndarray | None = None
        self.cached_render_revision: int | None = None
        self.last_audible_revision: int | None = None
        self.selected_track = tk.StringVar(value="lead")
        self.show_all_tracks = tk.BooleanVar(value=False)
        self.edit_mode = tk.StringVar(value="select")
        self.viewport_bars = tk.IntVar(value=min(16, int(self.score["transport"]["bars"])))
        self.view_start_bar = 0
        self.status = tk.StringVar(value=f"Ready · schema v{self.score.get('schema_version')} · revision {self.loaded_revision}")
        self.playhead_beat = 0.0
        self._playing = False
        self._play_requested = False
        self._render_pending = False
        self._render_generation = 0
        self._render_lock = threading.Lock()
        self._play_started_at = 0.0
        self._play_timer: str | None = None
        self._roll_coords = (66.0, 38.0, 900.0, 500.0)
        self.flag_buttons: dict[tuple[str, str], tk.Button] = {}
        self.count_labels: dict[str, tk.Label] = {}
        self.volume_vars: dict[str, tk.DoubleVar] = {}
        self.pan_vars: dict[str, tk.DoubleVar] = {}
        self.mix_value_labels: dict[str, tk.Label] = {}
        self.mode_buttons: dict[str, tk.Button] = {}
        self.feedback_buttons: dict[str, tk.Button] = {}
        self.feedback_colors: dict[str, str] = {}
        self.feedback_state = tk.StringVar(value="UNRATED")
        self._build()
        self._set_edit_mode("select")
        self.root.bind("<space>", self._space_transport)
        self.root.bind("<Control-s>", lambda _event: self.save())
        self.root.bind("<Home>", lambda _event: self.rewind())
        self.root.after(900, self._poll_external_score)
        if autoplay:
            self.root.after(250, self.play)

    def _button(self, parent: tk.Widget, text: str, command: Any,
                accent: str | None = None, width: int | None = None) -> tk.Button:
        options: dict[str, Any] = {
            "text": text, "command": command, "bg": "#182331",
            "fg": accent or self.TEXT, "activebackground": "#2a3a4c",
            "activeforeground": "#ffffff", "relief": "flat",
            "font": ("Consolas", 9, "bold"), "padx": 8, "pady": 5,
            "cursor": "hand2",
        }
        if width is not None:
            options["width"] = width
        return tk.Button(parent, **options)

    @staticmethod
    def _short(text: str, limit: int) -> str:
        return text if len(text) <= limit else text[:max(1, limit - 1)] + "…"

    def _patch_name(self, track: dict[str, Any]) -> str:
        patch_id = track.get("patch_id")
        patch = court_instruments.REGISTRY.get(str(patch_id))
        return patch.friendly_name if patch else "Compiler voice"

    def _build(self) -> None:
        self._build_header()
        body = tk.Frame(self.root, bg=self.BG)
        body.grid(row=1, column=0, sticky="nsew", padx=10, pady=(6, 3))
        body.grid_rowconfigure(0, weight=1)
        body.grid_columnconfigure(1, weight=1)
        self._build_track_panel(body)
        self._build_workspace(body)
        self._build_mixer()
        tk.Label(
            self.root, textvariable=self.status, bg="#071018", fg="#72dff7",
            anchor="w", font=("Consolas", 9), padx=10, pady=5,
        ).grid(row=3, column=0, sticky="ew", padx=10, pady=(2, 7))
        self.redraw()

    def _build_header(self) -> None:
        header = tk.Frame(self.root, bg=self.PANEL)
        header.grid(row=0, column=0, sticky="ew")
        header.grid_columnconfigure(1, weight=1)

        identity = tk.Frame(header, bg=self.PANEL)
        identity.grid(row=0, column=0, sticky="w", padx=(15, 8), pady=9)
        self.brand_label = tk.Label(identity, text="COURT SYNTH", bg=self.PANEL, fg=self.TEXT,
                                    font=("Arial", 18, "bold"))
        self.brand_label.pack(anchor="w")
        tk.Label(identity, text="SCORE-DRIVEN NATIVE WORKSTATION", bg=self.PANEL,
                 fg="#6f8da7", font=("Consolas", 7)).pack(anchor="w")

        transport = tk.Frame(header, bg=self.PANEL)
        transport.grid(row=0, column=1, pady=8)
        self._button(transport, "|<", self.rewind, "#d9e0e6", 3).pack(side="left", padx=1)
        self._button(transport, "PLAY", self.play, self.LIME, 5).pack(side="left", padx=1)
        self._button(transport, "STOP", self.stop, "#ff7895", 5).pack(side="left", padx=1)
        self._button(transport, "ARM", self._toggle_arm_selected, "#ff6d88", 4).pack(side="left", padx=1)
        self.loop_button = self._button(transport, "LOOP", self._toggle_loop, "#74d6ff", 5)
        self.loop_button.pack(side="left", padx=(1, 8))
        self.pos_label = tk.Label(transport, text="001.1.00", bg="#0b111a", fg=self.LIME,
                                  font=("Consolas", 11, "bold"), padx=8, pady=7)
        self.pos_label.pack(side="left")
        self.tempo_label = tk.Label(transport, text="", bg=self.PANEL, fg=self.TEXT,
                                    font=("Consolas", 10, "bold"), padx=8)
        self.tempo_label.pack(side="left")

        right = tk.Frame(header, bg=self.PANEL)
        right.grid(row=0, column=2, sticky="e", padx=12, pady=7)
        actions = tk.Frame(right, bg=self.PANEL)
        actions.pack(side="right", padx=(10, 0))
        self._button(actions, "RENDER WAV", self.render_async, "#74d6ff").pack(side="left", padx=2)
        self._button(actions, "SAVE", self.save, "#d9ee55").pack(side="left", padx=2)
        meta = tk.Frame(right, bg=self.PANEL)
        meta.pack(side="right")
        self.title_label = tk.Label(meta, text="", bg=self.PANEL, fg="#f7d778",
                                    font=("Consolas", 10, "bold"), anchor="e")
        self.title_label.pack(anchor="e")
        self.device_button = tk.Menubutton(
            meta, text="", bg=self.PANEL, fg=self.MUTED, activebackground="#1c2937",
            activeforeground=self.TEXT, relief="flat", font=("Consolas", 8), cursor="hand2",
        )
        device_menu = tk.Menu(self.device_button, tearoff=False, bg=self.PANEL_2, fg=self.TEXT)
        device_menu.add_command(label="Windows default", command=lambda: self._select_device(None))
        if self.transport.output_devices:
            device_menu.add_separator()
        for index, name in self.transport.output_devices:
            device_menu.add_command(
                label=f"{index}: {name}",
                command=lambda selected=index: self._select_device(selected),
            )
        self.device_button.configure(menu=device_menu)
        self.device_button.pack(anchor="e")
        self._build_feedback_panel(header)

    def _build_feedback_panel(self, header: tk.Frame) -> None:
        panel = tk.Frame(header, bg="#0c131d")
        panel.grid(row=1, column=0, columnspan=3, sticky="ew", padx=10, pady=(0, 7))
        tk.Label(
            panel,
            text="HUMAN FEEDBACK",
            bg="#0c131d",
            fg="#91a7b9",
            font=("Consolas", 8, "bold"),
            padx=7,
        ).pack(side="left")
        choices = (
            ("like_as_is", "Like (as is)", "#8bd65b"),
            ("like_work_on_it", "Like but work on it", "#c7df63"),
            ("dislike", "Dislike", "#ff6d88"),
            ("dislike_work_on_it", "Dislike but work on it", "#ff9a62"),
        )
        for decision, label, color in choices:
            self.feedback_colors[decision] = color
            button = self._button(
                panel,
                label,
                lambda selected=decision: self._record_human_feedback(selected),
                color,
            )
            button.configure(font=("Consolas", 8, "bold"), pady=3)
            button.pack(side="left", padx=2)
            self.feedback_buttons[decision] = button
        tk.Label(
            panel,
            textvariable=self.feedback_state,
            bg="#0c131d",
            fg="#72dff7",
            font=("Consolas", 8, "bold"),
            padx=9,
        ).pack(side="right")

    def _build_track_panel(self, body: tk.Frame) -> None:
        panel = tk.Frame(body, bg=self.PANEL, width=270)
        panel.grid(row=0, column=0, sticky="nsw", padx=(0, 8))
        panel.grid_propagate(False)
        tk.Label(panel, text="TRACKS", bg=self.PANEL, fg=self.TEXT,
                 font=("Consolas", 11, "bold")).pack(anchor="w", padx=12, pady=(12, 7))
        for number, track in enumerate(resolved_tracks(self.score), 1):
            row = tk.Frame(panel, bg="#111a25", height=42)
            row.pack(fill="x", padx=7, pady=2)
            row.pack_propagate(False)
            tk.Frame(row, bg=track["color"], width=4).pack(side="left", fill="y")
            text = f"{number:02d}  {track['name']}\n     {self._short(self._patch_name(track), 20)}"
            radio = tk.Radiobutton(
                row, variable=self.selected_track, value=track["id"], text=text,
                justify="left", anchor="w", bg="#111a25", fg=track["color"],
                selectcolor="#111a25", activebackground="#111a25",
                activeforeground=track["color"], font=("Consolas", 8, "bold"),
                command=self._select_track,
            )
            radio.pack(side="left", fill="both", expand=True)
            tools = tk.Frame(row, bg="#111a25")
            tools.pack(side="right", padx=3)
            count = tk.Label(tools, text="0", bg="#111a25", fg=self.MUTED, font=("Consolas", 7))
            count.grid(row=0, column=0, columnspan=3)
            self.count_labels[track["id"]] = count
            for column, (flag, label, color) in enumerate((
                ("mute", "M", "#ff6d88"), ("solo", "S", self.LIME), ("arm", "A", "#74d6ff"),
            )):
                button = tk.Button(
                    tools, text=label, command=lambda tid=track["id"], key=flag: self._toggle_track_flag(tid, key),
                    width=2, bg="#182331", fg=color, activebackground="#2a3a4c",
                    activeforeground="#ffffff", relief="flat", font=("Consolas", 7, "bold"),
                    padx=0, pady=0, cursor="hand2",
                )
                button.grid(row=1, column=column, padx=1)
                self.flag_buttons[(track["id"], flag)] = button
        tk.Label(
            panel, text="Select one track for a clear piano roll.\n"
                        "Left-click adds · right-click removes.\n"
                        "Space play/stop · Home rewind.",
            justify="left", bg=self.PANEL, fg=self.MUTED, font=("Consolas", 8),
        ).pack(side="bottom", anchor="w", padx=12, pady=12)

    def _build_workspace(self, body: tk.Frame) -> None:
        workspace = tk.Frame(body, bg="#0c131c")
        workspace.grid(row=0, column=1, sticky="nsew")
        workspace.grid_columnconfigure(0, weight=1)
        workspace.grid_rowconfigure(1, weight=1)

        toolbar = tk.Frame(workspace, bg=self.PANEL)
        toolbar.grid(row=0, column=0, sticky="ew", pady=(0, 3))
        self.focus_label = tk.Label(toolbar, text="", bg=self.PANEL, fg=self.TEXT,
                                    font=("Consolas", 9, "bold"), padx=9)
        self.focus_label.pack(side="left")
        self.all_button = self._button(toolbar, "FOCUS: ONE TRACK", self._toggle_show_all, "#7ce5ff")
        self.all_button.pack(side="left", padx=2, pady=4)
        for mode, label in (("select", "SELECT"), ("draw", "DRAW"), ("erase", "ERASE")):
            button = self._button(
                toolbar,
                label,
                lambda value=mode: self._set_edit_mode(value),
                "#9ce2cb",
            )
            button.pack(side="left", padx=2, pady=4)
            self.mode_buttons[mode] = button
        self._button(toolbar, "◀ 8 BARS", lambda: self._move_view(-8), "#a9b9c7").pack(side="left", padx=(8, 2), pady=4)
        self._button(toolbar, "8 BARS ▶", lambda: self._move_view(8), "#a9b9c7").pack(side="left", padx=2, pady=4)
        tk.Label(toolbar, text="VIEW", bg=self.PANEL, fg=self.MUTED, font=("Consolas", 8)).pack(side="left", padx=(8, 2))
        view_menu = tk.OptionMenu(toolbar, self.viewport_bars, 8, 16, 32, command=self._change_viewport)
        view_menu.configure(bg="#182331", fg=self.TEXT, activebackground="#2a3a4c",
                            activeforeground="#ffffff", relief="flat", highlightthickness=0,
                            font=("Consolas", 8), width=3)
        view_menu["menu"].configure(bg=self.PANEL_2, fg=self.TEXT)
        view_menu.pack(side="left")
        for style, label, color in (
            ("retro_adventure", "RETRO", "#ffc55a"),
            ("spicy_lofi", "SPICY LOFI", "#ff8dd9"),
            ("court_experimental", "EXPERIMENT", "#b79cff"),
        ):
            self._button(toolbar, label, lambda chosen=style: self.apply_style(chosen), color).pack(
                side="right", padx=2, pady=4,
            )

        self.canvas = tk.Canvas(workspace, bg="#091019", highlightthickness=0)
        self.canvas.grid(row=1, column=0, sticky="nsew")
        self.canvas.bind("<Button-1>", self._on_canvas_left)
        self.canvas.bind("<Button-3>", self.remove_manual_note)
        self.canvas.bind("<Configure>", lambda _event: self.redraw())

        automation = tk.Frame(workspace, bg=self.PANEL, height=82)
        automation.grid(row=2, column=0, sticky="ew", pady=(3, 0))
        automation.grid_propagate(False)
        tk.Label(automation, text="SECTION ENERGY · compiler control lane", bg=self.PANEL,
                 fg=self.MUTED, font=("Consolas", 7)).pack(anchor="w", padx=7, pady=(3, 0))
        self.auto_canvas = tk.Canvas(automation, bg="#0a111a", height=52, highlightthickness=0)
        self.auto_canvas.pack(fill="both", expand=True, padx=6, pady=(1, 5))
        self.auto_canvas.bind("<Configure>", lambda _event: self._draw_energy_lane())

    def _build_mixer(self) -> None:
        mixer = tk.Frame(self.root, bg=self.PANEL, height=138)
        mixer.grid(row=2, column=0, sticky="ew", padx=10, pady=(2, 0))
        mixer.grid_propagate(False)
        for track in resolved_tracks(self.score):
            strip = tk.Frame(mixer, bg=self.PANEL_2, width=145, height=126)
            strip.pack(side="left", fill="both", expand=True, padx=2, pady=5)
            strip.pack_propagate(False)
            tk.Label(strip, text=self._short(track["name"].upper(), 17), bg=self.PANEL_2,
                     fg=track["color"], font=("Consolas", 8, "bold")).pack(pady=(5, 0))
            tk.Label(strip, text=self._short(self._patch_name(track), 19), bg=self.PANEL_2,
                     fg=self.MUTED, font=("Consolas", 7)).pack()
            values = tk.Frame(strip, bg=self.PANEL_2)
            values.pack(fill="x", padx=5, pady=(3, 0))
            label = tk.Label(values, text="", bg=self.PANEL_2, fg=self.TEXT, font=("Consolas", 7))
            label.pack()
            self.mix_value_labels[track["id"]] = label
            volume = tk.DoubleVar(value=float(track["volume"]))
            pan = tk.DoubleVar(value=float(track["pan"]))
            self.volume_vars[track["id"]] = volume
            self.pan_vars[track["id"]] = pan
            vol_scale = tk.Scale(
                values, variable=volume, from_=0.0, to=1.25, resolution=.01, orient="horizontal",
                showvalue=False, length=112, bg=self.PANEL_2, fg=self.TEXT, troughcolor="#273747",
                activebackground=track["color"], sliderrelief="flat", highlightthickness=0, bd=0,
            )
            vol_scale.pack()
            vol_scale.bind("<ButtonRelease-1>", lambda _event, tid=track["id"]: self._set_mix_value(tid, "volume"))
            pan_scale = tk.Scale(
                values, variable=pan, from_=-1.0, to=1.0, resolution=.02, orient="horizontal",
                showvalue=False, length=112, bg=self.PANEL_2, fg=self.TEXT, troughcolor="#273747",
                activebackground="#9bb0c2", sliderrelief="flat", highlightthickness=0, bd=0,
            )
            pan_scale.pack()
            pan_scale.bind("<ButtonRelease-1>", lambda _event, tid=track["id"]: self._set_mix_value(tid, "pan"))

        self.master_frame = tk.Frame(mixer, bg="#193038", width=145, height=126)
        self.master_frame.pack(side="left", fill="y", padx=2, pady=5)
        self.master_frame.pack_propagate(False)
        tk.Label(self.master_frame, text="MASTER", bg="#193038", fg=self.LIME,
                 font=("Consolas", 9, "bold")).pack(pady=(8, 3))
        self.master_label = tk.Label(self.master_frame, text="", justify="left", bg="#193038",
                                     fg=self.TEXT, font=("Consolas", 7))
        self.master_label.pack()
        tk.Label(self.master_frame, text="safe ceiling · deterministic", bg="#193038",
                 fg=self.MUTED, font=("Consolas", 6)).pack(side="bottom", pady=8)

    def _score_track_state(self, track_id: str) -> dict[str, Any]:
        if self.score.get("schema_version") == 2:
            track = _v2_track(self.score, track_id)
            if track is None:
                raise ValueError(f"unknown track {track_id}")
            return track.setdefault("mixer", {})
        return self.score.setdefault("track_mix", {}).setdefault(track_id, {})

    def _commit_edit(self, before: dict[str, Any], message: str) -> bool:
        should_resume = self._playing or self._play_requested
        should_render = self._render_pending
        if self.save(bump_revision=True, note=message):
            if should_resume:
                self.stop()
            else:
                self._invalidate_render()
            self.redraw()
            if should_resume:
                self.status.set(
                    f"{message} · saved revision {self.loaded_revision} · rendering playback"
                )
                self._request_render(True)
            elif should_render:
                self.status.set(
                    f"{message} · saved revision {self.loaded_revision} · rendering"
                )
                self._request_render(False)
            else:
                self.status.set(f"{message} · saved revision {self.loaded_revision}")
            return True
        self.score = before
        self._refresh_controls()
        self.redraw()
        return False

    def save(self, bump_revision: bool = False, note: str = "ui-save") -> bool:
        errors = validate_score(self.score)
        if errors:
            self.status.set("Cannot save: " + errors[0])
            return False
        try:
            on_disk = json.loads(self.score_path.read_text(encoding="utf-8"))
            disk_revision = int(on_disk.get("revision", 0))
            if disk_revision != self.loaded_revision:
                self.status.set(
                    f"Save blocked: disk is revision {disk_revision}, editor loaded {self.loaded_revision}. Reloading is safer."
                )
                return False
            if not bump_revision:
                self.status.set(f"Already saved · revision {self.loaded_revision}")
                return True
            if self.score.get("schema_version") == 2:
                self.score["revision"] = self.loaded_revision
                result = court_store.save_atomic(
                    self.score, self.score_path, note=note,
                    expected_revision=self.loaded_revision,
                )
                self.score = result.score
            else:
                candidate = json.loads(json.dumps(self.score))
                candidate["revision"] = self.loaded_revision + 1
                safe_project_id = re.sub(
                    r"[^A-Za-z0-9._-]+", "-", str(on_disk.get("project_id", "court-score"))
                ).strip(".-") or "court-score"
                snapshot = self.score_path.parent / "projects" / (
                    f"{safe_project_id}_rev{self.loaded_revision}_"
                    f"{int(time.time())}_pre-ui-save.json"
                )
                _atomic_json(snapshot, on_disk)
                _atomic_json(self.score_path, candidate)
                self.score = candidate
            self.loaded_revision = int(self.score["revision"])
            self.last_mtime = self.score_path.stat().st_mtime
            self.cached_audio = None
            self.cached_render_revision = None
            return True
        except Exception as exc:
            self.status.set(f"Save failed: {exc}")
            return False

    def _toggle_track_flag(self, track_id: str, flag: str) -> None:
        before = json.loads(json.dumps(self.score))
        state = self._score_track_state(track_id)
        state[flag] = not bool(state.get(flag, False))
        self._commit_edit(before, f"{TRACK_BY_ID[track_id]['name']} {flag} {'on' if state[flag] else 'off'}")

    def _toggle_arm_selected(self) -> None:
        self._toggle_track_flag(self.selected_track.get(), "arm")

    def _toggle_loop(self) -> None:
        before = json.loads(json.dumps(self.score))
        transport = self.score["transport"]
        transport["loop"] = not bool(transport.get("loop", True))
        self._commit_edit(before, f"Loop {'on' if transport['loop'] else 'off'}")

    def _set_mix_value(self, track_id: str, key: str) -> None:
        before = json.loads(json.dumps(self.score))
        state = self._score_track_state(track_id)
        if key == "pan":
            value = _clamp(float(self.pan_vars[track_id].get()), -1.0, 1.0)
            state["pan"] = value
        else:
            value = _clamp(float(self.volume_vars[track_id].get()), 0.0, 1.25)
            if self.score.get("schema_version") == 2:
                base = max(.001, float(TRACK_BY_ID[track_id]["volume"]))
                state["gain_db"] = _clamp(20.0 * math.log10(max(.001, value) / base), -60.0, 12.0)
            else:
                state["volume"] = value
        self._commit_edit(before, f"{TRACK_BY_ID[track_id]['name']} {key} changed")

    def _select_track(self) -> None:
        self.status.set(f"Focused {TRACK_BY_ID[self.selected_track.get()]['name']}")
        self.redraw()

    def _toggle_show_all(self) -> None:
        self.show_all_tracks.set(not self.show_all_tracks.get())
        self.all_button.configure(text="FOCUS: ALL TRACKS" if self.show_all_tracks.get() else "FOCUS: ONE TRACK")
        self.redraw()

    def _change_viewport(self, value: Any) -> None:
        self.viewport_bars.set(int(value))
        self.view_start_bar = min(
            self.view_start_bar,
            max(0, int(self.score["transport"]["bars"]) - self.viewport_bars.get()),
        )
        self.redraw()

    def _move_view(self, amount: int) -> None:
        maximum = max(0, int(self.score["transport"]["bars"]) - self.viewport_bars.get())
        self.view_start_bar = max(0, min(maximum, self.view_start_bar + amount))
        self.redraw()

    def _select_device(self, index: int | None) -> None:
        try:
            self.transport.select_device(index)
            self.status.set(f"Audio output: {self.transport.device_name}")
            self._refresh_header()
        except Exception as exc:
            self.status.set(f"Audio device error: {exc}")

    def _refresh_header(self) -> None:
        harmony = self.score.get("harmony", {})
        bars = int(self.score["transport"]["bars"])
        duration = score_duration_seconds(self.score)
        minutes, seconds = divmod(int(round(duration)), 60)
        title = self._short(str(self.score.get("title", "Untitled")), 34)
        self.brand_label.configure(text=f"COURT SYNTH // {bars}")
        self.title_label.configure(
            text=f"{title} · {harmony.get('tonic', '?')} {harmony.get('mode', '?')} · rev {self.loaded_revision}"
        )
        self.tempo_label.configure(
            text=f"{float(self.score['transport']['bpm']):g} BPM Â· {bars} BARS Â· {minutes}:{seconds:02d}"
        )
        self.device_button.configure(text="OUT · " + self._short(self.transport.device_name, 38))
        loop = bool(self.score["transport"].get("loop", True))
        self.loop_button.configure(bg="#24404a" if loop else "#182331", fg=self.LIME if loop else "#74d6ff")
        self._refresh_feedback()

    def _refresh_feedback(self) -> None:
        try:
            event = court_feedback.current_feedback(self.score_path)
            decision = str(event["decision"]) if event is not None else None
        except court_feedback.FeedbackError:
            event = None
            decision = None
        for code, button in self.feedback_buttons.items():
            active = code == decision
            color = self.feedback_colors[code]
            button.configure(
                bg=color if active else "#182331",
                fg="#071018" if active else color,
            )
        workshop_summary = None
        try:
            global_status = court_workshop.workshop_status()
            visible_jobs = [
                item
                for item in global_status.get("jobs", [])
                if item.get("status") in {"queued", "in_progress", "review_ready", "failed"}
            ]
            if visible_jobs:
                workshop_summary = max(
                    visible_jobs,
                    key=lambda item: (str(item.get("updated_at", "")), str(item.get("event_id", ""))),
                )
        except court_workshop.WorkshopError:
            workshop_summary = None
        if decision:
            label = court_feedback.decision_semantics(decision)["label"]
            if event is not None and bool(event.get("action", {}).get("continue_work")):
                try:
                    job = court_workshop.get_job(str(event["event_id"]))
                except court_workshop.WorkshopError:
                    job = None
                if job is not None:
                    workshop_summary = {
                        "status": job.get("status", "queued"),
                        "completed_passes": sum(
                            item.get("status") == "completed" for item in job.get("passes", [])
                        ),
                        "pass_count": job.get("pass_count", 4),
                    }
            prefix = f"RATED · {str(label).upper()}"
        else:
            prefix = "UNRATED"
        if workshop_summary is not None:
            status = str(workshop_summary.get("status", "queued"))
            label = {
                "in_progress": "RUNNING",
                "review_ready": "REVIEW READY",
                "failed": "NEEDS ATTENTION",
            }.get(status, status.upper())
            completed = int(workshop_summary.get("completed_passes", 0))
            count = int(workshop_summary.get("pass_count", 4))
            prefix += f" · BACK WORKSHOP {label} {completed}/{count}"
        self.feedback_state.set(prefix)

    def _record_human_feedback(self, decision: str) -> None:
        if self.last_audible_revision != self.loaded_revision:
            self.status.set(
                "Feedback not recorded: play this revision first so the vote binds to what you heard."
            )
            return
        try:
            result = court_feedback.record_feedback(
                decision,
                score_path=self.score_path,
                state_path=STATE_PATH,
                render_path=RENDER_DIR / "current.wav",
                expected_revision=self.loaded_revision,
            )
        except court_feedback.FeedbackError as exc:
            self.status.set(f"Feedback not recorded: {exc}")
            return
        workshop_note = ""
        if bool(result.event.get("action", {}).get("continue_work")):
            try:
                job = court_workshop.queue_job(
                    result.event_path,
                    score_path=self.score_path,
                    state_path=STATE_PATH,
                    wav_path=RENDER_DIR / "current.wav",
                )
                workshop_note = (
                    f"; back workshop {job['status']} with {job['pass_count']} passes"
                )
            except court_workshop.WorkshopError as exc:
                # The immutable feedback event remains available for Rust to
                # retry. Do not claim that a workshop job exists when it does not.
                workshop_note = f"; workshop queue needs retry: {exc}"
        self._refresh_feedback()
        action = result.event["action"]
        if decision == "like_as_is":
            outcome = (
                "keeper archived permanently; front stage protected for 10 minutes, "
                "then identity rotation resumes"
            )
        elif decision == "like_work_on_it":
            outcome = "positive keeper parked for four-pass refinement while the front stage explores"
        elif decision == "dislike":
            outcome = "negative example recorded; a new identity was requested"
        else:
            outcome = "weakness recorded; this identity was parked for four-pass repair"
        duplicate = "already recorded" if not result.created else "recorded"
        self.status.set(
            f"{action['label']} {duplicate} · {outcome}{workshop_note} · revision {self.loaded_revision}"
        )

    def _refresh_controls(self) -> None:
        tracks = {track["id"]: track for track in resolved_tracks(self.score)}
        for track_id, track in tracks.items():
            for flag, active_color in (("mute", "#ff6d88"), ("solo", self.LIME), ("arm", "#74d6ff")):
                button = self.flag_buttons.get((track_id, flag))
                if button:
                    active = bool(track.get(flag, False))
                    button.configure(bg=active_color if active else "#182331",
                                     fg="#071018" if active else active_color)
            if track_id in self.volume_vars:
                self.volume_vars[track_id].set(float(track["volume"]))
                self.pan_vars[track_id].set(float(track["pan"]))
                self.mix_value_labels[track_id].configure(
                    text=f"VOL {track['volume']:.2f}   PAN {track['pan']:+.2f}"
                )
        if self.score.get("schema_version") == 2:
            master = self.score.get("master", {})
            chain = " > ".join(item.get("type", "?").upper() for item in master.get("chain", []) if item.get("enabled", True))
            self.master_label.configure(
                text=f"GAIN {float(master.get('gain_db', -3)):+.1f} dB\n"
                     f"WIDTH {float(master.get('width', .85)):.2f}\n{self._short(chain, 20)}"
            )
        else:
            mix = self.score.get("mix", {})
            self.master_label.configure(
                text=f"GAIN {float(mix.get('master_gain', .82)):.2f}\n"
                     f"WIDTH {float(mix.get('width', .72)):.2f}\n"
                     f"REV {float(mix.get('reverb', .22)):.2f} · DLY {float(mix.get('delay', .18)):.2f}"
            )

    def _piano_geometry(self) -> tuple[float, float, float, float]:
        width = max(500, self.canvas.winfo_width())
        height = max(340, self.canvas.winfo_height())
        return 66.0, 38.0, width - 12.0, height - 25.0

    def _set_edit_mode(self, mode: str) -> None:
        if mode not in {"select", "draw", "erase"}:
            return
        self.edit_mode.set(mode)
        for name, button in self.mode_buttons.items():
            button.configure(
                bg="#28433f" if name == mode else "#182331",
                fg=self.LIME if name == mode else "#9ce2cb",
            )
        self.status.set(
            "Select mode: clicks inspect without editing."
            if mode == "select"
            else f"{mode.title()} mode armed · harmony lock is active."
        )
        self.redraw()

    def _on_canvas_left(self, event: Any) -> None:
        mode = self.edit_mode.get()
        if mode == "draw":
            self.add_manual_note(event)
        elif mode == "erase":
            self.remove_manual_note(event)
        else:
            value = self._canvas_to_note(event)
            if value is not None:
                beat, midi = value
                self.status.set(f"Selected {midi_to_note(midi)} at beat {beat:.1f} · no edit made.")

    def redraw(self) -> None:
        if not hasattr(self, "canvas"):
            return
        self._refresh_header()
        self._refresh_controls()
        self.canvas.delete("all")
        left, top, right, bottom = self._piano_geometry()
        self._roll_coords = (left, top, right, bottom)
        selected = self.selected_track.get()
        low, high = court_harmony.track_pitch_range(selected)
        rows = high - low + 1
        row_height = (bottom - top) / rows
        start_bar = self.view_start_bar
        visible_bars = min(self.viewport_bars.get(), int(self.score["transport"]["bars"]) - start_bar)
        visible_bars = max(1, visible_bars)
        start_beat = start_bar * 4.0
        end_beat = (start_bar + visible_bars) * 4.0
        self.focus_label.configure(
            text=f"PIANO ROLL · {TRACK_BY_ID[selected]['name'].upper()} · {self.edit_mode.get().upper()} · bars {start_bar + 1}-{start_bar + visible_bars}"
        )

        for row in range(rows):
            midi = high - row
            y = top + row * row_height
            accidental = midi % 12 in {1, 3, 6, 8, 10}
            self.canvas.create_rectangle(
                left, y, right, y + row_height,
                fill="#0b121b" if accidental else "#101923", outline=self.GRID,
            )
            key_fill = "#202a34" if accidental else "#d6dce1"
            self.canvas.create_rectangle(1, y, left - 4, y + row_height, fill=key_fill, outline="#05080c")
            if midi % 12 == 0:
                self.canvas.create_text(left - 8, y + row_height / 2, text=midi_to_note(midi),
                                        anchor="e", fill="#7f95a8", font=("Consolas", 7))

        for bar_offset in range(visible_bars + 1):
            x = left + (bar_offset / visible_bars) * (right - left)
            absolute_bar = start_bar + bar_offset
            color = "#61778a" if absolute_bar % 4 == 0 else "#2a3a49"
            self.canvas.create_line(x, top, x, bottom, fill=color, width=2 if absolute_bar % 4 == 0 else 1)
            if bar_offset < visible_bars:
                self.canvas.create_text(x + 5, 17, text=str(absolute_bar + 1), anchor="w",
                                        fill="#9eb0bf", font=("Consolas", 8))
            if bar_offset < visible_bars:
                for beat in range(1, 4):
                    beat_x = x + (beat / 4.0) * ((right - left) / visible_bars)
                    self.canvas.create_line(beat_x, top, beat_x, bottom, fill="#1b2936")

        for section, section_bar in _section_offsets(self.score["sections"]):
            section_end = section_bar + int(section["bars"])
            if section_end <= start_bar or section_bar >= start_bar + visible_bars:
                continue
            clipped_start = max(section_bar, start_bar)
            clipped_end = min(section_end, start_bar + visible_bars)
            x1 = left + ((clipped_start - start_bar) / visible_bars) * (right - left)
            x2 = left + ((clipped_end - start_bar) / visible_bars) * (right - left)
            self.canvas.create_rectangle(x1, 1, x2, 31, fill="#193037", outline="#31545a")
            self.canvas.create_text(
                x1 + 6, 16, text=f"{section['name'].upper()} · {section['transform']}",
                anchor="w", fill="#9ce2cb", font=("Consolas", 7, "bold"),
            )

        try:
            events, _tracks, _sections = expand_score(self.score)
        except ValueError as exc:
            self.status.set(f"Score cannot compile: {exc}")
            events = []
        counts = {track["id"]: 0 for track in TRACKS}
        for event in events:
            counts[event.track] += 1
        for track_id, label in self.count_labels.items():
            label.configure(text=str(counts.get(track_id, 0)))

        human = {
            (str(item.get("track")), round(float(item.get("beat", 0)), 3), note_to_midi(str(item.get("pitch"))))
            for item in manual_notes(self.score)
        }
        for event in events:
            if event.midi is None or not low <= event.midi <= high:
                continue
            if not start_beat <= event.start < end_beat:
                continue
            if not self.show_all_tracks.get() and event.track != selected:
                continue
            x = left + ((event.start - start_beat) / (end_beat - start_beat)) * (right - left)
            width = max(3.0, (event.duration / (end_beat - start_beat)) * (right - left))
            y = top + (high - event.midi) * row_height + 1
            track = TRACK_BY_ID[event.track]
            focused = event.track == selected
            fill = track["color"] if focused else "#425669"
            is_human = (event.track, round(event.start, 3), event.midi) in human
            self.canvas.create_rectangle(
                x, y, min(right, x + width), y + max(2, row_height - 2),
                fill=fill, outline="#ffffff" if is_human else (fill if focused else ""),
                width=1,
            )
        self.canvas.create_text(
            right, bottom + 13,
            text="Focused score view · human notes have a white outline",
            anchor="e", fill="#62798e", font=("Consolas", 8),
        )
        self._draw_playhead()
        self._draw_energy_lane()

    def _draw_energy_lane(self) -> None:
        if not hasattr(self, "auto_canvas"):
            return
        canvas = self.auto_canvas
        canvas.delete("all")
        width = max(200, canvas.winfo_width())
        height = max(35, canvas.winfo_height())
        start = self.view_start_bar
        visible = max(1, min(self.viewport_bars.get(), int(self.score["transport"]["bars"]) - start))
        canvas.create_line(5, height - 8, width - 5, height - 8, fill="#283848")
        points: list[float] = []
        section_start = 0
        for section in self.score["sections"]:
            section_end = section_start + int(section["bars"])
            if section_end > start and section_start < start + visible:
                x1 = 5 + ((max(section_start, start) - start) / visible) * (width - 10)
                x2 = 5 + ((min(section_end, start + visible) - start) / visible) * (width - 10)
                energy = _clamp(float(section["energy"]), 0, 1)
                y = height - 8 - energy * (height - 16)
                canvas.create_rectangle(x1, y, x2, height - 8, fill="#17383b", outline="")
                points.extend((x1, y, x2, y))
                canvas.create_text(x1 + 4, y - 3, text=f"{energy:.2f}", anchor="sw",
                                   fill="#78cdb9", font=("Consolas", 6))
            section_start = section_end
        if len(points) >= 4:
            canvas.create_line(*points, fill=self.LIME, width=2)

    def _draw_playhead(self) -> None:
        if not hasattr(self, "canvas"):
            return
        self.canvas.delete("playhead")
        start_beat = self.view_start_bar * 4.0
        end_beat = start_beat + self.viewport_bars.get() * 4.0
        if not start_beat <= self.playhead_beat <= end_beat:
            return
        left, top, right, bottom = self._roll_coords
        x = left + ((self.playhead_beat - start_beat) / max(1.0, end_beat - start_beat)) * (right - left)
        self.canvas.create_line(x, top - 5, x, bottom + 3, fill=self.LIME, width=2, tags="playhead")
        self.canvas.create_polygon(x, top - 8, x - 4, top - 2, x + 4, top - 2,
                                   fill=self.LIME, tags="playhead")

    def _canvas_to_note(self, event: Any) -> tuple[float, int] | None:
        left, top, right, bottom = self._roll_coords
        if not (left <= event.x <= right and top <= event.y <= bottom):
            return None
        visible = max(1, self.viewport_bars.get())
        beat = self.view_start_bar * 4 + ((event.x - left) / (right - left)) * visible * 4
        beat = round(beat * 2) / 2
        low, high = court_harmony.track_pitch_range(self.selected_track.get())
        rows = high - low + 1
        row = min(rows - 1, max(0, int((event.y - top) / (bottom - top) * rows)))
        midi = high - row
        total_beats = int(self.score["transport"]["bars"]) * 4
        return min(max(0.0, beat), total_beats - .5), min(high, max(low, midi))

    def add_manual_note(self, event: Any) -> None:
        value = self._canvas_to_note(event)
        track_id = self.selected_track.get()
        if value is None or track_id in {"drums", "percussion", "fx"}:
            self.status.set("Choose Harmony, Pluck, Bass, Lead or Atmos before drawing a pitched note.")
            return
        beat, raw_midi = value
        midi = court_harmony.nearest_allowed_midi(self.score, track_id, beat, raw_midi)
        pitch = midi_to_note(midi)
        if any(
            str(note.get("track")) == track_id
            and abs(float(note.get("beat", -99)) - beat) <= 1e-6
            and str(note.get("pitch")) == pitch
            for note in manual_notes(self.score)
        ):
            self.status.set(f"{pitch} already exists at beat {beat:.1f}; no duplicate added.")
            return
        before = json.loads(json.dumps(self.score))
        replaced = 0
        if track_id in court_harmony.MONOPHONIC_TRACKS:
            replaced = remove_manual_notes_at_onset(self.score, track_id, beat)
        add_manual_note_to_score(self.score, {
            "track": track_id, "beat": beat, "duration": .5,
            "pitch": pitch, "velocity": .70,
        })
        snap = f" (snapped from {midi_to_note(raw_midi)})" if midi != raw_midi else ""
        replacement = " · replaced prior mono note" if replaced else ""
        self._commit_edit(before, f"Added {pitch}{snap} at beat {beat:.1f}{replacement}")

    def remove_manual_note(self, event: Any) -> None:
        value = self._canvas_to_note(event)
        if value is None:
            return
        beat, midi = value
        before = json.loads(json.dumps(self.score))
        if not remove_manual_note_from_score(
            self.score, self.selected_track.get(), beat, midi_to_note(midi),
        ):
            self.status.set("No human-authored note at that cell.")
            return
        self._commit_edit(before, f"Removed {midi_to_note(midi)} at beat {beat:.1f}")

    def apply_style(self, style: str, confirm: bool = True) -> None:
        if confirm and messagebox is not None:
            accepted = messagebox.askyesno(
                "Apply composition style?",
                "This changes generated form, harmony and tempo. Human notes and mixer choices are preserved.",
                parent=self.root,
            )
            if not accepted:
                return
        before = json.loads(json.dumps(self.score))
        replacement = default_score(style)
        replacement["project_id"] = self.score.get("project_id", replacement["project_id"])
        replacement["revision"] = self.loaded_revision
        replacement["title"] = {
            "retro_adventure": "Vaultlight Procession",
            "spicy_lofi": "Velvet Lanterns",
            "court_experimental": "Fractal Vespers",
        }[style]
        replacement["manual_notes"] = manual_notes(self.score)
        replacement, harmony_stats = normalize_score_harmony(
            replacement,
            source_score=self.score,
        )
        if self.score.get("schema_version") == 2:
            replacement = court_schema.migrate_v1_to_v2(
                replacement, target_revision=self.loaded_revision,
            )
            old_tracks = {track["id"]: track for track in self.score.get("tracks", [])}
            for track in replacement["tracks"]:
                old = old_tracks.get(track["id"], {})
                if old.get("instrument"):
                    track["instrument"] = json.loads(json.dumps(old["instrument"]))
                if old.get("mixer"):
                    track["mixer"] = json.loads(json.dumps(old["mixer"]))
            replacement["master"] = json.loads(json.dumps(self.score.get("master", replacement["master"])))
        else:
            replacement["track_mix"] = json.loads(json.dumps(self.score.get("track_mix", {})))
            replacement["mix"] = json.loads(json.dumps(self.score.get("mix", replacement["mix"])))
        self.score = replacement
        self._commit_edit(
            before,
            f"Applied {style.replace('_', ' ')} · reharmonized {harmony_stats['remapped_pitches']} note(s)",
        )

    def _invalidate_render(self) -> None:
        with self._render_lock:
            self._render_generation += 1
            self._render_pending = False
            self._play_requested = False

    def _request_render(self, play_after: bool) -> None:
        with self._render_lock:
            self._render_generation += 1
            generation = self._render_generation
            self._render_pending = True
            self._play_requested = play_after
            snapshot = json.loads(json.dumps(self.score))
        threading.Thread(
            target=self._render_worker,
            args=(snapshot, generation, play_after),
            daemon=True,
        ).start()

    def _render_worker(
        self,
        snapshot: dict[str, Any],
        generation: int,
        play_after: bool,
    ) -> None:
        try:
            audio, summary = render_score(snapshot)
            with self._render_lock:
                if generation != self._render_generation:
                    return
                path = RENDER_DIR / "current.wav"
                save_wav(path, audio)
                summary["render_path"] = str(path)
                summary["rendered_at"] = time.strftime("%Y-%m-%dT%H:%M:%S")
                _atomic_json(STATE_PATH, summary)
                self.cached_audio = audio
                self.cached_render_revision = int(snapshot.get("revision", 0))

            def complete() -> None:
                with self._render_lock:
                    if generation != self._render_generation:
                        return
                    self._render_pending = False
                    self._play_requested = False
                self.status.set(f"Rendered {summary['events']} events · {summary['seconds']}s · {path.name}")
                if play_after:
                    self._start_playback(audio)
            self.root.after(0, complete)
        except Exception as exc:
            message = str(exc)

            def failed() -> None:
                with self._render_lock:
                    if generation != self._render_generation:
                        return
                    self._render_pending = False
                    self._play_requested = False
                self.status.set(f"Render failed: {message}")

            self.root.after(0, failed)

    def render_async(self) -> None:
        self.status.set("Rendering deterministic arrangement…")
        self._request_render(False)

    def play(self) -> None:
        if self._playing or self._play_requested:
            self.stop()
            return
        if self.cached_audio is None:
            self.status.set("Rendering before playback…")
            self._request_render(True)
        else:
            self._start_playback(self.cached_audio)

    def _start_playback(self, audio: np.ndarray) -> None:
        self._play_requested = False
        try:
            self.transport.play(
                audio,
                loop=bool(self.score["transport"].get("loop", True)),
            )
        except Exception as exc:
            self.status.set(f"Playback error: {exc}")
            return
        self._playing = True
        self._play_started_at = time.monotonic()
        self.playhead_beat = 0.0
        self.last_audible_revision = self.cached_render_revision
        self._refresh_feedback()
        self.status.set(f"Playing through {self.transport.device_name}")
        self._schedule_playback_tick()

    def _schedule_playback_tick(self) -> None:
        if self._play_timer is not None:
            self.root.after_cancel(self._play_timer)
        self._play_timer = self.root.after(80, self._playback_tick)

    def _playback_tick(self) -> None:
        self._play_timer = None
        if not self._playing or self.cached_audio is None:
            return
        elapsed = time.monotonic() - self._play_started_at
        seconds_per_beat = 60.0 / float(self.score["transport"]["bpm"])
        total_beats = int(self.score["transport"]["bars"]) * 4
        audio_seconds = len(self.cached_audio) / SAMPLE_RATE
        is_loop = bool(self.score["transport"].get("loop", True))
        position_seconds = elapsed % audio_seconds if is_loop and audio_seconds > 0 else elapsed
        self.playhead_beat = min(total_beats, position_seconds / seconds_per_beat)
        bar_index = min(int(self.playhead_beat // 4), max(0, int(self.score["transport"]["bars"]) - 1))
        if not self.view_start_bar <= bar_index < self.view_start_bar + self.viewport_bars.get():
            self.view_start_bar = (bar_index // self.viewport_bars.get()) * self.viewport_bars.get()
            self.redraw()
        beat_in_bar = int(self.playhead_beat % 4) + 1
        tick = int((self.playhead_beat % 1) * 100)
        self.pos_label.configure(text=f"{bar_index + 1:03d}.{beat_in_bar}.{tick:02d}")
        self._draw_playhead()
        if elapsed >= audio_seconds and not is_loop:
            self.stop(reset_playhead=False)
            self.status.set("Playback complete")
            return
        self._schedule_playback_tick()

    def stop(self, reset_playhead: bool = True) -> None:
        self._invalidate_render()
        self.transport.stop()
        self._playing = False
        if self._play_timer is not None:
            self.root.after_cancel(self._play_timer)
            self._play_timer = None
        if reset_playhead:
            self.playhead_beat = 0.0
            self.pos_label.configure(text="001.1.00")
            self._draw_playhead()
        self.status.set("Stopped")

    def rewind(self) -> None:
        self.stop()
        self.view_start_bar = 0
        self.redraw()

    def _space_transport(self, _event: Any) -> str:
        if self._playing:
            self.stop()
        else:
            self.play()
        return "break"

    def _poll_external_score(self) -> None:
        try:
            mtime = self.score_path.stat().st_mtime
            if mtime > self.last_mtime + .0001:
                incoming = read_score(self.score_path)
                errors = validate_score(incoming)
                if errors:
                    self.status.set("External score rejected: " + errors[0])
                else:
                    # A live score change must replace what the listener hears,
                    # not only invalidate the cache underneath the old transport.
                    should_resume = self._playing or self._play_requested
                    should_render = self._render_pending
                    if should_resume:
                        self.stop()
                    else:
                        self._invalidate_render()
                    self.score = incoming
                    self.loaded_revision = int(incoming.get("revision", 1))
                    self.last_mtime = mtime
                    self.cached_audio = None
                    self.cached_render_revision = None
                    self.redraw()
                    if should_resume:
                        self.status.set(
                            f"Court update loaded · revision {self.loaded_revision} · rendering playback"
                        )
                        self._request_render(True)
                    elif should_render:
                        self.status.set(
                            f"Court update loaded · revision {self.loaded_revision} · rendering"
                        )
                        self._request_render(False)
                    else:
                        self.status.set(f"Court update loaded · revision {self.loaded_revision}")
        except Exception as exc:
            try:
                self.status.set(f"Score watch warning: {exc}")
            except Exception:
                pass
        finally:
            try:
                # Workshop passes are deliberately off-air, so they do not
                # touch the live score mtime. Refresh their native status on
                # the same bounded poll even when no front-stage score moved.
                self._refresh_feedback()
                self.root.after(900, self._poll_external_score)
            except Exception:
                # Window shutdown legitimately destroys the Tcl scheduler.
                pass

    def close(self) -> None:
        self.stop()
        self.root.destroy()

    def run(self) -> None:
        self.root.mainloop()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Court Synth native score compiler and DAW")
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("validate", "grade", "normalize", "render", "open", "status"):
        child = sub.add_parser(name)
        child.add_argument("score", nargs="?", type=Path, default=DEFAULT_SCORE_PATH)
    sub.choices["normalize"].add_argument("--source-score", type=Path)
    sub.choices["render"].add_argument("--out", type=Path)
    sub.choices["render"].add_argument(
        "--state-out",
        type=Path,
        help="write render state here instead of court_synth/state.json",
    )
    sub.choices["open"].add_argument("--geometry")
    sub.choices["open"].add_argument("--play", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    path: Path = args.score
    if args.command == "validate":
        try:
            errors = validate_score(read_score(path))
        except ValueError as exc:
            print(f"CourtScore invalid: {exc}")
            return 1
        if errors:
            print("CourtScore invalid:")
            print("\n".join(f"- {error}" for error in errors))
            return 1
        try:
            events, _tracks, sections = expand_score(read_score(path))
            interest = arrangement_interest_report(read_score(path), events, sections)
        except ValueError as exc:
            print(f"CourtScore invalid: {exc}")
            return 1
        if not interest["ok"]:
            print("CourtScore musically underdeveloped:")
            print("\n".join(f"- {issue}" for issue in interest["issues"]))
            return 1
        ver = read_score(path).get("schema_version")
        print(f"CourtScore valid: {path} (schema v{ver}; harmony + arrangement gates passed)")
        return 0
    if args.command == "grade":
        try:
            score = read_score(path)
            structural = validate_score(score)
            report = court_harmony.grade_score(score, manual_notes(score))
            report["structural_errors"] = structural
            if not structural:
                events, _tracks, sections = expand_score(score)
                report["compiled"] = court_harmony.grade_events(score, events)
                report["arrangement"] = arrangement_interest_report(score, events, sections)
        except ValueError as exc:
            print(json.dumps({"ok": False, "issues": [str(exc)]}, indent=2))
            return 1
        report["ok"] = (
            not structural
            and report["ok"]
            and report.get("compiled", {}).get("ok", True)
            and report.get("arrangement", {}).get("ok", True)
        )
        print(json.dumps(report, indent=2))
        return 0 if report["ok"] else 1
    if args.command == "normalize":
        try:
            score = read_score(path)
            source_score = read_score(args.source_score) if args.source_score else None
            score, _stats = normalize_score_harmony(
                score,
                source_score=source_score,
            )
        except ValueError as exc:
            print(f"CourtScore normalization failed: {exc}", file=sys.stderr)
            return 1
        print(json.dumps(score, indent=2, ensure_ascii=False))
        return 0
    if args.command == "render":
        try:
            target, summary, _audio = render_to_path(
                path,
                args.out,
                state_out_path=args.state_out,
            )
        except ValueError as exc:
            print(f"Render rejected: {exc}")
            return 1
        print(f"Rendered {summary['title']} ({summary['events']} events, {summary['seconds']}s) -> {target}")
        return 0
    if args.command == "status":
        if STATE_PATH.exists():
            print(STATE_PATH.read_text(encoding="utf-8"))
        else:
            print("No Court Synth render state yet.")
        return 0
    try:
        CourtSynthUI(path, args.geometry, args.play).run()
    except (RuntimeError, ValueError) as exc:
        print(f"Court Synth could not open: {exc}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
