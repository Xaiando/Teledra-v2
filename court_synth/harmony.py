"""Deterministic harmony policy for the active CourtScore pipeline.

The old Python/Strudel composer harness does not grade Court Synth projects.
This module keeps the native editor, autonomous producer, validator, and
renderer on one small set of tonal rules.  It deliberately favors readable
harmony by default; chromatic writing needs an explicit future tension model
instead of entering the score as an unlabelled piano-roll accident.
"""

from __future__ import annotations

import copy
import math
import re
from collections import Counter, defaultdict
from typing import Any, Iterable


NOTE_RE = re.compile(r"^([A-Ga-g])([#b]?)(-?\d+)$")
CHORD_RE = re.compile(
    r"^([A-Ga-g])([#b]?)(maj9|m9|maj7|m7|m6|add9|sus2|sus4|dim|9|7|6|m)?$"
)
NOTE_PC = {
    "C": 0, "C#": 1, "Db": 1, "D": 2, "D#": 3, "Eb": 3,
    "E": 4, "F": 5, "F#": 6, "Gb": 6, "G": 7, "G#": 8,
    "Ab": 8, "A": 9, "A#": 10, "Bb": 10, "B": 11,
}
PC_NAME = ("C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B")
MODE_INTERVALS = {
    "major": (0, 2, 4, 5, 7, 9, 11),
    "natural_minor": (0, 2, 3, 5, 7, 8, 10),
    "dorian": (0, 2, 3, 5, 7, 9, 10),
    "mixolydian": (0, 2, 4, 5, 7, 9, 10),
    "phrygian": (0, 1, 3, 5, 7, 8, 10),
    "harmonic_minor": (0, 2, 3, 5, 7, 8, 11),
}
CHORD_INTERVALS = {
    "": (0, 4, 7), "m": (0, 3, 7), "dim": (0, 3, 6),
    "7": (0, 4, 7, 10), "maj7": (0, 4, 7, 11),
    "m7": (0, 3, 7, 10), "sus2": (0, 2, 7),
    "sus4": (0, 5, 7), "add9": (0, 4, 7, 14),
    "6": (0, 4, 7, 9), "m6": (0, 3, 7, 9),
    "9": (0, 4, 7, 10, 14), "maj9": (0, 4, 7, 11, 14),
    "m9": (0, 3, 7, 10, 14),
}

# Compact upper structures for the three-voice chord lane.  Bass owns the
# root, so seventh/ninth colors can use guide tones instead of compressing
# root, third and seventh into sour semitone clusters.
CHORD_GUIDE_INTERVALS = {
    "": (0, 4, 7), "m": (0, 3, 7), "dim": (0, 3, 6),
    "7": (4, 7, 10), "maj7": (4, 7, 11), "m7": (3, 7, 10),
    "sus2": (0, 2, 7), "sus4": (0, 5, 7), "add9": (0, 4, 14),
    "6": (0, 4, 9), "m6": (0, 3, 9),
    "9": (4, 10, 14), "maj9": (4, 11, 14),
    # Minor ninths keep their fifth in the sustained harmony lane.  The ninth
    # remains available to melody/pluck roles; omitting it here avoids a large
    # inversion jump between common lo-fi maj9 -> m9 moves.
    "m9": (3, 7, 10),
}

# These are musical role ranges, intentionally narrower than the electrical
# limits of many patches.  In particular, a bass patch accepting MIDI 100 does
# not make B5 a useful bass-foundation note.
TRACK_RANGES = {
    "bass": (28, 55),
    "harmony": (48, 76),
    "pluck": (52, 88),
    "lead": (55, 88),
    "atmos": (48, 88),
}
MONOPHONIC_TRACKS = {"bass", "pluck", "lead"}


def note_to_midi(note: str) -> int:
    match = NOTE_RE.match(str(note).strip())
    if not match:
        raise ValueError(f"invalid note {note!r}")
    name = match.group(1).upper() + match.group(2)
    if name not in NOTE_PC:
        raise ValueError(f"unsupported note spelling {note!r}")
    return (int(match.group(3)) + 1) * 12 + NOTE_PC[name]


def midi_to_note(midi: int) -> str:
    value = int(midi)
    return f"{PC_NAME[value % 12]}{value // 12 - 1}"


def parse_chord(symbol: str) -> tuple[int, str, tuple[int, ...]]:
    """Return root pitch class, quality suffix and full chord intervals."""
    match = CHORD_RE.match(str(symbol).strip())
    if not match:
        raise ValueError(f"invalid chord {symbol!r}")
    root_name = match.group(1).upper() + match.group(2)
    if root_name not in NOTE_PC:
        raise ValueError(f"unsupported chord root {symbol!r}")
    quality = match.group(3) or ""
    return NOTE_PC[root_name], quality, CHORD_INTERVALS[quality]


def _chord_parts(symbol: str) -> tuple[int, tuple[int, ...]]:
    root, _quality, intervals = parse_chord(symbol)
    return root, intervals


def chord_guide_intervals(symbol: str) -> tuple[int, ...]:
    _root, quality, _intervals = parse_chord(symbol)
    return CHORD_GUIDE_INTERVALS[quality]


def chord_pitch_classes(symbol: str) -> set[int]:
    root, intervals = _chord_parts(symbol)
    return {(root + interval) % 12 for interval in intervals}


def scale_pitch_classes(score: dict[str, Any]) -> set[int]:
    harmony = score.get("harmony", {})
    tonic = NOTE_PC.get(str(harmony.get("tonic", "")))
    intervals = MODE_INTERVALS.get(str(harmony.get("mode", "")))
    if tonic is None or intervals is None:
        return set()
    return {(tonic + interval) % 12 for interval in intervals}


def _score_chords(score: dict[str, Any]) -> list[str]:
    harmony = score.get("harmony", {})
    chords = harmony.get("chords")
    if isinstance(chords, list):
        return [str(chord) for chord in chords if chord]
    events = harmony.get("chord_events", [])
    if isinstance(events, list):
        return [str(event.get("symbol")) for event in events if isinstance(event, dict) and event.get("symbol")]
    return []


def chord_at_beat(score: dict[str, Any], beat: float) -> str | None:
    harmony = score.get("harmony", {})
    events = harmony.get("chord_events")
    bar = max(0, int(math.floor(float(beat) / 4.0)))
    if isinstance(events, list) and events:
        # v2 describes a one-based chord cycle.  Migration intentionally emits
        # a four-bar cycle for a longer project, so preserve duration as well
        # as symbol when the cycle repeats.
        timeline: list[tuple[int, int, str]] = []
        for event in events:
            if not isinstance(event, dict):
                continue
            try:
                start_value = float(event.get("bar", 0))
                duration_value = float(event.get("duration_bars", 0))
            except (TypeError, ValueError):
                continue
            if (
                not start_value.is_integer()
                or not duration_value.is_integer()
                or start_value < 1
                or duration_value < 1
                or not event.get("symbol")
            ):
                continue
            start = int(start_value)
            duration = int(duration_value)
            timeline.append((start, duration, str(event["symbol"])))
        if not timeline:
            return None
        cycle_bars = max(start + duration - 1 for start, duration, _symbol in timeline)
        bar_one = bar % cycle_bars + 1
        for start, duration, symbol in timeline:
            if start <= bar_one < start + duration:
                return symbol
        # A gap in the declared cycle is invalid; do not invent a fallback
        # chord by cycling the raw event list.
        return None
    chords = _score_chords(score)
    return chords[bar % len(chords)] if chords else None


def progression_coherence_report(score: dict[str, Any]) -> dict[str, Any]:
    """Prove that a v1 chord cycle forms a four/eight-bar tonal sentence.

    Pitch-class legality alone accepts an unordered chord bag.  The live v1
    arranger changes harmony once per bar, so its compact list must establish
    tonic, move through predominant space, and articulate a real dominant
    cadence on the same clock as the groove.
    """
    chords = _score_chords(score)
    harmony = score.get("harmony", {})
    tonic = NOTE_PC.get(str(harmony.get("tonic", "")))
    metrics = {
        "progression_bars": len(chords),
        "tonic_phrase_start_coverage": 0.0,
        "predominant_preparation_coverage": 0.0,
        "dominant_cadence_coverage": 0.0,
        "tonic_resolution_coverage": 0.0,
        "final_tonic": False,
    }
    issues: list[str] = []
    if tonic is None or len(chords) not in {4, 8}:
        issues.append("progression must contain one coherent 4-bar phrase or 8-bar period")
        return {"ok": False, "issues": issues, "metrics": metrics}
    try:
        roots = [parse_chord(chord)[0] for chord in chords]
        pitch_classes = [chord_pitch_classes(chord) for chord in chords]
    except ValueError:
        issues.append("progression contains an invalid chord symbol")
        return {"ok": False, "issues": issues, "metrics": metrics}

    phrase_starts = [0] if len(chords) == 4 else [0, 4]
    start_hits = sum(roots[index] == tonic for index in phrase_starts)
    metrics["tonic_phrase_start_coverage"] = round(
        start_hits / len(phrase_starts), 4
    )

    # ii and iv are the clearest predominant roots; VI is also a conventional
    # minor-key predominant colour (for example Bb before A7 in D minor).
    predominant_roots = {(tonic + 2) % 12, (tonic + 5) % 12, (tonic + 8) % 12}
    predominant_windows = [(1, 3)] if len(chords) == 4 else [(1, 3), (5, 6)]
    predominant_hits = sum(
        any(roots[index] in predominant_roots for index in range(start, end))
        for start, end in predominant_windows
    )
    metrics["predominant_preparation_coverage"] = round(
        predominant_hits / len(predominant_windows), 4
    )

    dominant_root = (tonic + 7) % 12
    leading_tone = (tonic - 1) % 12
    dominant_positions = [3] if len(chords) == 4 else [3, 6]
    dominant_hits = sum(
        roots[index] == dominant_root and leading_tone in pitch_classes[index]
        for index in dominant_positions
    )
    metrics["dominant_cadence_coverage"] = round(
        dominant_hits / len(dominant_positions), 4
    )

    resolutions = (
        [(3, 0)] if len(chords) == 4 else [(3, 4), (6, 7)]
    )
    resolution_hits = sum(
        roots[dominant] == dominant_root
        and leading_tone in pitch_classes[dominant]
        and roots[arrival] == tonic
        for dominant, arrival in resolutions
    )
    metrics["tonic_resolution_coverage"] = round(
        resolution_hits / len(resolutions), 4
    )
    metrics["final_tonic"] = roots[-1] == tonic if len(chords) == 8 else True

    if start_hits != len(phrase_starts):
        issues.append("every harmonic phrase must begin by establishing tonic")
    if predominant_hits != len(predominant_windows):
        issues.append("every harmonic phrase must prepare its cadence with predominant function")
    if dominant_hits != len(dominant_positions):
        issues.append("cadences must use the leading-tone dominant of the declared tonic")
    if resolution_hits != len(resolutions):
        issues.append("every dominant cadence must resolve directly to tonic")
    if len(chords) == 8 and roots[-1] != tonic:
        issues.append("an eight-bar harmonic period must close on tonic")
    return {"ok": not issues, "issues": issues, "metrics": metrics}


def track_pitch_range(track: str) -> tuple[int, int]:
    return TRACK_RANGES.get(str(track), (48, 88))


def _is_strong_beat(beat: float) -> bool:
    rounded = round(float(beat))
    return abs(float(beat) - rounded) <= .08 and rounded % 2 == 0


def allowed_pitch_classes(score: dict[str, Any], track: str, beat: float) -> set[int]:
    scale = scale_pitch_classes(score)
    chord = chord_at_beat(score, beat)
    chord_pcs = chord_pitch_classes(chord) if chord else set()
    if track == "bass" and chord:
        root, intervals = _chord_parts(chord)
        fifth = 7 if 7 in intervals else (6 if 6 in intervals else intervals[-1] % 12)
        return {root, (root + fifth) % 12}
    if track in {"harmony", "atmos"}:
        return chord_pcs or scale
    if track in {"lead", "pluck"} and _is_strong_beat(beat):
        return chord_pcs or scale
    return scale | chord_pcs


def nearest_allowed_midi(score: dict[str, Any], track: str, beat: float, midi: int) -> int:
    low, high = track_pitch_range(track)
    allowed = allowed_pitch_classes(score, track, beat)
    candidates = [value for value in range(low, high + 1) if value % 12 in allowed]
    if not candidates:
        return min(high, max(low, int(midi)))
    chord = chord_at_beat(score, beat)
    chord_pcs = chord_pitch_classes(chord) if chord else set()
    return min(
        candidates,
        key=lambda value: (
            abs(value - int(midi)),
            0 if value % 12 in chord_pcs else 1,
            value,
        ),
    )


def _same_harmony(left: dict[str, Any] | None, right: dict[str, Any]) -> bool:
    if not left:
        return True
    return (
        left.get("harmony", {}).get("tonic") == right.get("harmony", {}).get("tonic")
        and left.get("harmony", {}).get("mode") == right.get("harmony", {}).get("mode")
    )


def _map_scale_degree(midi: int, source: dict[str, Any], target: dict[str, Any]) -> int:
    source_harmony = source.get("harmony", {})
    target_harmony = target.get("harmony", {})
    source_tonic = NOTE_PC.get(str(source_harmony.get("tonic", "")))
    target_tonic = NOTE_PC.get(str(target_harmony.get("tonic", "")))
    source_mode = MODE_INTERVALS.get(str(source_harmony.get("mode", "")))
    target_mode = MODE_INTERVALS.get(str(target_harmony.get("mode", "")))
    if source_tonic is None or target_tonic is None or source_mode is None or target_mode is None:
        return midi
    relative_pc = (midi - source_tonic) % 12
    degree = min(range(7), key=lambda index: min((relative_pc - source_mode[index]) % 12, (source_mode[index] - relative_pc) % 12))
    candidates = [
        value for value in range(max(0, midi - 12), min(127, midi + 12) + 1)
        if (value - target_tonic) % 12 == target_mode[degree]
    ]
    return min(candidates, key=lambda value: abs(value - midi)) if candidates else midi


def normalize_manual_notes(
    score: dict[str, Any],
    notes: Iterable[dict[str, Any]],
    *,
    source_score: dict[str, Any] | None = None,
    pitch_hints: dict[tuple[Any, ...], int] | None = None,
    force_pitch_hints: bool = False,
) -> tuple[list[dict[str, Any]], dict[str, int]]:
    """Return consonance-locked notes plus an inspectable repair summary."""
    source = [copy.deepcopy(note) for note in notes]
    result: list[dict[str, Any]] = []
    exact: set[tuple[str, float, float, str]] = set()
    occupied_spans: dict[str, list[tuple[float, float]]] = defaultdict(list)
    stats = {
        "input_notes": len(source),
        "output_notes": 0,
        "remapped_pitches": 0,
        "range_repairs": 0,
        "contextual_pitch_repairs": 0,
        "authored_register_repairs": 0,
        "dropped_duplicates": 0,
        "dropped_monophonic_collisions": 0,
    }
    harmony_changed = not _same_harmony(source_score, score)
    for note in source:
        track = str(note.get("track", "lead"))
        if track not in TRACK_RANGES:
            continue
        beat = float(note.get("beat", 0.0))
        try:
            original_midi = note_to_midi(str(note.get("pitch", "")))
        except ValueError:
            continue
        mapped = _map_scale_degree(original_midi, source_score, score) if harmony_changed and source_score else original_midi
        repaired = nearest_allowed_midi(score, track, beat, mapped)
        # When a malformed standalone edit already has to change pitch, the
        # arranger may supply the generated note that occupied its exact span.
        # Matching that local phrase context repairs the bad note without
        # creating a new extreme leap or erasing the score's motif identity.
        # Harmony-to-harmony migrations omit hints and retain scale-degree
        # mapping instead.
        hint_key = (track, round(beat, 6), original_midi)
        hint = (pitch_hints or {}).get(
            hint_key,
            (pitch_hints or {}).get((track, round(beat, 6))),
        )
        if hint is not None and (force_pitch_hints or repaired != original_midi):
            contextual = nearest_allowed_midi(score, track, beat, int(hint))
            if force_pitch_hints:
                low, high = track_pitch_range(track)
                target_pitch_classes = {contextual % 12}
                if track == "lead":
                    active_chord = chord_at_beat(score, beat)
                    if active_chord:
                        target_pitch_classes = chord_pitch_classes(active_chord)
                octave_matches = [
                    midi for midi in range(low, high + 1)
                    if midi % 12 in target_pitch_classes
                    and midi % 12 in allowed_pitch_classes(score, track, beat)
                ]
                if octave_matches:
                    contextual = min(
                        octave_matches,
                        key=lambda midi: (
                            abs(midi - repaired),
                            abs(midi - int(hint)),
                        ),
                    )
            stats["contextual_pitch_repairs"] += int(contextual != repaired)
            repaired = contextual
        low, high = track_pitch_range(track)
        if not low <= mapped <= high:
            stats["range_repairs"] += 1
        if repaired != original_midi:
            stats["remapped_pitches"] += 1
        note["pitch"] = midi_to_note(repaired)
        note["track"] = track
        note["beat"] = beat
        duration = float(note.get("duration", 0.5))
        note["duration"] = duration
        key = (track, round(beat, 6), round(duration, 6), note["pitch"])
        if key in exact:
            stats["dropped_duplicates"] += 1
            continue
        if track in MONOPHONIC_TRACKS:
            end = beat + duration
            if any(
                min(end, occupied_end) - max(beat, occupied_start) > 0.02
                for occupied_start, occupied_end in occupied_spans[track]
            ):
                stats["dropped_monophonic_collisions"] += 1
                continue
        exact.add(key)
        if track in MONOPHONIC_TRACKS:
            occupied_spans[track].append((beat, beat + duration))
        result.append(note)
    stats["output_notes"] = len(result)
    return result, stats


def lift_recovered_lead_register(
    score: dict[str, Any],
    notes: Iterable[dict[str, Any]],
    *,
    minimum_midi: int = 60,
) -> tuple[list[dict[str, Any]], int]:
    """Explicit one-time repair for recovered low-register lead artifacts.

    This is intentionally separate from normal note normalization: a future
    composer may deliberately write a low counterline.  Migration callers opt
    in, retain timing/duration/velocity, and record the number of octave lifts
    in lineage.
    """
    repaired_notes = [copy.deepcopy(note) for note in notes]
    lifted = 0
    _low, high = track_pitch_range("lead")
    for note in repaired_notes:
        if str(note.get("track", "lead")) != "lead":
            continue
        try:
            midi = note_to_midi(str(note.get("pitch", "")))
        except ValueError:
            continue
        if midi >= minimum_midi:
            continue
        beat = float(note.get("beat", 0.0))
        candidate = midi
        while candidate < minimum_midi:
            candidate += 12
        while candidate <= high and candidate % 12 not in allowed_pitch_classes(score, "lead", beat):
            candidate += 12
        if candidate > high:
            candidate = nearest_allowed_midi(score, "lead", beat, midi + 12)
        note["pitch"] = midi_to_note(candidate)
        lifted += int(candidate != midi)
    return repaired_notes, lifted


def _motif_notes(score: dict[str, Any]) -> list[str]:
    motif = score.get("motif")
    if isinstance(motif, list):
        return [str(note) for note in motif]
    motifs = score.get("motifs")
    if isinstance(motifs, list) and motifs and isinstance(motifs[0], dict):
        return [str(note) for note in motifs[0].get("notes", [])]
    return []


def grade_score(score: dict[str, Any], notes: Iterable[dict[str, Any]]) -> dict[str, Any]:
    """Grade declared harmony and the manual/delta layer independently."""
    issues: list[str] = []
    scale = scale_pitch_classes(score)
    chords = _score_chords(score)
    chord_ratios: list[float] = []
    for chord in chords:
        try:
            pcs = chord_pitch_classes(chord)
        except ValueError:
            continue
        chord_ratios.append(len(pcs & scale) / max(1, len(pcs)))
    if chord_ratios and min(chord_ratios) < 0.66:
        issues.append("harmony contains a chord with too little connection to the declared key/mode")
    progression = (
        progression_coherence_report(score)
        if score.get("schema_version") == 1 else None
    )
    if progression is not None:
        issues.extend(progression["issues"])

    motif_midis: list[int] = []
    for note in _motif_notes(score):
        try:
            motif_midis.append(note_to_midi(note))
        except ValueError:
            pass
    motif_fit = sum(midi % 12 in scale for midi in motif_midis) / max(1, len(motif_midis))
    if motif_midis and motif_fit < 1.0:
        issues.append("motif contains unlabelled chromatic pitches outside the declared key/mode")

    material = [dict(note) for note in notes]
    parsed: list[tuple[dict[str, Any], int]] = []
    for note in material:
        try:
            parsed.append((note, note_to_midi(str(note.get("pitch", "")))))
        except ValueError:
            continue
    exact_counts = Counter(
        (str(note.get("track")), round(float(note.get("beat", 0.0)), 6),
         round(float(note.get("duration", 0.0)), 6), midi)
        for note, midi in parsed
    )
    duplicate_count = sum(count - 1 for count in exact_counts.values())
    mono_collisions = 0
    for track in MONOPHONIC_TRACKS:
        voice = sorted(
            (
                float(note.get("beat", 0.0)),
                float(note.get("beat", 0.0)) + float(note.get("duration", 0.0)),
            )
            for note, _midi in parsed
            if str(note.get("track")) == track
        )
        for index, (start, end) in enumerate(voice):
            for next_start, next_end in voice[index + 1:]:
                if next_start >= end - 0.02:
                    break
                if min(end, next_end) - max(start, next_start) > 0.02:
                    mono_collisions += 1
    range_failures = 0
    policy_failures = 0
    scale_hits = 0
    chord_hits = 0
    strong_total = 0
    strong_hits = 0
    examples: list[str] = []
    for note, midi in parsed:
        track = str(note.get("track", "lead"))
        beat = float(note.get("beat", 0.0))
        low, high = track_pitch_range(track)
        if not low <= midi <= high:
            range_failures += 1
            if len(examples) < 4:
                examples.append(f"{track} {midi_to_note(midi)}@{beat:g} outside {midi_to_note(low)}..{midi_to_note(high)}")
        allowed = allowed_pitch_classes(score, track, beat)
        if midi % 12 not in allowed:
            policy_failures += 1
            if len(examples) < 4:
                examples.append(f"{track} {midi_to_note(midi)}@{beat:g} conflicts with its active harmony")
        if midi % 12 in scale:
            scale_hits += 1
        chord = chord_at_beat(score, beat)
        chord_pcs = chord_pitch_classes(chord) if chord else set()
        if midi % 12 in chord_pcs:
            chord_hits += 1
        if _is_strong_beat(beat):
            strong_total += 1
            if midi % 12 in chord_pcs:
                strong_hits += 1
    if duplicate_count:
        issues.append(f"manual layer contains {duplicate_count} exact duplicate note(s)")
    if mono_collisions:
        issues.append(f"manual layer contains {mono_collisions} overlapping note pair(s) on monophonic roles")
    if range_failures:
        issues.append(f"manual layer contains {range_failures} note(s) outside their musical role range")
    if policy_failures:
        issues.append(f"manual layer contains {policy_failures} unprepared pitch(es) outside its scale/chord policy")
    metrics = {
        "manual_notes": len(parsed),
        "manual_scale_fit": round(scale_hits / max(1, len(parsed)), 4),
        "manual_chord_fit": round(chord_hits / max(1, len(parsed)), 4),
        "manual_strong_beat_chord_fit": round(strong_hits / max(1, strong_total), 4),
        "manual_duplicates": duplicate_count,
        "manual_monophonic_collisions": mono_collisions,
        "manual_monophonic_overlap_pairs": mono_collisions,
        "manual_range_failures": range_failures,
        "manual_policy_failures": policy_failures,
        "motif_scale_fit": round(motif_fit, 4),
        "minimum_chord_scale_fit": round(min(chord_ratios), 4) if chord_ratios else 0.0,
    }
    if progression is not None:
        metrics.update(progression["metrics"])
    return {"ok": not issues, "issues": issues, "metrics": metrics, "examples": examples}


def grade_events(score: dict[str, Any], events: Iterable[Any]) -> dict[str, Any]:
    """Grade the compiled pitched stream so arranger bugs cannot hide."""
    failures: list[str] = []
    pitched = sorted(
        [event for event in events if getattr(event, "midi", None) is not None],
        key=lambda event: float(event.start),
    )
    policy_failures = 0
    range_failures = 0
    for event in pitched:
        midi = int(event.midi)
        start = float(event.start)
        end = start + float(event.duration)
        low, high = track_pitch_range(str(event.track))
        if not low <= midi <= high:
            range_failures += 1
        checkpoints = [start]
        boundary = (math.floor(start / 4.0) + 1) * 4.0
        while boundary < end - 1e-9:
            checkpoints.append(boundary + 1e-6)
            boundary += 4.0
        if any(
            midi % 12 not in allowed_pitch_classes(score, str(event.track), checkpoint)
            for checkpoint in checkpoints
        ):
            policy_failures += 1
    if range_failures:
        failures.append(f"compiled arrangement has {range_failures} pitched event(s) outside role ranges")
    if policy_failures:
        failures.append(f"compiled arrangement has {policy_failures} event(s) outside active scale/chord policy")
    mono_overlaps = 0
    sustained_clashes = 0
    sustained_tritones = 0
    clash_overlap_beats = 0.0
    tritone_overlap_beats = 0.0
    close_chord_clusters = 0
    close_chord_cluster_beats = 0.0
    chord_onsets: dict[tuple[str, float], list[Any]] = defaultdict(list)
    for event in pitched:
        if str(event.track) in {"harmony", "atmos"} and float(event.duration) > .5:
            chord_onsets[(str(event.track), round(float(event.start), 4))].append(event)
    for voices in chord_onsets.values():
        voices = sorted(voices, key=lambda event: int(event.midi))
        for left, right in zip(voices, voices[1:]):
            if int(right.midi) - int(left.midi) <= 2:
                close_chord_clusters += 1
                close_chord_cluster_beats += min(float(left.duration), float(right.duration))
    for index, left in enumerate(pitched):
        left_end = float(left.start) + float(left.duration)
        for right in pitched[index + 1:]:
            if float(right.start) >= left_end:
                break
            overlap_start = max(float(left.start), float(right.start))
            overlap_end = min(
                left_end,
                float(right.start) + float(right.duration),
            )
            overlap = overlap_end - overlap_start
            # Sub-20 ms-equivalent beat overlaps are deliberate legato tails,
            # not simultaneous mono voices.
            if overlap <= 0.02:
                continue
            if left.track == right.track and left.track in MONOPHONIC_TRACKS:
                mono_overlaps += 1
            if int(left.midi) == int(right.midi) or overlap <= 0.5:
                continue
            interval_class = abs(int(left.midi) - int(right.midi)) % 12
            if interval_class not in {1, 6, 11}:
                continue
            # Inspect every bar crossed by the overlap.  Sampling only its
            # first instant lets a chord-tone exemption leak across the next
            # harmony change.
            cursor = overlap_start
            unplanned_overlap = 0.0
            while cursor < overlap_end - 1e-9:
                segment_end = min(overlap_end, (math.floor(cursor / 4.0) + 1) * 4.0)
                chord = chord_at_beat(score, cursor)
                chord_pcs = chord_pitch_classes(chord) if chord else set()
                if not (
                    int(left.midi) % 12 in chord_pcs
                    and int(right.midi) % 12 in chord_pcs
                ):
                    unplanned_overlap += segment_end - cursor
                cursor = segment_end
            if unplanned_overlap <= 0.5:
                continue
            if interval_class == 6:
                sustained_tritones += 1
                tritone_overlap_beats += unplanned_overlap
            else:
                sustained_clashes += 1
                clash_overlap_beats += unplanned_overlap
    if mono_overlaps:
        failures.append(f"compiled arrangement has {mono_overlaps} overlapping distinct-pitch mono voice pair(s)")
    if sustained_clashes:
        failures.append(
            f"compiled arrangement has {sustained_clashes} unplanned sustained minor-second/minor-ninth clash(es)"
        )
    if sustained_tritones:
        failures.append(
            f"compiled arrangement has {sustained_tritones} unplanned sustained tritone clash(es)"
        )
    if close_chord_clusters:
        failures.append(
            f"compiled arrangement has {close_chord_clusters} cramped sustained chord-tone cluster(s)"
        )
    return {
        "ok": not failures,
        "issues": failures,
        "metrics": {
            "pitched_events": len(pitched),
            "event_range_failures": range_failures,
            "event_policy_failures": policy_failures,
            "event_policy_fit": round((len(pitched) - policy_failures) / max(1, len(pitched)), 4),
            "monophonic_overlap_pairs": mono_overlaps,
            "sustained_m2_m9_clashes": sustained_clashes,
            "sustained_tritone_clashes": sustained_tritones,
            "sustained_m2_m9_overlap_beats": round(clash_overlap_beats, 4),
            "sustained_tritone_overlap_beats": round(tritone_overlap_beats, 4),
            "sustained_close_chord_clusters": close_chord_clusters,
            "sustained_close_chord_cluster_beats": round(close_chord_cluster_beats, 4),
        },
    }
