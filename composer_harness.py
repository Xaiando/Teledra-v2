"""Music-theory planning gate for Teledra's autonomous composers.

The audio verifier answers "does it run?".  This module answers the earlier
question: "did the composer make deliberate musical choices that are likely to
read as a piece rather than a pile of individually valid sounds?"
"""

from __future__ import annotations

import math
import re
from typing import Any


STYLE_PROFILES: dict[str, dict[str, Any]] = {
    "retro_adventure": {
        "bpm": (88, 152),
        "modes": {"major", "natural_minor", "dorian", "mixolydian", "phrygian", "harmonic_minor"},
        "swing": (0.0, 0.18),
        "minimum_step_ratio": 0.42,
        "description": "memorable chip-like motif, economical harmony, clear pulse, and quest-shaped form",
    },
    "spicy_lofi": {
        "bpm": (64, 102),
        "modes": {"major", "natural_minor", "dorian", "mixolydian"},
        "swing": (0.08, 0.34),
        "minimum_step_ratio": 0.32,
        "description": "warm pocket, tasteful extensions, syncopation, breathing room, and controlled grit",
    },
    "court_experimental": {
        "bpm": (48, 190),
        "modes": {"major", "natural_minor", "dorian", "mixolydian", "phrygian", "harmonic_minor"},
        "swing": (0.0, 0.45),
        "minimum_step_ratio": 0.2,
        "description": "deliberate experiment with an explicit consonance/tension/resolution policy",
    },
}

MODE_INTERVALS = {
    "major": (0, 2, 4, 5, 7, 9, 11),
    "natural_minor": (0, 2, 3, 5, 7, 8, 10),
    "dorian": (0, 2, 3, 5, 7, 9, 10),
    "mixolydian": (0, 2, 4, 5, 7, 9, 10),
    "phrygian": (0, 1, 3, 5, 7, 8, 10),
    "harmonic_minor": (0, 2, 3, 5, 7, 8, 11),
}

PITCH_CLASSES = {
    "C": 0, "B#": 0, "C#": 1, "DB": 1, "D": 2, "D#": 3, "EB": 3,
    "E": 4, "FB": 4, "E#": 5, "F": 5, "F#": 6, "GB": 6, "G": 7,
    "G#": 8, "AB": 8, "A": 9, "A#": 10, "BB": 10, "B": 11, "CB": 11,
}

NOTE = re.compile(r"^([A-Ga-g])([#b]?)(-?\d+)$")

EVENT_KINDS = {"note", "drum", "fx"}
ROLE_ALIASES = {
    "foundation": "bass",
    "sub": "bass",
    "chords": "harmony",
    "chord": "harmony",
    "pad": "harmony",
    "body": "harmony",
    "melody": "lead",
    "focus": "lead",
    "counterline": "motion",
    "counter": "motion",
    "arp": "motion",
    "arpeggio": "motion",
    "drums": "percussion",
    "kick": "percussion",
    "snare": "percussion",
    "hat": "percussion",
    "air": "texture",
    "transition": "texture",
    "transitions": "texture",
    "atmosphere": "texture",
}


def _issue(code: str, message: str, **details: Any) -> dict[str, Any]:
    return {"code": code, "message": message, **details}


def note_to_midi(note: Any) -> int | None:
    if isinstance(note, int) and not isinstance(note, bool):
        return note if 0 <= note <= 127 else None
    if not isinstance(note, str):
        return None
    match = NOTE.fullmatch(note.strip())
    if not match:
        return None
    name = (match.group(1) + match.group(2)).upper()
    pitch = PITCH_CLASSES.get(name)
    if pitch is None:
        return None
    return (int(match.group(3)) + 1) * 12 + pitch


def _register_range(value: Any) -> tuple[int, int] | None:
    if not isinstance(value, (list, tuple)) or len(value) != 2:
        return None
    try:
        low, high = int(value[0]), int(value[1])
    except (TypeError, ValueError):
        return None
    return (low, high) if 0 <= low < high <= 9 else None


def _finite_number(value: Any) -> float | None:
    try:
        number = float(value)
    except (TypeError, ValueError):
        return None
    return number if math.isfinite(number) else None


def _canonical_role(value: Any) -> str:
    role = str(value or "").strip().lower().replace(" ", "_")
    return ROLE_ALIASES.get(role, role)


def _median(values: list[int]) -> float:
    ordered = sorted(values)
    middle = len(ordered) // 2
    if len(ordered) % 2:
        return float(ordered[middle])
    return (ordered[middle - 1] + ordered[middle]) * 0.5


def _correlation(left: list[float], right: list[float]) -> float:
    if len(left) != len(right) or len(left) < 2:
        return 0.0
    left_mean = sum(left) / len(left)
    right_mean = sum(right) / len(right)
    left_delta = [value - left_mean for value in left]
    right_delta = [value - right_mean for value in right]
    denominator = math.sqrt(
        sum(value * value for value in left_delta)
        * sum(value * value for value in right_delta)
    )
    if denominator <= 1e-12:
        return 0.0
    return sum(a * b for a, b in zip(left_delta, right_delta)) / denominator


def _normalise_token(value: Any) -> str:
    return str(value or "").strip().lower().replace(" ", "_").replace("-", "_")


def evaluate_composer_plan(
    plan: Any,
    *,
    bpm: Any = None,
    section_count: int = 0,
) -> dict[str, Any]:
    """Return hard issues, advisories, and explainable musical metrics."""

    issues: list[dict[str, Any]] = []
    advisories: list[dict[str, Any]] = []
    metrics: dict[str, Any] = {}
    if not isinstance(plan, dict):
        return {
            "ok": False,
            "issues": [_issue(
                "missing_composer_plan",
                "Declare TELEDRA_COMPOSER so harmony, motif, rhythm, registers, and tension can be checked before playback.",
            )],
            "advisories": [],
            "metrics": {},
        }

    profile_name = str(plan.get("style_profile", "")).strip().lower()
    profile = STYLE_PROFILES.get(profile_name)
    if profile is None:
        issues.append(_issue(
            "unknown_style_profile",
            "style_profile must be retro_adventure, spicy_lofi, or court_experimental.",
            value=profile_name,
        ))
        profile = STYLE_PROFILES["court_experimental"]
    metrics["style_profile"] = profile_name

    try:
        bpm_value = float(bpm)
    except (TypeError, ValueError):
        bpm_value = 0.0
    bpm_low, bpm_high = profile["bpm"]
    if not bpm_low <= bpm_value <= bpm_high:
        issues.append(_issue(
            "style_tempo_mismatch",
            f"{profile_name or 'selected'} profile expects {bpm_low}-{bpm_high} BPM.",
            bpm=bpm_value,
        ))

    root_name = str(plan.get("tonal_center", "")).strip().upper()
    mode = str(plan.get("mode", "")).strip().lower().replace(" ", "_")
    root_pc = PITCH_CLASSES.get(root_name)
    intervals = MODE_INTERVALS.get(mode)
    if root_pc is None:
        issues.append(_issue("invalid_tonal_center", "Use a concrete tonal_center such as A, C#, or Eb."))
    if intervals is None or mode not in profile["modes"]:
        issues.append(_issue(
            "style_mode_mismatch",
            f"Mode '{mode}' is not supported by the selected style profile.",
            supported=sorted(profile["modes"]),
        ))
    scale_pcs = set() if root_pc is None or intervals is None else {(root_pc + value) % 12 for value in intervals}

    progression = plan.get("progression_degrees")
    if not isinstance(progression, (list, tuple)) or len(progression) < 4:
        issues.append(_issue("weak_harmonic_plan", "Plan at least four scale degrees for a repeatable harmonic journey."))
    else:
        invalid_degrees = [value for value in progression if not isinstance(value, int) or not 1 <= value <= 7]
        if invalid_degrees:
            issues.append(_issue("invalid_scale_degree", "progression_degrees must contain integers 1-7.", values=invalid_degrees))
        if len(set(progression)) < 3:
            issues.append(_issue("static_harmony", "Use at least three distinct harmonic roots; one or two repeated roots sound inert."))
        if progression[0] != 1 and progression[-1] != 1:
            advisories.append(_issue("weak_home_signal", "Let the progression begin or resolve on degree 1 so its tonal home reads clearly."))

    intentional = {str(value).strip().upper() for value in plan.get("intentional_tensions", []) if isinstance(value, str)}
    chord_voicings = plan.get("chord_voicings")
    off_scale: list[str] = []
    if not isinstance(chord_voicings, (list, tuple)) or len(chord_voicings) < 4:
        issues.append(_issue("missing_chord_voicings", "Declare four or more chord_voicings using concrete note names."))
    else:
        for chord in chord_voicings:
            if not isinstance(chord, (list, tuple)) or not 2 <= len(chord) <= 5:
                issues.append(_issue("unbalanced_chord", "Each chord voicing needs 2-5 notes; giant clusters are usually mush."))
                continue
            for note in chord:
                midi = note_to_midi(note)
                if midi is None:
                    issues.append(_issue("invalid_plan_note", "Composer plan contains an invalid note.", note=note))
                elif scale_pcs and midi % 12 not in scale_pcs and str(note).upper() not in intentional:
                    off_scale.append(str(note))
        if off_scale:
            issues.append(_issue(
                "unresolved_dissonance",
                "Chord tones outside the chosen scale must be named in intentional_tensions and given a resolution purpose.",
                notes=sorted(set(off_scale)),
            ))

    motif_raw = plan.get("motif_notes")
    motif = [note_to_midi(note) for note in motif_raw] if isinstance(motif_raw, (list, tuple)) else []
    valid_motif = [note for note in motif if note is not None]
    if not 4 <= len(valid_motif) <= 16 or len(valid_motif) != len(motif):
        issues.append(_issue("weak_motif", "Declare a memorable motif of 4-16 valid notes."))
    elif len(valid_motif) >= 2:
        intervals_melodic = [abs(b - a) for a, b in zip(valid_motif, valid_motif[1:])]
        step_ratio = sum(value <= 4 for value in intervals_melodic) / len(intervals_melodic)
        max_leap = max(intervals_melodic)
        metrics.update({"motif_step_ratio": round(step_ratio, 3), "motif_max_leap": max_leap})
        if step_ratio < profile["minimum_step_ratio"]:
            issues.append(_issue(
                "jagged_melody",
                "The motif leaps too often for this profile; anchor it with more stepwise or repeated motion.",
                step_ratio=step_ratio,
            ))
        if max_leap > 12:
            issues.append(_issue("extreme_melodic_leap", "Motif leaps over an octave without an explicit recovery."))
        if scale_pcs:
            in_scale = sum(note % 12 in scale_pcs for note in valid_motif) / len(valid_motif)
            metrics["motif_in_scale_ratio"] = round(in_scale, 3)
            if in_scale < 0.8:
                issues.append(_issue("melody_key_conflict", "At least 80% of motif notes must reinforce the declared scale."))

    phrase_bars = plan.get("phrase_bars")
    if phrase_bars not in (4, 8):
        issues.append(_issue("unclear_phrase_length", "Use a 4- or 8-bar phrase grid, then bend it deliberately at transitions."))
    transformations = plan.get("transformations")
    if not isinstance(transformations, (list, tuple)) or len(set(map(str, transformations))) < 3:
        issues.append(_issue("insufficient_motif_development", "Name at least three distinct motif transformations."))

    try:
        swing = float(plan.get("swing", 0.0))
    except (TypeError, ValueError):
        swing = -1.0
    swing_low, swing_high = profile["swing"]
    if not swing_low <= swing <= swing_high:
        issues.append(_issue(
            "style_groove_mismatch",
            f"{profile_name or 'selected'} profile expects swing in {swing_low:.2f}-{swing_high:.2f}.",
            swing=swing,
        ))

    registers = plan.get("registers")
    parsed_registers = {
        name: _register_range(registers.get(name)) if isinstance(registers, dict) else None
        for name in ("bass", "harmony", "lead")
    }
    if any(value is None for value in parsed_registers.values()):
        issues.append(_issue("missing_register_plan", "Declare octave ranges for bass, harmony, and lead."))
    else:
        bass_range = parsed_registers["bass"]
        harmony_range = parsed_registers["harmony"]
        lead_range = parsed_registers["lead"]
        assert bass_range and harmony_range and lead_range
        if bass_range[1] > harmony_range[0] or harmony_range[1] > lead_range[0]:
            advisories.append(_issue("register_masking_risk", "Separate bass, harmony, and lead ranges more clearly to reduce mud."))

    density = plan.get("section_density")
    if not isinstance(density, (list, tuple)) or len(density) != section_count or section_count < 4:
        issues.append(_issue("missing_density_arc", "section_density must map every exposed section."))
    else:
        try:
            density_values = [float(value) for value in density]
        except (TypeError, ValueError):
            density_values = []
        if not density_values or any(not 0.0 <= value <= 1.0 for value in density_values):
            issues.append(_issue("invalid_density_arc", "Section density values must be within 0-1."))
        elif max(density_values) - min(density_values) < 0.25:
            issues.append(_issue("mush_density", "The plan stays too uniformly dense; create breath, focus, and a real peak."))

    tension_policy = str(plan.get("tension_policy", "")).strip()
    if len(tension_policy) < 24:
        issues.append(_issue("missing_tension_policy", "State which dissonances are intentional and how they resolve."))

    return {"ok": not issues, "issues": issues, "advisories": advisories, "metrics": metrics}


def evaluate_composer_events(
    events: Any,
    plan: Any,
    *,
    bpm: Any,
    bars: Any,
    beats_per_bar: Any = 4,
    section_names: Any = None,
    layer_names: Any = None,
    audio_duration: float = 0.0,
) -> dict[str, Any]:
    """Validate a factual beat-timed event trace and compare it with the plan.

    ``TELEDRA_EVENTS`` is a list of dictionaries. Every event declares
    ``kind`` (note/drum/fx), ``track``, ``role``, ``start_beat``,
    ``duration_beats``, ``velocity``, and ``section``. Note events also declare
    ``pitch`` as a Teledra note name or MIDI integer. ``motif`` and ``transform``
    are optional trace labels; motif presence is still checked from pitches.
    """

    issues: list[dict[str, Any]] = []
    advisories: list[dict[str, Any]] = []
    metrics: dict[str, Any] = {}
    if not isinstance(events, list) or not events:
        return {
            "ok": False,
            "issues": [_issue(
                "missing_composer_events",
                "Composer-grade music must expose a non-empty TELEDRA_EVENTS performance trace.",
            )],
            "advisories": [],
            "metrics": {},
            "normalized_events": [],
        }

    bpm_value = _finite_number(bpm)
    bars_value = _finite_number(bars)
    beats_value = _finite_number(beats_per_bar)
    if bpm_value is None or bpm_value <= 0:
        issues.append(_issue("invalid_event_tempo", "BPM must be positive to validate event timing."))
    if bars_value is None or bars_value <= 0:
        issues.append(_issue("invalid_event_bars", "BARS must be positive to validate event timing."))
    if beats_value is None or not 1 <= beats_value <= 12:
        issues.append(_issue("invalid_event_meter", "BEATS_PER_BAR must be between 1 and 12."))
    if bpm_value is None or bpm_value <= 0 or bars_value is None or bars_value <= 0 or beats_value is None:
        return {
            "ok": False,
            "issues": issues,
            "advisories": advisories,
            "metrics": metrics,
            "normalized_events": [],
        }

    names: list[str] = []
    if isinstance(section_names, (list, tuple)):
        for entry in section_names:
            name = entry.get("name") if isinstance(entry, dict) else entry
            if isinstance(name, str) and name.strip():
                names.append(name.strip())
    if len(names) < 4:
        issues.append(_issue(
            "invalid_event_sections",
            "Event validation needs at least four named TELEDRA_SCORE sections.",
        ))
    total_beats = float(bars_value * beats_value)
    if audio_duration > 0:
        expected_duration = total_beats * 60.0 / bpm_value
        tolerance = max(0.25, (60.0 / bpm_value) * 0.5)
        metrics["declared_duration_seconds"] = round(expected_duration, 4)
        if abs(expected_duration - audio_duration) > tolerance:
            issues.append(_issue(
                "score_duration_mismatch",
                "BARS, meter, and BPM do not describe the rendered duration.",
                declared=round(expected_duration, 4),
                rendered=round(audio_duration, 4),
                tolerance=round(tolerance, 4),
            ))

    known_layers = {str(value) for value in layer_names} if isinstance(layer_names, (list, tuple, set)) else set()
    schema_errors: list[dict[str, Any]] = []
    timing_errors: list[int] = []
    section_errors: list[dict[str, Any]] = []
    unknown_tracks: set[str] = set()
    normalized: list[dict[str, Any]] = []
    section_length = total_beats / len(names) if names else total_beats

    for index, event in enumerate(events):
        if not isinstance(event, dict):
            schema_errors.append({"index": index, "fields": ["event_not_object"]})
            continue
        kind = str(event.get("kind", "")).strip().lower()
        track = str(event.get("track", "")).strip()
        raw_role = str(event.get("role", "")).strip()
        role = _canonical_role(raw_role)
        start = _finite_number(event.get("start_beat"))
        duration = _finite_number(event.get("duration_beats"))
        velocity = _finite_number(event.get("velocity"))
        section = str(event.get("section", "")).strip()
        pitch = note_to_midi(event.get("pitch")) if kind == "note" else None
        bad_fields: list[str] = []
        if kind not in EVENT_KINDS:
            bad_fields.append("kind")
        if not track:
            bad_fields.append("track")
        if not role:
            bad_fields.append("role")
        if start is None or start < 0:
            bad_fields.append("start_beat")
        if duration is None or duration <= 0:
            bad_fields.append("duration_beats")
        if velocity is None or not 0 < velocity <= 1:
            bad_fields.append("velocity")
        if kind == "note" and pitch is None:
            bad_fields.append("pitch")
        if names and section not in names:
            bad_fields.append("section")
        if bad_fields:
            if len(schema_errors) < 20:
                schema_errors.append({"index": index, "fields": bad_fields})
            continue
        assert start is not None and duration is not None and velocity is not None
        if start + duration > total_beats + 1e-6:
            if len(timing_errors) < 20:
                timing_errors.append(index)
            continue
        if known_layers and track not in known_layers:
            unknown_tracks.add(track)
        expected_section = section
        if names:
            section_index = min(int(start / max(section_length, 1e-9)), len(names) - 1)
            expected_section = names[section_index]
            if section != expected_section and len(section_errors) < 20:
                section_errors.append({"index": index, "claimed": section, "actual": expected_section})
        normalized.append({
            "index": index,
            "kind": kind,
            "track": track,
            "role": role,
            "pitch": pitch,
            "start_beat": start,
            "duration_beats": duration,
            "velocity": velocity,
            "section": expected_section,
            "motif": str(event.get("motif", "")).strip(),
            "transform": _normalise_token(event.get("transform")),
        })

    if schema_errors:
        issues.append(_issue(
            "invalid_event_schema",
            "TELEDRA_EVENTS entries are missing or contain invalid required fields.",
            events=schema_errors,
        ))
    if timing_errors:
        issues.append(_issue(
            "event_out_of_bounds",
            "One or more events extend beyond the declared score length.",
            indices=timing_errors,
            total_beats=total_beats,
        ))
    if section_errors:
        issues.append(_issue(
            "event_section_mismatch",
            "Event section labels disagree with their beat positions.",
            events=section_errors,
        ))
    if unknown_tracks:
        issues.append(_issue(
            "unknown_event_track",
            "Every event track must name a real TELEDRA_LAYERS buffer.",
            tracks=sorted(unknown_tracks),
        ))
    minimum_events = max(12, len(names) * 4)
    if len(normalized) < minimum_events:
        issues.append(_issue(
            "thin_event_trace",
            "The performance trace is too small to demonstrate a developed arrangement.",
            events=len(normalized),
            minimum=minimum_events,
        ))
    if not normalized:
        return {
            "ok": False,
            "issues": issues,
            "advisories": advisories,
            "metrics": metrics,
            "normalized_events": [],
        }

    track_roles: dict[str, set[str]] = {}
    for event in normalized:
        track_roles.setdefault(event["track"], set()).add(event["role"])
    conflicts = {track: sorted(roles) for track, roles in track_roles.items() if len(roles) > 1}
    if conflicts:
        issues.append(_issue(
            "track_role_conflict",
            "A track must keep one stable musical role throughout the trace.",
            tracks=conflicts,
        ))

    plan_dict = plan if isinstance(plan, dict) else {}
    profile_name = str(plan_dict.get("style_profile", "court_experimental")).strip().lower()
    root_pc = PITCH_CLASSES.get(str(plan_dict.get("tonal_center", "")).strip().upper())
    mode = str(plan_dict.get("mode", "")).strip().lower().replace(" ", "_")
    intervals = MODE_INTERVALS.get(mode)
    scale_pcs = set() if root_pc is None or intervals is None else {(root_pc + value) % 12 for value in intervals}
    pitched = [event for event in normalized if event["kind"] == "note" and event["pitch"] is not None]
    role_set = {event["role"] for event in normalized}
    metrics.update({
        "event_count": len(normalized),
        "pitched_event_count": len(pitched),
        "active_roles": sorted(role_set),
    })
    if len(role_set) < 4:
        issues.append(_issue(
            "thin_role_trace",
            "Use at least four performed roles so foundation, body, focus, and motion/air are factual.",
            roles=sorted(role_set),
        ))
    missing_pitched_roles = [role for role in ("bass", "harmony", "lead") if not any(event["role"] == role for event in pitched)]
    if missing_pitched_roles:
        issues.append(_issue(
            "missing_pitched_roles",
            "TELEDRA_EVENTS must perform bass, harmony, and lead roles.",
            roles=missing_pitched_roles,
        ))

    if pitched and scale_pcs:
        total_weight = sum(event["duration_beats"] * event["velocity"] for event in pitched)
        scale_weight = sum(
            event["duration_beats"] * event["velocity"]
            for event in pitched
            if event["pitch"] % 12 in scale_pcs
        )
        scale_fit = scale_weight / max(total_weight, 1e-12)
        metrics["performed_scale_fit"] = round(scale_fit, 4)
        minimum_fit = {"retro_adventure": 0.90, "spicy_lofi": 0.75}.get(profile_name, 0.55)
        if scale_fit < minimum_fit:
            issues.append(_issue(
                "performed_key_conflict",
                "The performed pitched events do not sufficiently support the declared scale.",
                scale_fit=round(scale_fit, 4),
                minimum=minimum_fit,
            ))

    motif_notes = plan_dict.get("motif_notes")
    motif = [note_to_midi(note) for note in motif_notes] if isinstance(motif_notes, (list, tuple)) else []
    motif = [note for note in motif if note is not None]
    focus_pitches = [
        event["pitch"]
        for event in sorted(pitched, key=lambda item: (item["start_beat"], item["index"]))
        if event["role"] in {"lead", "motion"}
    ]
    motif_occurrences = 0
    if len(motif) >= 4 and len(focus_pitches) >= len(motif):
        target_intervals = [b - a for a, b in zip(motif, motif[1:])]
        for start in range(len(focus_pitches) - len(motif) + 1):
            window = focus_pitches[start:start + len(motif)]
            intervals_actual = [b - a for a, b in zip(window, window[1:])]
            if intervals_actual == target_intervals:
                motif_occurrences += 1
    metrics["motif_trace_occurrences"] = motif_occurrences
    if motif_occurrences < 1:
        issues.append(_issue(
            "unperformed_motif",
            "The declared motif contour never appears in the performed lead/motion events.",
        ))

    claimed_transforms = {
        _normalise_token(value)
        for value in plan_dict.get("transformations", [])
        if _normalise_token(value)
    }
    performed_transforms = {
        event["transform"]
        for event in normalized
        if event["transform"] and event["transform"] not in {"prime", "statement"}
    }
    matched_transforms = claimed_transforms & performed_transforms
    metrics["performed_transformations"] = sorted(performed_transforms)
    if len(performed_transforms) < 2 or not matched_transforms:
        issues.append(_issue(
            "unperformed_transformations",
            "Trace at least two motif transformations and perform one named in the composition plan.",
            claimed=sorted(claimed_transforms),
            performed=sorted(performed_transforms),
        ))

    harmony_groups: dict[float, set[int]] = {}
    for event in pitched:
        if event["role"] == "harmony":
            harmony_groups.setdefault(round(event["start_beat"], 4), set()).add(event["pitch"] % 12)
    planned_chords: list[set[int]] = []
    chord_voicings = plan_dict.get("chord_voicings")
    if isinstance(chord_voicings, (list, tuple)):
        for chord in chord_voicings:
            if isinstance(chord, (list, tuple)):
                notes = [note_to_midi(note) for note in chord]
                if notes and all(note is not None for note in notes):
                    planned_chords.append({int(note) % 12 for note in notes if note is not None})
    if planned_chords:
        performed_chords = sum(
            any(chord.issubset(group) for group in harmony_groups.values())
            for chord in planned_chords
        )
        chord_ratio = performed_chords / len(planned_chords)
        metrics["performed_chord_ratio"] = round(chord_ratio, 4)
        minimum_chord_ratio = 0.50 if profile_name == "court_experimental" else 0.75
        if chord_ratio < minimum_chord_ratio:
            issues.append(_issue(
                "unperformed_harmony_plan",
                "Concrete harmony events do not perform enough of the declared chord voicings.",
                ratio=round(chord_ratio, 4),
                minimum=minimum_chord_ratio,
            ))

    registers = plan_dict.get("registers")
    role_midis = {
        role: [event["pitch"] for event in pitched if event["role"] == role]
        for role in ("bass", "harmony", "lead")
    }
    actual_medians = {role: _median(values) for role, values in role_midis.items() if values}
    metrics["role_median_midi"] = {role: round(value, 3) for role, value in actual_medians.items()}
    if all(role in actual_medians for role in ("bass", "harmony", "lead")):
        bass_harmony = actual_medians["harmony"] - actual_medians["bass"]
        harmony_lead = actual_medians["lead"] - actual_medians["harmony"]
        metrics["register_separation_semitones"] = {
            "bass_to_harmony": round(bass_harmony, 3),
            "harmony_to_lead": round(harmony_lead, 3),
        }
        if bass_harmony < 5 or harmony_lead < 3:
            issues.append(_issue(
                "performed_register_collision",
                "Performed bass, harmony, and lead occupy insufficiently separated centers.",
                bass_to_harmony=round(bass_harmony, 3),
                harmony_to_lead=round(harmony_lead, 3),
            ))
    if isinstance(registers, dict):
        outside: list[dict[str, Any]] = []
        checked = 0
        for role in ("bass", "harmony", "lead"):
            planned_range = _register_range(registers.get(role))
            if planned_range is None:
                continue
            for event in pitched:
                if event["role"] != role:
                    continue
                checked += 1
                octave = event["pitch"] // 12 - 1
                if not planned_range[0] <= octave <= planned_range[1] and len(outside) < 20:
                    outside.append({"index": event["index"], "role": role, "octave": octave})
        mismatch_ratio = len(outside) / max(checked, 1)
        metrics["register_plan_mismatch_ratio"] = round(mismatch_ratio, 4)
        if mismatch_ratio > 0.10:
            issues.append(_issue(
                "event_register_mismatch",
                "Too many pitched events fall outside their declared role registers.",
                ratio=round(mismatch_ratio, 4),
                events=outside,
            ))

    section_counts = [0 for _ in names]
    if names:
        section_lookup = {name: index for index, name in enumerate(names)}
        for event in normalized:
            section_counts[section_lookup[event["section"]]] += 1
        maximum_count = max(section_counts) if section_counts else 0
        normalized_density = [count / max(maximum_count, 1) for count in section_counts]
        metrics["section_event_counts"] = section_counts
        metrics["performed_section_density"] = [round(value, 4) for value in normalized_density]
        if maximum_count and (maximum_count - min(section_counts)) / maximum_count < 0.20:
            issues.append(_issue(
                "flat_event_density",
                "Performed event counts stay too uniform across sections.",
                counts=section_counts,
            ))
        planned_density = plan_dict.get("section_density")
        if isinstance(planned_density, (list, tuple)) and len(planned_density) == len(names):
            try:
                planned_values = [float(value) for value in planned_density]
            except (TypeError, ValueError):
                planned_values = []
            if planned_values:
                correlation = _correlation(planned_values, normalized_density)
                metrics["section_density_correlation"] = round(correlation, 4)
                if correlation < 0.35:
                    issues.append(_issue(
                        "section_density_mismatch",
                        "Performed event counts do not follow the planned section-density arc.",
                        correlation=round(correlation, 4),
                        planned=planned_values,
                        performed=[round(value, 4) for value in normalized_density],
                    ))

    # An eighth-beat grid preserves short intentional gaps in chip/lo-fi
    # articulations that a quarter-beat occupancy grid would round away.
    grid_step = 0.125
    slot_count = max(1, int(math.ceil(total_beats / grid_step)))
    role_slots = {role: bytearray(slot_count) for role in role_set}
    for event in normalized:
        start_slot = max(0, int(math.floor(event["start_beat"] / grid_step)))
        end_slot = min(slot_count, max(start_slot + 1, int(math.ceil((event["start_beat"] + event["duration_beats"]) / grid_step))))
        role_slots[event["role"]][start_slot:end_slot] = b"\x01" * (end_slot - start_slot)
    active_counts = [sum(mask[index] for mask in role_slots.values()) for index in range(slot_count)]
    role_count = max(1, len(role_slots))
    low_density_threshold = max(1, int(math.floor(role_count * 0.40)))
    breathing_ratio = sum(value <= low_density_threshold for value in active_counts) / slot_count
    full_threshold = max(3, int(math.ceil(role_count * 0.75)))
    full_density_ratio = sum(value >= full_threshold for value in active_counts) / slot_count
    focus_masks = [role_slots[role] for role in ("lead", "motion") if role in role_slots]
    focus_rest_ratio = (
        sum(not any(mask[index] for mask in focus_masks) for index in range(slot_count)) / slot_count
        if focus_masks else 0.0
    )
    metrics.update({
        "mean_active_roles": round(sum(active_counts) / slot_count, 4),
        "breathing_ratio": round(breathing_ratio, 4),
        "focus_rest_ratio": round(focus_rest_ratio, 4),
        "full_density_ratio": round(full_density_ratio, 4),
    })
    if breathing_ratio < 0.04 or focus_rest_ratio < 0.08:
        issues.append(_issue(
            "no_rhythmic_breathing",
            "Performed roles leave too little rhythmic breathing or foreground rest.",
            breathing_ratio=round(breathing_ratio, 4),
            focus_rest_ratio=round(focus_rest_ratio, 4),
        ))
    if full_density_ratio > 0.70:
        issues.append(_issue(
            "overfull_event_density",
            "Nearly every role remains active for too much of the piece.",
            full_density_ratio=round(full_density_ratio, 4),
        ))

    return {
        "ok": not issues,
        "issues": issues,
        "advisories": advisories,
        "metrics": metrics,
        "normalized_events": normalized,
    }


def event_audio_alignment(
    normalized_events: list[dict[str, Any]],
    layers: dict[str, Any],
    *,
    bpm: Any,
    sample_rate: int,
) -> dict[str, Any]:
    """Check that rhythmic event windows are audibly reflected in their stems."""

    import numpy as np

    issues: list[dict[str, Any]] = []
    metrics: dict[str, Any] = {}
    bpm_value = _finite_number(bpm)
    if bpm_value is None or bpm_value <= 0 or sample_rate <= 0:
        return {"issues": [], "advisories": [], "metrics": {}}
    by_track: dict[str, list[dict[str, Any]]] = {}
    for event in normalized_events:
        if event["role"] not in {"harmony", "texture"}:
            by_track.setdefault(event["track"], []).append(event)
    contrasts: dict[str, float] = {}
    evaluated = 0
    mismatched: list[str] = []
    frame_size = max(16, int(sample_rate * 0.025))
    seconds_per_beat = 60.0 / bpm_value
    for track, track_events in by_track.items():
        if track not in layers:
            continue
        audio = np.asarray(layers[track], dtype=float)
        mono = audio.mean(axis=1) if audio.ndim == 2 else audio.reshape(-1)
        frame_count = len(mono) // frame_size
        if frame_count < 8:
            continue
        frames = mono[:frame_count * frame_size].reshape(frame_count, frame_size)
        envelope = np.sqrt(np.mean(np.square(frames), axis=1))
        mask = np.zeros(frame_count, dtype=bool)
        for event in track_events:
            start_seconds = event["start_beat"] * seconds_per_beat
            end_seconds = (event["start_beat"] + event["duration_beats"]) * seconds_per_beat
            start_frame = max(0, int(math.floor(start_seconds * sample_rate / frame_size)))
            end_frame = min(frame_count, max(start_frame + 1, int(math.ceil(end_seconds * sample_rate / frame_size))))
            mask[start_frame:end_frame] = True
        coverage = float(np.mean(mask))
        if coverage < 0.08 or coverage > 0.88 or np.count_nonzero(~mask) < 5:
            continue
        inside = float(np.sqrt(np.mean(np.square(envelope[mask]))))
        outside = float(np.sqrt(np.mean(np.square(envelope[~mask]))))
        contrast = inside / max(outside, 1e-12)
        contrasts[track] = round(contrast, 4)
        evaluated += 1
        if contrast < 1.15:
            mismatched.append(track)
    metrics["event_audio_contrast"] = contrasts
    metrics["event_audio_tracks_checked"] = evaluated
    if mismatched:
        issues.append(_issue(
            "event_audio_mismatch",
            "Declared rhythmic rests are not reflected in the corresponding stem audio.",
            tracks=sorted(mismatched),
            minimum_contrast=1.15,
        ))
    if evaluated < 2:
        issues.append(_issue(
            "unverifiable_event_audio",
            "At least two rhythmic stems must contain measurable event/rest contrast.",
            tracks_checked=evaluated,
        ))
    return {"issues": issues, "advisories": [], "metrics": metrics}


def audio_quality_metrics(wave: Any, sample_rate: int) -> dict[str, float]:
    """Cheap mix diagnostics using bounded original-rate spectral windows."""

    import numpy as np

    audio = np.asarray(wave, dtype=float)
    mono = audio.mean(axis=1) if audio.ndim == 2 else audio.reshape(-1)
    if mono.size == 0 or not np.all(np.isfinite(mono)):
        return {}
    peak = float(np.max(np.abs(audio)))
    rms = float(np.sqrt(np.mean(np.square(audio))))
    if audio.ndim == 2:
        dc = float(np.max(np.abs(np.mean(audio, axis=0))))
    else:
        dc = float(abs(np.mean(mono)))
    crest = peak / max(rms, 1e-12)
    # Use a bounded number of contiguous windows at the original sample rate.
    # Raw stride decimation aliases high frequencies and can lower the effective
    # Nyquist below the very band being measured on long compositions.
    frame_size = min(4096, mono.size)
    if frame_size < 32 or sample_rate <= 0:
        high_ratio = 0.0
        lowmid_ratio = 0.0
    else:
        possible_frames = max(1, mono.size // frame_size)
        frame_count = min(24, possible_frames)
        last_start = max(0, mono.size - frame_size)
        starts = np.linspace(0, last_start, frame_count, dtype=int)
        window = np.hanning(frame_size)
        power = np.zeros(frame_size // 2 + 1, dtype=float)
        for start in starts:
            frame = mono[start:start + frame_size]
            frame = frame - np.mean(frame)
            power += np.abs(np.fft.rfft(frame * window)) ** 2
        freqs = np.fft.rfftfreq(frame_size, d=1.0 / float(sample_rate))
        total_power = max(float(power.sum()), 1e-12)
        high_ratio = float(power[freqs >= 8000].sum() / total_power) if sample_rate / 2 > 8000 else 0.0
        lowmid = (freqs >= 150) & (freqs < 500)
        lowmid_ratio = float(power[lowmid].sum() / total_power)
    peak_dbfs = 20.0 * math.log10(max(peak, 1e-12))
    rms_dbfs = 20.0 * math.log10(max(rms, 1e-12))
    return {
        "dc_offset": dc,
        "crest_factor": crest,
        "high_frequency_ratio": high_ratio,
        "lowmid_frequency_ratio": lowmid_ratio,
        "mix_peak": peak,
        "mix_rms": rms,
        "peak_dbfs": peak_dbfs,
        "rms_dbfs": rms_dbfs,
    }
