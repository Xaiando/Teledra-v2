"""A continuous rate-circuit performance captured as an explicit note ledger.

There is no section plan, chord progression, stored song or template arranger.
One persistent Controller drives the next gesture from its current activity and
the performance's own musical memory. Pitch mapping, memory and voice-leading
constraints are engineered; they are not biological cognition or learned taste.
"""
from __future__ import annotations

import hashlib
import json
import math

import numpy as np

from .composition import ROLES, SCALES, midi, nearest_chord
from .controller import Controller, fingerprint


VOICE_POOLS = {
    "lead": ("electric_piano", "piano", "marimba", "clavinet"),
    "answer": ("theremin", "clarinet", "oboe", "pan_flute"),
    "harmony": ("cello", "viola", "warm_pad", "string_ensemble"),
    "bass": ("classic:bass", "double_bass", "bass", "bassoon"),
    "texture": ("vibraphone", "harp", "celesta", "mandolin"),
}
_REGISTERS = {"lead": (58, 82, 60), "answer": (60, 83, 60), "harmony": (48, 69, 48),
              "bass": (36, 53, 36), "texture": (72, 88, 72)}
_TONICS = ("C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B")


def _select(value, choices):
    index = min(len(choices) - 1, max(0, int((float(value) + 1.) * .5 * len(choices))))
    return choices[index]


def write_performance(circuit, seed=2026, seconds=75, voices=None, condition="connectome"):
    """Capture a free-running musical controller without opening an audio device.

    ``seconds`` includes a one-second silent release margin. With no explicit
    ``voices``, the initial circuit readout chooses a five-part palette from the
    documented compatible pools. All later activity uses the same persistent
    circuit state. A supplied voice map fixes only timbres, not the performance.
    """
    return _write_performance(circuit, seed, seconds, voices, condition)


def extend_performance(circuit, source_score, seconds=120):
    """Replay and continue a saved performance while retaining every source note.

    Each earlier capture limit is replayed, including its shortened final gesture.
    The same Controller, musical memory and part state then resume at the next
    gesture boundary. Capture-only trailing silence is not inserted as a pause.
    This preserves the authored note prefix, not a separately mastered PCM prefix.
    No source object, library entry or audio file is modified.
    """
    if not isinstance(source_score, dict) or source_score.get("kind") != "performance" or source_score.get("performer") != "fly":
        raise ValueError("Continuation needs a saved Fly performance")
    if source_score.get("arrangement_version") != "autonomous_rate_performance_v1":
        raise ValueError("Source performance uses an unsupported Fly engine")
    source_seconds = source_score.get("duration_seconds")
    if type(source_seconds) not in (int, float) or not math.isfinite(source_seconds) or not 10 <= source_seconds <= 180:
        raise ValueError("Source performance has an invalid capture duration")
    if type(seconds) not in (int, float) or not math.isfinite(seconds) or not source_seconds + 5 <= seconds <= 180:
        raise ValueError("Continuation must add at least five seconds, up to 180 seconds total")
    previous_limits = []
    if "continuation" in source_score:
        context = source_score["continuation"]
        limits = context.get("capture_limits_seconds") if isinstance(context, dict) else None
        if not isinstance(limits, list) or len(limits) < 2:
            raise ValueError("Source continuation has no reproducible capture history")
        if any(type(value) not in (int, float) or not math.isfinite(value) or not 10 <= value <= 180 for value in limits):
            raise ValueError("Source continuation has invalid capture limits")
        if limits[-1] != source_seconds or any(b - a < 5 for a, b in zip(limits, limits[1:])):
            raise ValueError("Source continuation capture history is inconsistent")
        previous_limits = limits[:-1]
    provenance = source_score.get("decoder_provenance")
    if not isinstance(provenance, dict):
        raise ValueError("Source performance has no decoder provenance")
    selection = provenance.get("voice_selection")
    if selection == "explicit override":
        voices = source_score.get("voices")
    elif selection == "initial neural readout selects from role-compatible pools":
        voices = None
    else:
        raise ValueError("Source performance has unknown instrument-selection provenance")
    try:
        replay = _write_performance(circuit, source_score["seed"], source_seconds, voices,
                                    source_score["condition"], previous_limits)
    except (KeyError, TypeError) as error:
        raise ValueError("Source performance lacks reproducible authoring parameters") from error
    # Compare the complete authored result and its state/memory provenance. Titles
    # and external library annotations may change without changing the performance.
    for key, expected in replay.items():
        if key == "title":
            continue
        if source_score.get(key) != expected:
            raise ValueError(f"Source performance cannot be reproduced: {key} differs")
    continued = _write_performance(circuit, source_score["seed"], seconds, voices,
                                   source_score["condition"], [*previous_limits, source_seconds])
    prefix = continued["events"][:len(source_score["events"])]
    if prefix != source_score["events"]:
        raise ValueError("Continuation failed to preserve the exact source note prefix")
    if len(continued["events"]) == len(prefix):
        raise ValueError("Continuation produced no new notes")
    return continued


def _write_performance(circuit, seed, seconds, voices, condition, previous_capture_limits=()):
    if type(seed) is not int or not 0 <= seed <= 2**31 - 1:
        raise ValueError("Seed must be an integer in 0..2147483647")
    if isinstance(seconds, bool) or not isinstance(seconds, (int, float)) or not math.isfinite(seconds) or not 10 <= seconds <= 180:
        raise ValueError("Capture duration must be finite and between 10 and 180 seconds")
    if voices is not None and (not isinstance(voices, dict) or set(voices) != set(ROLES)
                               or any(not isinstance(v, str) or not v for v in voices.values())):
        raise ValueError("Provide five named instrumental voices")
    seconds = float(seconds)
    controller = Controller(circuit, seed, condition)
    feedback_energy, harmonic_root = .45, 0
    first = np.asarray(controller.phrase(0, feedback_energy, harmonic_root), dtype=float)
    initial = np.tanh(2. * first)
    bpm = int(round(96 + 18 * float(initial[7])))
    beat_seconds = 60. / bpm
    tonic = int(_select(initial[0], (0, 2, 5, 7, 9)))
    mode = str(_select(initial[1], ("minor", "dorian", "major")))
    scale = SCALES[mode]
    degree_by_pitch = {midi(degree, tonic, scale, 60): degree for degree in range(-35, 42)}
    voice_map = dict(voices) if voices is not None else {
        role: _select(np.tanh(initial[i] + .5 * initial[(i + 3) % 8]), VOICE_POOLS[role])
        for i, role in enumerate(ROLES)}
    capture_limits = [*previous_capture_limits, seconds]
    capture_index, checkpoints, capture_exhausted = 0, [], False
    stop_beat = (capture_limits[capture_index] - 1.) / beat_seconds
    events, phrases, decisions, memory = [], [], [], []
    last_pitch, last_degree, last_end = {}, {}, {}
    idle = {role: 0 for role in ROLES}
    recent_roots, span_history = [], []
    previous_voicing = [midi(d, tonic, scale, 48) for d in (0, 2, 4)]
    foreground_pitch = midi(0, tonic, scale, 60)
    foreground_history = []
    position, index, scalar_redirections = 0., 0, 0

    def note(role, beat, degree, duration, velocity, root, decision, primary=False, direct_pitch=None):
        nonlocal foreground_pitch, scalar_redirections
        low, high, base = _REGISTERS[role]
        raw_pitch = midi(degree, tonic, scale, base)
        candidates = [p for p in range(low, high + 1) if p % 12 == raw_pitch % 12]
        target = foreground_pitch if primary else last_pitch.get(role, raw_pitch)
        pitch = int(direct_pitch) if direct_pitch is not None else min(candidates, key=lambda p: (abs(p - target), abs(p - raw_pitch)))
        if role in ("lead", "answer"):
            beat = max(float(beat), last_end.get(role, 0.) + .03)
        duration = min(float(duration), stop_beat - beat - .10)
        if duration <= .08:
            return None
        if role in ("lead", "answer") and len(foreground_history) >= 6:
            steps = np.diff(foreground_history[-6:] + [pitch])
            if all(s in (1, 2) for s in steps) or all(s in (-1, -2) for s in steps):
                chord_pcs = {midi(root + d, tonic, scale, 0) % 12 for d in (0, 2, 4)}
                alternatives = [p for p in range(low, high + 1)
                                if p % 12 in chord_pcs and abs(p - foreground_history[-1]) not in (1, 2)]
                pitch = min(alternatives, key=lambda p: abs(p - pitch))
                scalar_redirections += 1
        last_pitch[role], last_degree[role], last_end[role] = pitch, int(degree_by_pitch[pitch]), beat + duration
        if role in ("lead", "answer"):
            foreground_pitch = pitch
            foreground_history.append(pitch)
        event = {"id": len(events), "layer": role, "voice": voice_map[role], "midi": pitch,
                 "start_beat": round(float(beat), 6), "length_beats": round(duration, 6),
                 "velocity": round(float(np.clip(velocity, .25, .86)), 6), "section": 0,
                 "start_seconds": float(beat) * beat_seconds, "duration_seconds": duration * beat_seconds,
                 "harmonic_root": int(root), "scale_degree": int(degree_by_pitch[pitch]), "decision": int(decision)}
        events.append(event)
        return event

    def hold_harmony(begin, root, span, drive, decision):
        nonlocal previous_voicing
        options = []
        chord = [root, root + 2, root + 4]
        for inversion in range(3):
            ordered = chord[inversion:] + [d + 7 for d in chord[:inversion]]
            for shift in (-7, 0, 7):
                degrees = [d + shift for d in ordered]
                pitches = [midi(d, tonic, scale, 48) for d in degrees]
                if min(pitches) >= 48 and max(pitches) <= 69:
                    cost = sum(abs(a - b) for a, b in zip(previous_voicing, pitches))
                    options.append((cost, degrees, pitches))
        _, degrees, previous_voicing = min(options, key=lambda option: option[0])
        selected = list(zip(degrees, previous_voicing))
        if drive[5] < .2:
            selected = [(d, p) for d, p in selected if (d - root) % 7 != 0]
        spread = _select(drive[6], (0., .06, .18))
        held = span * (.65 + .25 * (drive[5] + 1.) / 2)
        for slot, (degree, pitch) in enumerate(selected):
            note("harmony", begin + .10 + spread * slot, degree, held - spread * slot,
                 .37 + .10 * feedback_energy, root, decision, direct_pitch=pitch)

    while True:
        if position >= stop_beat - 1. or capture_exhausted:
            if capture_index == len(capture_limits) - 1:
                break
            # Preserve the state *after the actually captured final gesture*, not
            # the different memory a fresh long run's uncropped gesture would make.
            ordered = sorted(events, key=lambda e: (e["start_seconds"], ROLES.index(e["layer"]), e["midi"]))
            prefix = [{**event, "id": i} for i, event in enumerate(ordered)]
            checkpoint = {
                "capture_duration_seconds": float(capture_limits[capture_index]),
                "resume_at_beat": position, "resume_at_seconds": position * beat_seconds,
                "source_note_count": len(events),
                "source_event_sha256": hashlib.sha256(json.dumps(prefix, sort_keys=True).encode()).hexdigest(),
                "source_last_note_end_seconds": max(e["start_seconds"] + e["duration_seconds"] for e in events),
                "next_decision_index": index,
                "controller_state_sha256": fingerprint(controller.state),
                "working_memory_sha256": hashlib.sha256(json.dumps(memory, sort_keys=True).encode()).hexdigest(),
                "feedback_energy": feedback_energy, "harmonic_root": harmonic_root,
                "retained_final_gesture": {
                    "decision": decisions[-1]["index"],
                    "selected_span_beats": decisions[-1]["selected_span_beats"],
                    "captured_span_beats": decisions[-1]["captured_span_beats"],
                    "capture_limited": decisions[-1]["capture_limited"],
                },
            }
            checkpoint["capture_padding_removed_seconds"] = capture_limits[capture_index] - checkpoint["resume_at_seconds"]
            if len(controller.trace) != len(decisions):
                # A fractional capture limit may stop after a Controller update
                # but before a short candidate gesture emits notes. That real
                # state advance belongs to the source and must not be discarded.
                checkpoint["next_controller_trace_index"] = len(controller.trace)
                checkpoint["unwritten_controller_steps"] = len(controller.trace) - len(decisions)
            if capture_exhausted:
                checkpoint["capture_exit"] = "Controller advanced, but remaining gesture span was below the note interpreter minimum"
            checkpoints.append(checkpoint)
            capture_index += 1
            capture_exhausted = False
            stop_beat = (capture_limits[capture_index] - 1.) / beat_seconds
            # Keep position unchanged: only recording padding is removed. No
            # extra rest, repeated opening, state reset or composed bridge is added.
            continue
        control = first if index == 0 else np.asarray(controller.phrase(index, feedback_energy, harmonic_root), dtype=float)
        drive = np.tanh(2. * control)
        prior_feedback, prior_root = feedback_energy, harmonic_root
        # Angular readout selects a harmonic destination. Recent use discourages
        # indefinite dwelling; there is no stored progression or final tonic plan.
        root_scores = [1.5 * (drive[0] * math.cos(root * 2 * math.pi / 7)
                              + drive[1] * math.sin(root * 2 * math.pi / 7))
                       - .24 * recent_roots[-4:].count(root)
                       + (.18 if root == harmonic_root else 0.) for root in range(7)]
        harmonic_root = int(np.argmax(root_scores))
        recent_roots.append(harmonic_root)
        # Span is a decision made now. Repeated choices acquire a small memory
        # cost, allowing circuit movement to reshape pacing instead of looping a
        # single fixed phrase size throughout the recording.
        span_options = (2., 3., 4., 5., 6., 8.)
        desired_span = 2. + 6. * (drive[2] + 1.) / 2
        span_scores = [-abs(span - desired_span) - .38 * span_history[-3:].count(span) for span in span_options]
        chosen_span = span_options[int(np.argmax(span_scores))]
        span = min(chosen_span, stop_beat - position - .12)
        span_history.append(chosen_span)
        if span < .9:
            capture_exhausted = True
            continue
        feedback_energy = float(.60 * feedback_energy + .40 * np.clip(.20 + np.mean(np.abs(control)) * 1.3, .15, .9))
        role_scores = {
            "lead": .28 + .75 * drive[4] + .16 * idle["lead"],
            "answer": .12 - .75 * drive[4] + .19 * idle["answer"],
            "harmony": .20 + .75 * drive[5] + .16 * idle["harmony"],
            "bass": .33 + .72 * drive[6] + .14 * idle["bass"],
            "texture": -.20 + .90 * drive[7] + .19 * idle["texture"],
        }
        primary = "lead" if role_scores["lead"] >= role_scores["answer"] else "answer"
        reply = "answer" if primary == "lead" else "lead"
        active = [role for role in ROLES if role_scores[role] > 0]
        if primary not in active:
            active.append(primary)
        do_reply = span >= 3 and role_scores[reply] > -.15
        if do_reply and reply not in active:
            active.append(reply)
        if not do_reply and reply in active:
            active.remove(reply)
        use_memory = bool(memory and drive[3] > -.55)
        memory_index = None
        if use_memory:
            recall_scores = [drive[(slot + 2) % 8] - .16 * item["uses"] for slot, item in enumerate(memory)]
            memory_index = int(np.argmax(recall_scores))
            remembered = memory[memory_index]
            remembered["uses"] += 1
            operation = _select(drive[3], ("recall", "continue", "invert"))
        else:
            remembered, operation = None, "explore"
        response_space = min(1.7, span * .30) if do_reply else .20
        primary_end = span - response_space - .18
        cursor = float(_select(drive[0] - drive[1] * .15, (0., .25, .50)))
        gesture, articulation, actual_primary = [], [], []
        prior_degree = last_degree.get(primary, harmonic_root)
        slot = 0
        while cursor < primary_end - .22 and slot < 12:
            channel = (slot + 1) % 8
            pulse_drive = float(np.tanh(drive[channel] + .35 * drive[(channel + 3) % 8] * (-1 if slot % 2 else 1)))
            interval = _select(pulse_drive, (-4, -2, -1, 1, 2, 4))
            if remembered:
                cell = remembered["degrees"]
                offset = 0 if operation == "recall" else 1 if operation == "continue" else 0
                degree = cell[(slot + offset) % len(cell)] + harmonic_root - remembered["root"]
                if operation == "invert":
                    degree = 2 * (cell[0] + harmonic_root - remembered["root"]) - degree
                if slot and slot % len(cell) == len(cell) - 1:
                    degree += _select(drive[(slot + 5) % 8], (-2, 0, 2))
            else:
                degree = prior_degree + interval
            # Nearest-octave mapping preserves pitch classes with nearby register
            # motion; octave folds may change contour. Openings meet the chord.
            if slot == 0:
                degree = nearest_chord(degree, harmonic_root)
            step = float(_select(pulse_drive, (.5, .75, 1., 1.25, 1.5)))
            if remembered and operation == "recall":
                step = remembered["steps"][slot % len(remembered["steps"])]
            if primary == "answer":
                step = max(.85, step)
            length = min(step * (.65 + .20 * (drive[5] + 1.) / 2), primary_end - cursor)
            event = note(primary, position + cursor, degree, length,
                         .46 + .20 * feedback_energy + .05 * drive[(slot + 4) % 8],
                         harmonic_root, index, primary=True)
            if event:
                gesture.append(event["scale_degree"])
                actual_primary.append(event)
            prior_degree = event["scale_degree"] if event else degree
            rest = 0. if remembered and operation == "recall" else float(_select(drive[(slot + 6) % 8], (0., .15, .30, .5)))
            cursor += step + rest
            slot += 1
        if not actual_primary:
            event = note(primary, position + .08, nearest_chord(prior_degree, harmonic_root),
                         max(.25, span * .45), .47, harmonic_root, index, primary=True)
            if event:
                actual_primary.append(event)
                gesture = [event["scale_degree"]]
        if actual_primary:
            # Remember sounded intervals, including rests, rather than provisional
            # decoder requests. A final note has a duration plus a short breath.
            articulation = [(b["start_seconds"] - a["start_seconds"]) / beat_seconds
                            for a, b in zip(actual_primary, actual_primary[1:])]
            articulation.append(actual_primary[-1]["duration_seconds"] / beat_seconds + .15)
        if do_reply and actual_primary:
            last = actual_primary[-1]
            onset = max(position + span - response_space,
                        (last["start_seconds"] + last["duration_seconds"]) / beat_seconds + .14)
            related = gesture[-1] + _select(drive[6], (-2, 0, 2))
            degree = nearest_chord(related, harmonic_root)
            note(reply, onset, degree, min(response_space * .87, position + span - onset - .10),
                 .38 + .11 * feedback_energy, harmonic_root, index, primary=True)
        if "harmony" in active:
            hold_harmony(position, harmonic_root, span, drive, index)
        if "bass" in active:
            note("bass", position, harmonic_root, min(2.1, span * .44),
                 .51 + .11 * feedback_energy, harmonic_root, index)
            if span > 3 and drive[6] > -.20:
                onset = span * float(_select(drive[2], (.45, .55, .67)))
                note("bass", position + onset, harmonic_root + int(_select(drive[3], (0, 4, 7))),
                     min(1.1, span - onset - .2), .43, harmonic_root, index)
        if "texture" in active:
            count = int(_select(drive[7], (1, 1, 2, 3)))
            for slot in range(count):
                onset = (slot + .65) * span / (count + .5)
                note("texture", position + onset, harmonic_root + int(_select(drive[(slot + 4) % 8], (2, 4, 7))),
                     min(.8, span - onset - .13), .29 + .06 * feedback_energy, harmonic_root, index)
        written = [e for e in events if e["decision"] == index]
        sounded = {e["layer"] for e in written}
        for role in ROLES:
            idle[role] = 0 if role in sounded else idle[role] + 1
        # Memory contains material actually performed, not a predefined theme.
        # A bounded cell store and use penalty avoid retriggering one entire track.
        stored = False
        if gesture and (not memory or operation == "explore" or drive[7] > .28):
            cell = {"phrase": index, "root": harmonic_root, "degrees": gesture[:6],
                    "steps": articulation[:6], "uses": 0}
            if not any(item["degrees"] == cell["degrees"] and item["root"] == cell["root"] for item in memory):
                memory.append(cell)
                if len(memory) > 6:
                    memory.pop(0)
                stored = True
        lead_events = [e for e in written if e["layer"] == "lead"]
        signature = [(e["midi"], round(e["start_beat"] - position, 6), e["length_beats"]) for e in lead_events]
        phrases.append({"index": index, "section": 0, "root": harmonic_root,
                        "start_beat": position, "length_beats": span, "lead_notes": len(lead_events),
                        "lead_signature": hashlib.sha256(json.dumps(signature).encode()).hexdigest(),
                        "activity": control.tolist()})
        decision = {"index": index, "at_seconds": position * beat_seconds,
                          "input_harmonic_root": prior_root, "input_feedback_energy": prior_feedback,
                          "outputs": control.tolist(), "decoded_drive": drive.tolist(),
                          "harmonic_scores": root_scores, "selected_root": harmonic_root,
                          "span_candidates": list(span_options), "span_scores": span_scores,
                          "selected_span_beats": chosen_span, "captured_span_beats": span,
                          "capture_limited": span < chosen_span, "feedback_energy": feedback_energy,
                          "role_scores": role_scores, "sounding_roles": sorted(sounded), "foreground": primary,
                          "memory_operation": operation, "memory_source_phrase": remembered["phrase"] if remembered else None,
                          "stored_new_cell": stored, "performed_degrees": gesture,
                          "performed_rhythm_steps": articulation, "notes_scheduled": len(written)}
        if len(controller.trace) - 1 != index:
            decision["controller_trace_index"] = len(controller.trace) - 1
        decisions.append(decision)
        position += span
        index += 1

    events.sort(key=lambda e: (e["start_seconds"], ROLES.index(e["layer"]), e["midi"]))
    for index, event in enumerate(events):
        event["id"] = index
    event_hash = hashlib.sha256(json.dumps(events, sort_keys=True).encode()).hexdigest()
    score = {"version": 1, "kind": "performance", "performer": "fly",
            "arrangement_version": "autonomous_rate_performance_v1", "title": f"Fly performance {seed}",
            "seed": seed, "condition": condition, "duration_seconds": seconds, "bpm": bpm,
            "tonic": _TONICS[tonic], "mode": mode, "meter": "4/4", "beats_per_bar": 4,
            "bars": int(math.ceil(stop_beat / 4)), "voices": voice_map, "events": events,
            "phrases": phrases, "progression": recent_roots,
            "sections": [{"name": "Continuous capture", "start_seconds": 0., "end_seconds": seconds - 1.,
                          "scope": "Audio measurement window; not a planned musical section"}],
            "event_sha256": event_hash, "controller": controller.identity, "neural_trace": controller.trace,
            "decision_log": decisions, "musical_memory": memory, "circuit_metadata": circuit.get("metadata", {}),
            "decoder_provenance": {
                "engine": "persistent Controller state, current readout and bounded musical memory",
                "planned_form": None, "planned_progression": None, "prewritten_melody": None,
                "voice_selection": "explicit override" if voices is not None else "initial neural readout selects from role-compatible pools",
                "voice_pools": {role: list(pool) for role, pool in VOICE_POOLS.items()},
                "initial_outputs": first.tolist(), "readout_gain": 2.,
                "activity_feedback": "0.60 previous + 0.40 clip(0.20 + mean(abs(outputs))*1.3, 0.15, 0.9)",
                "memory": "Up to six performed cells; neural recall/continuation/inversion/exploration with use penalty",
                "harmonic_constraint": "Diatonic pitch classes, recent-root dwell cost and closest chord inversions",
                "rhythmic_constraint": "Variable neural phrase spans; bounded note lengths, rests and monophonic foreground",
                "scalar_run_redirections": scalar_redirections,
                "capture_stop": "Bound each final note before second (duration-1); one second reserved for release silence",
                "limitations": "Rate dynamics, projections, interpretation and musical memory are engineered. No biological cognition, trained taste or quality claim."}}
    if checkpoints:
        score["continuation"] = {
            "method": "Deterministic replay of saved capture boundaries, then continuation of the same Controller and musical memory",
            "status": "complete", "target_seconds": seconds, "actual_seconds": seconds,
            "new_notes": len(events) - checkpoints[-1]["source_note_count"], "target_reached": True,
            "capture_limits_seconds": [float(value) for value in capture_limits],
            "checkpoints": checkpoints,
            "source_event_sha256": checkpoints[-1]["source_event_sha256"],
            "source_note_count": checkpoints[-1]["source_note_count"],
            "source_duration_seconds": checkpoints[-1]["capture_duration_seconds"],
            "resume_at_seconds": checkpoints[-1]["resume_at_seconds"],
            "prefix_guarantee": "Exact authored note ledger; source IDs, times, lengths, pitches and instruments retained",
            "seam": "Retain the source's shortened final gesture; resume at its next gesture boundary without capture-only trailing silence",
            "audio_scope": "No mastered PCM-prefix guarantee; recording-level gain may differ and new notes replace prior capture padding",
        }
    return score
