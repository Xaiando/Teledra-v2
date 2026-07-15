"""CourtScore v2 schema, validation, and deterministic v1->v2 migration.

This is the canonical project model. v2 is the single source of truth for:
- tracks with instrument patches, clips, mixer, automation
- full transport, harmony, sections, motifs
- master chain
- lineage + revision safety

v1 compatibility is preserved for one release cycle.
All migration must be deterministic and preserve human-authored manual_notes exactly.
"""

from __future__ import annotations

import hashlib
import json
import math
import re
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any


MIN_COMPOSITION_SECONDS = 120.0
CANONICAL_MIN_BARS = 64

# -----------------------------
# Core dataclasses (internal model)
# -----------------------------

@dataclass
class Transport:
    bpm: float = 112.0
    meter: list[int] = field(default_factory=lambda: [4, 4])
    bars: int = 64
    swing: float = 0.025
    loop: bool = True
    grid: str = "1/8"
    snap: bool = True

@dataclass
class Harmony:
    tonic: str = "D"
    mode: str = "dorian"
    chord_events: list[dict[str, Any]] = field(default_factory=list)  # [{"bar":1, "duration_bars":1, "symbol":"Dm"}, ...]
    consonance_policy: str = "locked"

@dataclass
class Motif:
    id: str = "main"
    notes: list[str] = field(default_factory=list)
    preserve: bool = True

@dataclass
class Section:
    id: str
    name: str
    start_bar: int
    bars: int
    energy: float
    transform: str

@dataclass
class Note:
    id: str
    beat: float
    duration: float
    pitch: str
    velocity: float = 0.7
    probability: float = 1.0

@dataclass
class Clip:
    id: str
    start_beat: float
    length_beats: float
    notes: list[Note] = field(default_factory=list)

@dataclass
class AutomationPoint:
    beat: float
    value: float

@dataclass
class AutomationLane:
    parameter: str  # e.g. "gain_db", "sends.reverb", "tone"
    points: list[AutomationPoint] = field(default_factory=list)

@dataclass
class InstrumentBinding:
    sound_engine: str = "teledra_synth"
    version: str = "1.0.0"
    patch_id: str = "keys.nocturne_felt"
    macros: dict[str, float] = field(default_factory=dict)

@dataclass
class Mixer:
    mute: bool = False
    solo: bool = False
    arm: bool = False
    gain_db: float = 0.0
    pan: float = 0.0
    width: float = 1.0
    sends: dict[str, float] = field(default_factory=lambda: {"reverb": 0.0, "delay": 0.0})

@dataclass
class ActivityExpectation:
    section_id: str
    mode: str = "REQUIRED"
    minimum_event_count: int = 1

@dataclass
class Track:
    id: str
    name: str
    role: str
    color: str
    instrument_bindings: InstrumentBinding
    enabled: bool = True
    activity_expectations: list[ActivityExpectation] = field(default_factory=list)
    mixer: Mixer = field(default_factory=Mixer)
    clips: list[Clip] = field(default_factory=list)
    automation: list[AutomationLane] = field(default_factory=list)

@dataclass
class Master:
    gain_db: float = -3.0
    width: float = 0.85
    chain: list[dict[str, Any]] = field(default_factory=lambda: [
        {"type": "eq", "enabled": True},
        {"type": "glue", "enabled": True},
        {"type": "tape", "enabled": True},
        {"type": "limiter", "enabled": True, "ceiling_db": -0.8},
    ])

@dataclass
class Lineage:
    parent_revision: int | None = None
    source: str = "human_and_organist"
    preserve: list[str] = field(default_factory=list)
    changed_axes: list[str] = field(default_factory=list)

@dataclass
class CourtScoreV3:
    schema_version: str = "3.0"
    project_id: str = "vaultlight-procession"
    revision: int = 21
    title: str = "Vaultlight Procession"
    style: str = "retro_adventure"
    seeds: dict[str, int] = field(default_factory=lambda: {
        "arrangement": 1001, "drums": 1002, "percussion": 1003,
        "sub_bass": 1004, "harmony": 1005, "glass_pluck": 1006,
        "prism_lead": 1007, "atmos_pad": 1008, "transitions": 1009,
        "humanization": 1010
    })
    transport: Transport = field(default_factory=Transport)
    harmony: Harmony = field(default_factory=Harmony)
    motifs: list[Motif] = field(default_factory=list)
    sections: list[Section] = field(default_factory=list)
    tracks: list[Track] = field(default_factory=list)
    master: Master = field(default_factory=Master)
    lineage: Lineage = field(default_factory=Lineage)

# -----------------------------
# Validation (strict)
# -----------------------------

VALID_STYLES = {"retro_adventure", "spicy_lofi", "gothic_lofi", "chiptune_quest", "ambient_court", "court_experimental"}
VALID_MODES = {"major", "natural_minor", "dorian", "mixolydian", "phrygian", "harmonic_minor"}

NOTE_RE = re.compile(r"^([A-Ga-g])([#b]?)(-?\d+)$")
SAFE_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{1,79}$")
NOTE_PC = {
    "C": 0, "C#": 1, "Db": 1, "D": 2, "D#": 3, "Eb": 3,
    "E": 4, "F": 5, "F#": 6, "Gb": 6, "G": 7, "G#": 8,
    "Ab": 8, "A": 9, "A#": 10, "Bb": 10, "B": 11,
}

def _validate_note(pitch: str) -> None:
    if not NOTE_RE.match(str(pitch).strip()):
        raise ValueError(f"invalid pitch {pitch!r}")

def validate_v3(score: dict[str, Any]) -> list[str]:
    """Return list of error strings (empty = valid)."""
    errors: list[str] = []
    if str(score.get("schema_version")) != "3.0":
        errors.append("schema_version must be '3.0'")
    if not isinstance(score.get("project_id"), str) or not SAFE_ID_RE.fullmatch(score["project_id"]):
        errors.append("project_id must be a safe 2..80 character identifier")
    if not isinstance(score.get("title"), str) or not score["title"].strip():
        errors.append("title required")
    if score.get("style") not in VALID_STYLES:
        errors.append(f"style must be one of {sorted(VALID_STYLES)}")
    if not isinstance(score.get("revision"), int) or score["revision"] < 1:
        errors.append("revision must be positive int")

    seeds = score.get("seeds", {})
    required_seeds = {"arrangement", "drums", "percussion", "sub_bass", "harmony", "glass_pluck", "prism_lead", "atmos_pad", "transitions", "humanization"}
    if not isinstance(seeds, dict) or set(seeds.keys()) != required_seeds:
        errors.append(f"seeds must strictly contain keys {sorted(required_seeds)}")
    elif any(not isinstance(v, int) for v in seeds.values()):
        errors.append("all seeds must be integers")

    transport = score.get("transport", {})
    if not isinstance(transport, dict):
        errors.append("transport object required")
        transport = {}
    bpm = transport.get("bpm")
    if not isinstance(bpm, (int, float)) or not 55 <= bpm <= 170:
        errors.append("transport.bpm 55..170")
    if transport.get("meter") != [4, 4]:
        errors.append("transport.meter must be [4,4] for now")
    bars = transport.get("bars")
    if not isinstance(bars, int) or not 8 <= bars <= 256:
        errors.append("transport.bars 8..256")
    valid_bars = bars if isinstance(bars, int) and bars > 0 else 0
    if (
        isinstance(bpm, (int, float))
        and not isinstance(bpm, bool)
        and float(bpm) > 0
        and valid_bars > 0
    ):
        duration = valid_bars * 4 * 60.0 / float(bpm)
        if duration + 1e-6 < MIN_COMPOSITION_SECONDS:
            errors.append(
                "CourtScore form must last at least "
                f"{MIN_COMPOSITION_SECONDS:.0f} seconds at its declared tempo "
                f"(currently {duration:.3f}s)"
            )
    swing = transport.get("swing", 0.0)
    if not isinstance(swing, (int, float)) or not 0.0 <= float(swing) <= 0.20:
        errors.append("transport.swing 0..0.20")

    harmony = score.get("harmony", {})
    if not isinstance(harmony, dict):
        errors.append("harmony must be an object")
        harmony = {}
    if harmony.get("tonic") not in NOTE_PC:
        errors.append("harmony.tonic must be valid pitch class")
    if harmony.get("mode") not in VALID_MODES:
        errors.append(f"harmony.mode one of {sorted(VALID_MODES)}")
    chord_events = harmony.get("chord_events", [])
    if not isinstance(chord_events, list) or not chord_events:
        errors.append("harmony.chord_events requires at least one chord")
    else:
        valid_timeline: list[tuple[int, int]] = []
        for event in chord_events:
            if not isinstance(event, dict) or not isinstance(event.get("symbol"), str) or not event["symbol"].strip():
                errors.append("every chord event requires a symbol")
                continue
            bar = event.get("bar")
            duration = event.get("duration_bars")
            valid_bar = (
                isinstance(bar, (int, float))
                and not isinstance(bar, bool)
                and float(bar).is_integer()
                and float(bar) >= 1
            )
            valid_duration = (
                isinstance(duration, (int, float))
                and not isinstance(duration, bool)
                and float(duration).is_integer()
                and float(duration) >= 1
            )
            if not valid_bar:
                errors.append("chord event bar must be a one-based integer")
            if not valid_duration:
                errors.append("chord event duration_bars must be a positive integer")
            if valid_bar and valid_duration:
                valid_timeline.append((int(float(bar)), int(float(duration))))
        if len(valid_timeline) == len(chord_events):
            expected_bar = 1
            for start, duration in sorted(valid_timeline):
                if start != expected_bar:
                    errors.append("chord event cycle must be contiguous and begin at bar 1")
                    break
                expected_bar = start + duration

    motifs = score.get("motifs", [])
    if not isinstance(motifs, list) or not motifs:
        errors.append("at least one motif required")
    else:
        for motif in motifs:
            if not isinstance(motif, dict) or not SAFE_ID_RE.fullmatch(str(motif.get("id", ""))):
                errors.append("motif id must be safe and non-empty")
                continue
            notes = motif.get("notes", [])
            if not isinstance(notes, list) or not 1 <= len(notes) <= 64:
                errors.append(f"motif {motif.get('id')} requires 1..64 notes")
                continue
            for pitch in notes:
                try:
                    _validate_note(str(pitch))
                except ValueError as exc:
                    errors.append(str(exc))

    # sections total bars must match transport.bars
    sections = score.get("sections", [])
    if isinstance(sections, list):
        total = sum(s.get("bars", 0) for s in sections
                    if isinstance(s, dict) and isinstance(s.get("bars", 0), int))
        if total != valid_bars:
            errors.append(f"sections total bars ({total}) != transport.bars ({bars})")
        expected_start = 0
        for section in sections:
            if not isinstance(section, dict):
                errors.append("every section must be an object")
                continue
            start_bar = section.get("start_bar", -1)
            if not isinstance(start_bar, int) or start_bar != expected_start:
                errors.append("sections must be contiguous and ordered by start_bar")
            section_bars = section.get("bars")
            if not isinstance(section_bars, int) or section_bars <= 0:
                errors.append("section bars must be a positive integer")
                section_bars = 0
            expected_start += int(section_bars)
            energy = section.get("energy")
            if not isinstance(energy, (int, float)) or not 0.0 <= float(energy) <= 1.0:
                errors.append("section energy must be 0..1")
    else:
        errors.append("sections must be a list")

    # tracks
    tracks = score.get("tracks", [])
    if not isinstance(tracks, list) or len(tracks) == 0:
        errors.append("at least one track required")
        tracks = []
    else:
        ids = set()
        try:
            from .instruments import REGISTRY
        except Exception:
            REGISTRY = {}
        for t in tracks:
            if not isinstance(t, dict):
                errors.append("every track must be an object")
                continue
            tid = t.get("id")
            duplicate = isinstance(tid, str) and tid in ids
            if not isinstance(tid, str) or not SAFE_ID_RE.fullmatch(tid) or duplicate:
                errors.append(f"duplicate or missing track id {tid}")
            if isinstance(tid, str):
                ids.add(tid)
            
            if not isinstance(t.get("enabled", True), bool):
                errors.append(f"track {tid} enabled must be boolean")
            activity_expectations = t.get("activity_expectations", [])
            if not isinstance(activity_expectations, list):
                errors.append(f"track {tid} activity_expectations must be a list")
            else:
                for act in activity_expectations:
                    if not isinstance(act, dict) or "section_id" not in act or "mode" not in act:
                        errors.append(f"track {tid} activity_expectations items must be objects with section_id and mode")
                    elif act.get("mode") not in {"REQUIRED", "OPTIONAL", "INTENTIONALLY_SILENT"}:
                        errors.append(f"track {tid} activity_expectations mode must be REQUIRED, OPTIONAL, or INTENTIONALLY_SILENT")

            instrument_bindings = t.get("instrument_bindings", {})
            if not isinstance(instrument_bindings, dict):
                errors.append(f"track {tid} instrument_bindings must be an object")
                instrument_bindings = {}
            sound_engine = instrument_bindings.get("sound_engine")
            version = instrument_bindings.get("version")
            patch_id = instrument_bindings.get("patch_id")
            if not sound_engine or not version or not patch_id:
                errors.append(f"track {tid} missing sound_engine, version, or patch_id in instrument_bindings")
            elif patch_id not in REGISTRY:
                errors.append(f"track {tid} references unknown patch_id {patch_id}")
            macros = instrument_bindings.get("macros", {})
            if not isinstance(macros, dict) or any(not isinstance(v, (int, float)) or not 0 <= float(v) <= 1 for v in macros.values()):
                errors.append(f"track {tid} instrument_bindings macros must be numeric 0..1")

            mixer = t.get("mixer", {})
            if not isinstance(mixer, dict):
                errors.append(f"track {tid} mixer must be an object")
                mixer = {}
            for flag in ("mute", "solo", "arm"):
                if not isinstance(mixer.get(flag, False), bool):
                    errors.append(f"track {tid} mixer.{flag} must be boolean")
            ranges = (("gain_db", -60.0, 12.0), ("pan", -1.0, 1.0), ("width", 0.0, 2.0))
            for key, low, high in ranges:
                value = mixer.get(key, 0.0 if key != "width" else 1.0)
                if not isinstance(value, (int, float)) or not low <= float(value) <= high:
                    errors.append(f"track {tid} mixer.{key} must be {low:g}..{high:g}")
            sends = mixer.get("sends", {})
            if not isinstance(sends, dict) or any(not isinstance(v, (int, float)) or not 0 <= float(v) <= 1 for v in sends.values()):
                errors.append(f"track {tid} mixer sends must be numeric 0..1")

    total_beats = valid_bars * 4
    for t in tracks:
        if not isinstance(t, dict):
            continue
        clips = t.get("clips", [])
        if not isinstance(clips, list):
            errors.append(f"track {t.get('id')} clips must be a list")
            clips = []
        for clip in clips:
            if not isinstance(clip, dict) or not SAFE_ID_RE.fullmatch(str(clip.get("id", ""))):
                errors.append(f"track {t.get('id')} has an invalid clip id")
                continue
            clip_start = clip.get("start_beat")
            clip_length = clip.get("length_beats")
            if not isinstance(clip_start, (int, float)) or not 0 <= float(clip_start) < max(1, total_beats):
                errors.append(f"clip {clip.get('id')} start_beat is outside the project")
                clip_start = 0
            clip_start_value = float(clip_start) if isinstance(clip_start, (int, float)) else 0.0
            if not isinstance(clip_length, (int, float)) or float(clip_length) <= 0 or clip_start_value + float(clip_length) > total_beats + 1e-6:
                errors.append(f"clip {clip.get('id')} length is invalid")
                clip_length_value = 0.0
            else:
                clip_length_value = float(clip_length)
            notes = clip.get("notes", [])
            if not isinstance(notes, list):
                errors.append(f"clip {clip.get('id')} notes must be a list")
                notes = []
            note_ids: set[str] = set()
            for n in notes:
                if not isinstance(n, dict):
                    errors.append(f"clip {clip.get('id')} contains a non-object note")
                    continue
                try:
                    _validate_note(n.get("pitch", ""))
                except Exception as e:
                    errors.append(str(e))
                note_id = str(n.get("id", ""))
                if not SAFE_ID_RE.fullmatch(note_id) or note_id in note_ids:
                    errors.append(f"clip {clip.get('id')} has an invalid note id")
                note_ids.add(note_id)
                beat = n.get("beat")
                duration = n.get("duration")
                velocity = n.get("velocity", .7)
                probability = n.get("probability", 1.0)
                beat_value = float(beat) if isinstance(beat, (int, float)) else -1.0
                duration_value = float(duration) if isinstance(duration, (int, float)) else -1.0
                if not isinstance(beat, (int, float)) or not 0 <= beat_value < max(clip_length_value, 1e-9):
                    errors.append(f"note {n.get('id')} beat is outside its clip")
                if not isinstance(duration, (int, float)) or duration_value <= 0 or beat_value + duration_value > clip_length_value + 1e-6:
                    errors.append(f"note {n.get('id')} duration is invalid")
                if not isinstance(velocity, (int, float)) or not 0 <= float(velocity) <= 1:
                    errors.append(f"note {n.get('id')} velocity must be 0..1")
                if not isinstance(probability, (int, float)) or not 0 <= float(probability) <= 1:
                    errors.append(f"note {n.get('id')} probability must be 0..1")
        automation = t.get("automation", [])
        if not isinstance(automation, list):
            errors.append(f"track {t.get('id')} automation must be a list")
            automation = []
        for lane in automation:
            if not isinstance(lane, dict) or not isinstance(lane.get("parameter"), str):
                errors.append(f"track {t.get('id')} has an invalid automation lane")
                continue
            last_beat = -1.0
            points = lane.get("points", [])
            if not isinstance(points, list):
                errors.append(f"track {t.get('id')} automation points must be a list")
                points = []
            for point in points:
                beat = point.get("beat") if isinstance(point, dict) else None
                value = point.get("value") if isinstance(point, dict) else None
                if not isinstance(beat, (int, float)) or not 0 <= float(beat) <= total_beats or float(beat) < last_beat:
                    errors.append(f"track {t.get('id')} automation beats must be ordered within the project")
                else:
                    last_beat = float(beat)
                if not isinstance(value, (int, float)) or not math.isfinite(float(value)) or not -120 <= float(value) <= 120:
                    errors.append(f"track {t.get('id')} automation value is invalid")

    master = score.get("master", {})
    if not isinstance(master, dict):
        errors.append("master must be an object")
    else:
        gain_db = master.get("gain_db", -3.0)
        width = master.get("width", .85)
        if not isinstance(gain_db, (int, float)) or not -60 <= float(gain_db) <= 12:
            errors.append("master.gain_db must be -60..12")
        if not isinstance(width, (int, float)) or not 0 <= float(width) <= 2:
            errors.append("master.width must be 0..2")
        if not isinstance(master.get("chain", []), list):
            errors.append("master.chain must be a list")

    return list(dict.fromkeys(errors))  # dedup


# -----------------------------
# v1 -> v2 deterministic migration (preserves human edits)
# -----------------------------

V1_TO_V2_DEFAULT_PATCHES = {
    "drums": "kit.mechanical_court",
    "percussion": "kit.velvet_lofi",
    "bass": "bass.substructure",
    "harmony": "keys.nocturne_felt",
    "pluck": "pluck.glass_current",
    "lead": "lead.ember_superwave",
    "atmos": "pad.aurora_choir",
    "fx": "fx.riser",
}

def _v1_track_defaults(track_id: str, name: str, role: str, color: str) -> dict[str, Any]:
    patch = V1_TO_V2_DEFAULT_PATCHES.get(track_id, "keys.nocturne_felt")
    return {
        "id": track_id,
        "name": name,
        "role": role,
        "color": color,
        "instrument_bindings": {"sound_engine": "teledra_synth", "version": "1.0.0", "patch_id": patch, "macros": {"tone": 0.6, "character": 0.4}},
        "mixer": {
            "mute": False, "solo": False, "arm": False,
            "gain_db": -3.0 if track_id != "drums" else -1.0,
            "pan": 0.0,
            "width": 0.6 if track_id in ("atmos", "harmony") else 1.0,
            "sends": {"reverb": 0.25 if track_id in ("atmos", "harmony") else 0.08, "delay": 0.1}
        },
        "clips": [],
        "automation": []
    }

def migrate_v1_to_v3(v1: dict[str, Any], target_revision: int | None = None) -> dict[str, Any]:
    """Deterministic lift of v1 CourtScore to v3 shape.
    CRITICAL: human manual_notes are turned into explicit clips/notes and preserved.
    """
    if v1.get("schema_version") != 1:
        raise ValueError(f"expected schema_version 1, got {v1.get('schema_version')!r}")

    rev = target_revision if target_revision is not None else int(v1.get("revision", 1)) + 1
    title = v1.get("title", "Untitled")
    style = v1.get("style", "retro_adventure")
    seed = int(v1.get("seed", 48358))
    project_id = v1.get("project_id", "migrated-project").replace("court-synth-default", "vaultlight-procession")

    transport_v1 = v1.get("transport", {})
    bpm = float(transport_v1.get("bpm", 112))
    source_bars = int(transport_v1.get("bars", 32))
    duration_bars = math.ceil(MIN_COMPOSITION_SECONDS * bpm / (4 * 60.0) / 4) * 4
    target_bars = max(source_bars, CANONICAL_MIN_BARS, duration_bars)
    transport = {
        "bpm": bpm,
        "meter": [4, 4],
        "bars": target_bars,
        "swing": float(transport_v1.get("swing", 0.025)),
        "loop": bool(transport_v1.get("loop", True)),
        "grid": "1/8",
        "snap": True,
    }
    bars = transport["bars"]

    harmony_v1 = v1.get("harmony", {})
    chords = harmony_v1.get("chords", ["Dm", "Am", "Bdim", "G"])
    chord_events = [{"bar": i + 1, "duration_bars": 1, "symbol": c} for i, c in enumerate(chords)]
    harmony = {
        "tonic": harmony_v1.get("tonic", "D"),
        "mode": harmony_v1.get("mode", "dorian"),
        "chord_events": chord_events,
        "consonance_policy": "locked",
    }

    motif_notes = v1.get("motif", ["A4", "F4", "E4", "D4", "A3", "D4", "F4", "A4"])
    motifs = [{"id": "main", "notes": motif_notes, "preserve": True}]

    # sections
    sections_v1 = v1.get("sections", [])
    sections = []
    bar = 0
    for i, sec in enumerate(sections_v1):
        sec_bars = int(sec.get("bars", 4))
        sections.append({
            "id": sec.get("name", f"sec{i}"),
            "name": sec.get("name", f"Section {i}"),
            "start_bar": bar,
            "bars": sec_bars,
            "energy": float(sec.get("energy", 0.5)),
            "transform": sec.get("transform", "forward"),
        })
        bar += sec_bars
    extension_index = 1
    extension_transforms = ("forward", "sequence", "call_response", "recombine")
    while bar < bars:
        remaining = bars - bar
        sec_bars = 8 if remaining >= 8 else remaining
        if sec_bars <= 0 or sec_bars % 4:
            raise ValueError("v1 section map cannot be extended on the four-bar phrase clock")
        previous_energy = float(sections[-1]["energy"]) if sections else 0.4
        energy = max(
            0.18,
            min(0.88, previous_energy + (0.12 if extension_index % 2 else -0.08)),
        )
        name = f"extended_return_{extension_index}"
        sections.append({
            "id": name,
            "name": name.replace("_", " ").title(),
            "start_bar": bar,
            "bars": sec_bars,
            "energy": energy,
            "transform": extension_transforms[(extension_index - 1) % len(extension_transforms)],
        })
        bar += sec_bars
        extension_index += 1

    # base tracks (8 roles from v1)
    v1_track_meta = [
        ("drums", "Drums", "pulse", "#ff5c7c"),
        ("percussion", "Percussion", "motion", "#ffae57"),
        ("bass", "Sub Bass", "foundation", "#69a9ff"),
        ("harmony", "Harmony", "middle", "#b796ff"),
        ("pluck", "Glass Pluck", "rhythm", "#58e2ca"),
        ("lead", "Prism Lead", "motif", "#ff82df"),
        ("atmos", "Atmos Pad", "air", "#64d4ff"),
        ("fx", "Transitions", "transition", "#d9ee55"),
    ]
    tracks = [_v1_track_defaults(tid, nm, role, col) for tid, nm, role, col in v1_track_meta]

    # Migrate manual_notes -> clips on the matching track
    # Group by track
    manual_notes = v1.get("manual_notes", [])
    total_beats = bars * 4
    clips_by_track: dict[str, list[dict]] = {t["id"]: [] for t in tracks}

    for idx, mn in enumerate(manual_notes):
        track_id = str(mn.get("track", "lead"))
        if track_id not in clips_by_track:
            raise ValueError(f"manual note {idx} references unknown track {track_id!r}")
        beat = float(mn.get("beat", 0))
        if not 0 <= beat < total_beats:
            raise ValueError(f"manual note {idx} beat {beat} is outside the project")
        dur = float(mn.get("duration", 0.5))
        if dur <= 0 or beat + dur > total_beats:
            raise ValueError(f"manual note {idx} duration crosses the project boundary")
        pitch = str(mn.get("pitch", "D4"))
        vel = float(mn.get("velocity", 0.7))
        note = {
            "id": f"migrated-n{idx}",
            "beat": beat,
            "duration": dur,
            "pitch": pitch,
            "velocity": vel,
            "probability": 1.0,
        }
        # Put into a single clip spanning the project for simplicity (real clips later)
        # For migration we create/append to a "main" clip per track
        if not clips_by_track[track_id]:
            clips_by_track[track_id] = [{
                "id": f"clip-migrated-{track_id}",
                "start_beat": 0,
                "length_beats": float(total_beats),
                "notes": []
            }]
        clips_by_track[track_id][0]["notes"].append(note)

    for t in tracks:
        t["clips"] = clips_by_track.get(t["id"], [])

    # Master from mix if present
    mix = v1.get("mix", {})
    master = {
        "gain_db": -3.0,
        "width": float(mix.get("width", 0.72)),
        "chain": [
            {"type": "eq", "enabled": True},
            {"type": "glue", "enabled": True},
            {"type": "tape", "enabled": True},
            {"type": "limiter", "enabled": True, "ceiling_db": -0.8},
        ],
    }

    lineage = {
        "parent_revision": int(v1.get("revision", 20)),
        "source": v1.get("lineage", {}).get("source", "court_synth_default"),
        "preserve": ["human_notes", "main_motif", "tonal_center"],
        "changed_axes": ["instrumentation", "structure"],
    }

    v3: dict[str, Any] = {
        "schema_version": "3.0",
        "project_id": project_id,
        "revision": rev,
        "title": title,
        "style": style,
        "seeds": {
            "arrangement": seed, "drums": seed + 1, "percussion": seed + 2,
            "sub_bass": seed + 3, "harmony": seed + 4, "glass_pluck": seed + 5,
            "prism_lead": seed + 6, "atmos_pad": seed + 7, "transitions": seed + 8,
            "humanization": seed + 9
        },
        "transport": transport,
        "harmony": harmony,
        "motifs": motifs,
        "sections": sections,
        "tracks": tracks,
        "master": master,
        "lineage": lineage,
    }

    errs = validate_v3(v3)
    if errs:
        raise ValueError("Migration produced invalid v3: " + "; ".join(errs))
    return v3


def load_and_migrate(path: Path) -> dict[str, Any]:
    """Read whatever is at path (v1, v2, or v3) and return a valid v3 dict."""
    raw = json.loads(path.read_text(encoding="utf-8"))
    if str(raw.get("schema_version")) == "3.0":
        errs = validate_v3(raw)
        if errs:
            raise ValueError("Invalid v3: " + "; ".join(errs))
        return raw
    if raw.get("schema_version") == 1 or raw.get("schema_version") == 2:
        return migrate_v1_to_v3(raw)
    raise ValueError(f"Unsupported schema_version {raw.get('schema_version')}")


def to_json(score: dict[str, Any] | CourtScoreV3) -> str:
    if isinstance(score, CourtScoreV3):
        score = asdict(score)
    return json.dumps(score, indent=2, ensure_ascii=False) + "\n"


def write_atomic(path: Path, score: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(to_json(score), encoding="utf-8")
    import os
    os.replace(tmp, path)


# Convenience for tests
def migrate_current_v1_snapshot() -> dict[str, Any]:
    """Helper used by tests: migrate the known rev21 fixture."""
    # Caller provides the path; this is pure for the test to call with snapshot.
    raise NotImplementedError("Use migrate_v1_to_v3(json) directly in tests")
