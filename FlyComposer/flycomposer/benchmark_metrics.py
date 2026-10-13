"""Independent recording facts, without a reward, taste grade or winning author."""
from __future__ import annotations

from collections import Counter, defaultdict
import hashlib
import json
import math

SR = 48_000
DRUM_VOICES = frozenset(("chip:noise", "amiga:drums"))


def _finite(value):
    return type(value) in (int, float) and math.isfinite(value)


def _digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def _sweep(rows, end_frame):
    edges = defaultdict(list)
    for event in rows:
        start, end = max(0, event["start_frame"]), min(end_frame, event["end_frame"])
        if start < end:
            edges[start].append((event["layer"], 1))
            edges[end].append((event["layer"], -1))
    edges[0]
    edges[end_frame]
    active = Counter()
    previous = 0
    occupied = part_frames = note_frames = two = three = 0
    part_peak = note_peak = 0
    gaps = []
    for moment in sorted(edges):
        width = moment - previous
        parts = sum(count > 0 for count in active.values())
        notes = sum(active.values())
        if width:
            if parts:
                occupied += width
            elif gaps and gaps[-1][1] == previous:
                gaps[-1][1] = moment
            else:
                gaps.append([previous, moment])
            part_frames += parts * width
            note_frames += notes * width
            two += (parts >= 2) * width
            three += (parts >= 3) * width
            part_peak, note_peak = max(part_peak, parts), max(note_peak, notes)
        # Apply all ends and starts at this frame together: touching notes do not overlap.
        for part, delta in edges[moment]:
            active[part] += delta
        previous = moment
    return {"occupied_fraction": occupied / end_frame,
            "longest_gap_seconds": max((b - a for a, b in gaps), default=0) / SR,
            "leading_gap_seconds": gaps[0][1] / SR if gaps and gaps[0][0] == 0 else 0.,
            "trailing_gap_seconds": (end_frame - gaps[-1][0]) / SR if gaps and gaps[-1][1] == end_frame else 0.,
            "silent_intervals_seconds": [[a / SR, b / SR] for a, b in gaps],
            "parts": {"mean": part_frames / end_frame, "fraction_at_least_2": two / end_frame,
                      "fraction_at_least_3": three / end_frame, "peak": part_peak},
            "notes": {"mean": note_frames / end_frame, "peak": note_peak}}


def _variety(values):
    counts = Counter(round(float(value), 8) for value in values)
    total = sum(counts.values())
    return {"distinct": len(counts), "minimum": min(counts) if counts else None,
            "maximum": max(counts) if counts else None,
            "most_common_share": max(counts.values()) / total if total else None}


def _foreground(rows, bpm, target_frames):
    """Two-bar note-onset fingerprints; ties/chords break the step-run analysis."""
    phrase_frames = SR * 8 * 60 / bpm
    complete_windows = int(math.floor(target_frames / phrase_frames + 1e-10))
    windows = [[] for _ in range(complete_windows)]
    onset_groups = defaultdict(list)
    for event in rows:
        onset = event["start_frame"]
        if not 0 <= onset < target_frames:
            continue
        onset_groups[onset].append(event)
        phrase_index = int(math.floor(onset / phrase_frames))
        if phrase_index < complete_windows:
            origin = int(round(phrase_index * phrase_frames))
            windows[phrase_index].append((onset - origin, event["frames"], event["midi"], event["velocity"]))
    fingerprints = [_digest(sorted(window)) if window else None for window in windows]
    nonempty = [value for value in fingerprints if value is not None]
    adjacent = [(a, b) for a, b in zip(fingerprints, fingerprints[1:]) if a is not None and b is not None]
    previous = None
    direction = length = longest = 0
    for onset, notes in sorted(onset_groups.items()):
        if len(notes) != 1:
            previous, direction, length = None, 0, 0
            continue
        current = notes[0]
        step = current["midi"] - previous["midi"] if previous is not None else 0
        # A rest longer than a quarter note or simultaneous voices breaks a run.
        connects = previous is not None and onset >= previous["end_frame"] and onset - previous["end_frame"] <= SR * 60 / bpm
        if connects and abs(step) in (1, 2):
            sign = 1 if step > 0 else -1
            length = length + 1 if direction == sign else 2
            direction = sign
        else:
            direction, length = 0, 1
        longest = max(longest, length)
        previous = current
    return {"phrase_beats": 8, "complete_two_bar_windows": complete_windows,
            "nonempty_two_bar_windows": len(nonempty), "exact_fingerprints": fingerprints,
            "exact_repeated_phrase_share": (len(nonempty) - len(set(nonempty))) / len(nonempty) if nonempty else None,
            "adjacent_exact_repeat_share": sum(a == b for a, b in adjacent) / len(adjacent) if adjacent else None,
            "longest_monotonic_step_run_notes": longest,
            "definition": "Exact onset, duration, MIDI and velocity within complete two-bar windows; empty windows excluded from repeat share. Step runs use one note per onset, non-overlapping notes, same-direction 1–2 semitone steps and at most one beat of intervening rest. This does not infer a key or melodic voices through chords."}


def measure(score, audit, target_seconds=40):
    """Measure audited 48 kHz placements over [0, target_seconds), including silence.

    Preserves both inputs. Coverage uses mixed ledger rows, not floating note ends.
    Delivery flags check the renderer's evidence; they do not independently inspect
    WAV bytes or prove human audibility. No composition or preference is inferred.
    """
    if not _finite(target_seconds) or target_seconds <= 0 or int(target_seconds * SR) < 1:
        raise ValueError("target_seconds must define a positive finite frame window")
    if audit.get("sample_rate") != SR:
        raise ValueError("Benchmark audit must use 48000 Hz")
    bpm = score.get("bpm")
    if not _finite(bpm) or bpm <= 0:
        raise ValueError("Score BPM must be positive and finite")
    target_frames = int(target_seconds * SR)
    scheduled = score.get("events", [])
    ledger = audit.get("events", [])
    if not isinstance(scheduled, list) or not isinstance(ledger, list):
        raise ValueError("Score and audit events must be lists")
    valid = []
    for event in ledger:
        if (type(event.get("start_frame")) is int and event["start_frame"] >= 0
                and type(event.get("frames")) is int and event["frames"] > 0
                and type(event.get("end_frame")) is int
                and event["end_frame"] == event["start_frame"] + event["frames"]
                and isinstance(event.get("layer"), str) and type(event.get("midi")) is int
                and _finite(event.get("velocity"))):
            valid.append(event)
    rows = [event for event in valid if event.get("mixed") is True]
    coverage = _sweep(rows, target_frames)
    pitched = [event for event in rows if event.get("voice") not in DRUM_VOICES]
    layering = _sweep(pitched, target_frames)
    score_ids = [event.get("id") for event in scheduled]
    audit_ids = [event.get("id") for event in ledger]
    unique_ids = (len(set(score_ids)) == len(score_ids) and len(set(audit_ids)) == len(audit_ids)
                  and None not in score_ids and None not in audit_ids)
    originals = {event.get("id"): event for event in scheduled} if unique_ids else {}
    exact_notes = unique_ids and set(score_ids) == set(audit_ids) and all(
        all(event.get(key) == value for key, value in originals[event["id"]].items()) for event in ledger)
    event_hash = _digest(scheduled)
    expected_placements = len(valid) == len(ledger) and all(
        _finite(event.get("start_seconds")) and _finite(event.get("duration_seconds"))
        and event["start_frame"] == int(event["start_seconds"] * SR)
        and event["frames"] == int(event["duration_seconds"] * SR) for event in valid)
    pcm_evidence = bool(rows) and len(rows) == len(ledger) and all(
        _finite(event.get("placed_rms")) and event["placed_rms"] > 0
        and isinstance(event.get("note_pcm_sha256"), str) and len(event["note_pcm_sha256"]) == 64
        and all(char in "0123456789abcdef" for char in event["note_pcm_sha256"]) for event in rows)
    declared_counts = audit.get("notes_scheduled") == len(scheduled) and audit.get("notes_mixed") == len(rows)
    ledger_hash_matches = score.get("event_sha256") == event_hash == audit.get("event_sha256")
    in_recording = type(audit.get("frames")) is int and all(event["end_frame"] <= audit["frames"] for event in valid)
    delivery = {"notes_scheduled": len(scheduled), "notes_mixed": len(rows),
                "declared_counts_match": declared_counts, "exact_authored_notes": bool(exact_notes),
                "event_hash_matches": ledger_hash_matches, "valid_frame_rows": len(valid),
                "exact_frame_placements": expected_placements, "all_notes_within_recording": in_recording,
                "nonzero_note_pcm_evidence": pcm_evidence, "renderer_claims_complete": audit.get("complete") is True}
    delivery["complete"] = bool(scheduled) and all((declared_counts, exact_notes, ledger_hash_matches,
        expected_placements, in_recording, pcm_evidence, audit.get("complete") is True,
        len(scheduled) == len(rows) == len(ledger)))
    parts = {}
    for part in sorted(set(score.get("voices", {})) | {event["layer"] for event in rows}):
        notes = [event for event in rows if event["layer"] == part]
        parts[part] = {"notes": len(notes), "notes_onset_in_target": sum(event["start_frame"] < target_frames for event in notes),
                       "register_midi": [min(event["midi"] for event in notes), max(event["midi"] for event in notes)] if notes else None,
                       "length_seconds": _variety(event["frames"] / SR for event in notes),
                       "velocity": _variety(event["velocity"] for event in notes),
                       "occupied_fraction": _sweep(notes, target_frames)["occupied_fraction"]}
    quarters = []
    for quarter in range(4):
        start, end = target_frames * quarter // 4, target_frames * (quarter + 1) // 4
        notes = [event for event in rows if start <= event["start_frame"] < end]
        quarters.append({"start_seconds": start / SR, "end_seconds": end / SR, "notes": len(notes),
                         "notes_per_second": len(notes) * SR / (end - start) if end > start else 0.,
                         "mean_velocity": sum(event["velocity"] for event in notes) / len(notes) if notes else None})
    return {"version": "benchmark_recording_facts_v1", "sample_rate": SR, "target_seconds": target_frames / SR,
            "delivery": delivery, **{key: value for key, value in coverage.items() if key not in ("parts", "notes")},
            "musical_end_seconds": max((event["end_frame"] for event in rows), default=0) / SR,
            "notes_extending_past_target": sum(event["end_frame"] > target_frames for event in rows),
            "notes_starting_after_target": sum(event["start_frame"] >= target_frames for event in rows),
            "pitched_parts": layering["parts"], "pitched_note_polyphony": layering["notes"],
            "excluded_drum_voices": sorted(DRUM_VOICES), "per_part": parts,
            "foreground": {part: _foreground([event for event in rows if event["layer"] == part], bpm, target_frames)
                           for part in ("lead", "answer")}, "quarters": quarters,
            "scope": "Objective score and renderer-ledger evidence. Fixed-window denominators include silence. Pitched parts exclude drum voices, not all texture instruments. No listening, taste grade or winner; WAV verification is separate."}
