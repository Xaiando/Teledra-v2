"""Three development studies with deliberately different musical grammars.

The fly-derived Controller supplies evolving musical choices. Meter, harmony,
phrase grammar, orchestration and cadence are engineered composition rules,
not claims about biological musical ability. No model service is used here.
"""
from __future__ import annotations

import hashlib
import json

import numpy as np

from .composition import ENSEMBLES, ROLES, SCALES, choose, midi, nearest_chord
from .controller import Controller


DIRECTIONS = {
    "nocturne": {
        "name": "Lantern Room",
        "description": "A spacious 4/4 dialogue: long breaths, spare low chords and an answering voice.",
    },
    "tide": {
        "name": "Glass Tide",
        "description": "A lilting 6/8 study: an uneven swell, held support and a rolling upper voice.",
    },
    "sparks": {
        "name": "Crossing Sparks",
        "description": "A syncopated 4/4 study: interlocking bass and upper parts, displaced phrases and a quiet breakdown.",
    },
}

_CONFIG = {
    "nocturne": {"meter": "4/4", "bar": 4, "tempos": (64, 69, 74),
                 "modes": ("major", "dorian", "minor"),
                 "forms": ((3, 3, 3, 2), (2, 4, 3, 2), (3, 3, 4, 2)),
                 "sections": ("A light appears", "Two voices", "Open windows", "The room settles"),
                 "energy": (.30, .53, .77, .29)},
    "tide": {"meter": "6/8", "bar": 3, "tempos": (87, 96, 105),
             "modes": ("dorian", "major", "minor"),
             "forms": ((3, 4, 3, 3), (4, 3, 4, 2), (3, 3, 4, 3)),
             "sections": ("First ripple", "Gathering current", "The crest", "Receding shore"),
             "energy": (.35, .60, .88, .38)},
    "sparks": {"meter": "4/4", "bar": 4, "tempos": (108, 116, 124),
               "modes": ("minor", "dorian", "major"),
               "forms": ((2, 3, 2, 3, 2), (3, 3, 2, 3, 2), (2, 4, 2, 3, 2)),
               "sections": ("The mechanism wakes", "Interlock", "Almost still", "New bearings", "Lights out"),
               "energy": (.44, .81, .25, .91, .37)},
}


def _pitch_class(degree, tonic, scale):
    return midi(degree, tonic, scale, 0) % 12


def _thematic_cell(rng, length):
    """An asymmetric contour, with steps, skips and a purposeful return."""
    result = [int(rng.choice((0, 2, 4)))]
    contour = int(rng.choice((-1, 1)))
    for index in range(1, length - 1):
        steps = (2, 3, -1, -2) if contour > 0 else (-2, -3, 1, 2)
        step = int(rng.choice(steps))
        value = result[-1] + step
        if not -2 <= value <= 9:
            value = result[-1] - step
        result.append(int(np.clip(value, -2, 9)))
        if index == length // 2:
            contour *= -1
    result.append(int(rng.choice((0, 2, 4))))
    return result


def _plans(rng, config, direction, mode):
    """Draw the musical brief before circuit evaluation, so controls share it."""
    counts = config["forms"][int(rng.integers(len(config["forms"])))]
    major = mode == "major"
    routes = {
        "nocturne": ((0, 5, 3, 1), (0, 2, 3, 5), (5, 3, 0, 1)) if major else
                    ((0, 5, 3, 4), (0, 3, 6, 2), (5, 2, 3, 0)),
        "tide": ((0, 3, 1, 4), (0, 5, 1, 4), (3, 4, 2, 5)) if major else
                ((0, 6, 3, 4), (0, 3, 5, 6), (5, 6, 2, 4)),
        "sparks": ((0, 5, 1, 4), (0, 3, 2, 5), (5, 1, 3, 4)) if major else
                  ((0, 3, 5, 4), (0, 6, 2, 3), (3, 0, 5, 4)),
    }[direction]
    plans, sections = [], []
    position = 0.
    for section, count in enumerate(counts):
        route = routes[int(rng.integers(len(routes)))]
        route_offset = int(rng.integers(len(route))) if section else 0
        section_start = position
        for local in range(count):
            # Occasional extensions and contractions change phrase architecture,
            # rather than padding every idea to the same two bars.
            bars = 2
            if direction in ("nocturne", "tide") and local == count - 1 and section == 1:
                bars = 3
            if direction == "sparks" and section in (0, 2) and local == 0:
                bars = 1
            root = route[(local + route_offset) % len(route)]
            if section == 0 and local == 0:
                root = 0
            if section == len(counts) - 1 and local >= count - 2:
                root = 4 if local == count - 2 else 0
            # Each phrase's timing and ornament priors are frozen exogenous data.
            plans.append({"index": len(plans), "section": section, "local": local,
                          "start": position, "length": bars * config["bar"],
                          "bars": bars, "root": root,
                          "second_root": route[(local + route_offset + 1) % len(route)],
                          "energy": config["energy"][section],
                          "rhythm_bias": rng.uniform(-1, 1, 16),
                          "motif_offset": int(rng.integers(7)),
                          "variant": int(rng.integers(4)),
                          "hold": float(rng.uniform(.77, .93))})
            position += bars * config["bar"]
        sections.append({"name": config["sections"][section], "start_beat": section_start,
                         "end_beat": position, "target_energy": config["energy"][section]})
    return plans, sections, position


def _rhythm(window_start, window_end, count, control, bias, direction):
    """A bounded phrase grammar: choose occupied positions, then articulate them.

    Counts vary with energy. Empty positions remain real rests; durations cannot
    run into the next foreground note or the answering voice's window.
    """
    grid = .5 if direction == "tide" else .25
    positions = np.arange(window_start, window_end - .25 + 1e-8, grid)
    count = min(count, len(positions))
    if count < 1:
        return []
    # Spread candidates across the phrase with phase-specific bias. Onset choice
    # responds continuously to the circuit before final metric quantization.
    ranked = []
    for i, beat in enumerate(positions):
        accent = .38 if (direction == "tide" and round(beat * 2) % 3 == 0) else 0.
        if direction == "sparks":
            accent += .42 if round(beat * 4) % 4 in (1, 3) else -.08
        score = np.sin(i * (1.18 + .4 * control[0]) + 1.7 * control[1])
        score += .53 * bias[i % len(bias)] + accent
        ranked.append((float(score), i))
    # Divide into count bins to retain a phrase contour without uniform steps.
    bins = np.array_split(np.arange(len(positions)), count)
    scores = {i: value for value, i in ranked}
    selected = [max(group.tolist(), key=lambda i: scores[i]) for group in bins]
    starts = sorted(float(positions[index]) for index in selected)
    result = []
    for i, beat in enumerate(starts):
        available = (starts[i + 1] if i + 1 < len(starts) else window_end) - beat
        articulation = (.69 if direction == "sparks" else .84) + .08 * float(control[2])
        length = max(.13, available * articulation)
        if direction == "nocturne":
            length = min(length, 1.8)
        result.append((beat, min(length, available - .04)))
    return result


def write_development(circuit, seed=42, ensemble="luminous", condition="connectome",
                      voices=None, direction="nocturne", revision=None):
    """Return a complete explicit score, compatible with the existing renderer.

    ``bpm`` and beat coordinates always use quarter-note units. Thus 6/8 has
    ``beats_per_bar == 3`` and a dotted-quarter pulse at ``bpm * 2 / 3``.
    Every voice map applies identically across musical directions and controls.
    Omit ``revision`` to reproduce the original development_v1 score. The explicit
    Sparks revision ``connected_phrases_v1`` develops that same score's foreground
    continuity without changing its accompaniment, form or neural activity.
    """
    if direction not in DIRECTIONS:
        raise ValueError("Choose nocturne, tide or sparks")
    if revision not in (None, "connected_phrases_v1"):
        raise ValueError("Unknown musical revision")
    if revision is not None and direction != "sparks":
        raise ValueError("The connected-phrase revision is only available for sparks")
    if ensemble not in ENSEMBLES or type(seed) is not int or not 0 <= seed <= 2**31 - 1:
        raise ValueError("Choose a supported ensemble and integer seed in 0..2147483647")
    voice_map = dict(ENSEMBLES[ensemble] if voices is None else voices)
    if set(voice_map) != set(ROLES) or any(not isinstance(v, str) or not v for v in voice_map.values()):
        raise ValueError("Exactly five named instrumental parts are required")
    config = _CONFIG[direction]
    # Same seed means a reproducible brief. Direction is a deliberately changed
    # composition grammar, not a hidden controller condition.
    rng = np.random.default_rng(seed)
    tonic = int(rng.choice((0, 2, 5, 7, 9)))
    mode = str(rng.choice(config["modes"]))
    scale = SCALES[mode]
    bpm = int(rng.choice(config["tempos"]))
    beat_seconds = 60 / bpm
    theme = _thematic_cell(rng, int(rng.integers(7, 12)))
    companion = [theme[0], theme[2] + 2, theme[-3], theme[1], theme[-1]]
    plans, sections, total_beats = _plans(rng, config, direction, mode)
    controller = Controller(circuit, seed, condition)
    events, phrases = [], []
    lead_history, last_pitch = [], {}
    scalar_repairs = 0
    previous_voicing = [midi(d, tonic, scale, 48) for d in (2, 4, 7)]

    def register(role, degree):
        base = {"lead": 60, "answer": 60, "harmony": 48, "bass": 36, "texture": 72}[role]
        low, high = {"lead": (58, 82), "answer": (60, 83), "harmony": (45, 72),
                     "bass": (33, 55), "texture": (69, 91)}[role]
        pitch = midi(degree, tonic, scale, base)
        choices = [p for p in range(low, high + 1) if p % 12 == pitch % 12]
        target = last_pitch.get(role, pitch)
        return min(choices, key=lambda p: (abs(p - target), abs(p - pitch)))

    def note(role, beat, degree, length, velocity, section, root, anchor=False):
        nonlocal scalar_repairs
        if anchor:
            degree = nearest_chord(degree, root)
        pitch = register(role, degree)
        # Explicit musical constraint: a long mechanical staircase is redirected
        # to a nearby chord tone. This operates on the actual sounded MIDI notes.
        if role == "lead" and len(lead_history) >= 6:
            recent = np.diff(lead_history[-6:] + [pitch])
            if all(x in (1, 2) for x in recent) or all(x in (-1, -2) for x in recent):
                candidates = [p for p in range(58, 83)
                              if p % 12 in {_pitch_class(root + d, tonic, scale) for d in (0, 2, 4)}
                              and abs(p - lead_history[-1]) not in (1, 2)]
                pitch = min(candidates, key=lambda p: (abs(p - pitch), abs(p - lead_history[-1])))
                scalar_repairs += 1
        length = min(float(length), total_beats - beat - .16)
        if length <= .04:
            return
        last_pitch[role] = pitch
        if role == "lead":
            lead_history.append(pitch)
        events.append({"id": len(events), "layer": role, "voice": voice_map[role],
                       "midi": int(pitch), "start_beat": round(float(beat), 5),
                       "length_beats": round(length, 5),
                       "velocity": round(float(np.clip(velocity, .25, .90)), 5),
                       "section": int(section), "harmonic_root": int(root),
                       "start_seconds": float(beat) * beat_seconds,
                       "duration_seconds": length * beat_seconds})

    def harmony(beat, root, length, velocity, section, dyad=False, spread=0.):
        nonlocal previous_voicing
        chord = [root, root + 2, root + 4]
        candidates = []
        for inversion in range(3):
            rotated = chord[inversion:] + [d + 7 for d in chord[:inversion]]
            for shift in (-7, 0, 7):
                degrees = [d + shift for d in rotated]
                pitches = [midi(d, tonic, scale, 48) for d in degrees]
                if min(pitches) >= 45 and max(pitches) <= 72:
                    candidates.append((sum(abs(a - b) for a, b in zip(pitches, previous_voicing)), degrees, pitches))
        _, degrees, previous_voicing = min(candidates, key=lambda c: c[0])
        if dyad:
            # Keep the third and upper voice, while the bass supplies the root.
            degrees = [d for d in degrees if (d - root) % 7 != 0]
        # Harmony register is selected explicitly as a voicing, not a monophonic
        # register walk, so close inversions retain their intended spacing.
        for index, degree in enumerate(degrees):
            last_pitch.pop("harmony", None)
            note("harmony", beat + spread * index, degree, length - spread * index,
                 velocity, section, root)

    for plan in plans:
        index, section, local = plan["index"], plan["section"], plan["local"]
        start, length, root, energy = plan["start"], plan["length"], plan["root"], plan["energy"]
        final = index == len(plans) - 1
        control = np.asarray(controller.phrase(index, energy, root), dtype=float)
        # Expand modest rate readouts into useful musical decision ranges. This
        # decoding gain is fixed, documented and shared with wiring controls.
        drive = np.tanh(1.8 * control)

        def chord_at(onset):
            if final:
                return 0
            if direction == "nocturne":
                return plan["second_root"] if energy >= .7 and onset >= length * .53 else root
            if direction == "tide":
                return root if onset < 3 or local % 2 == 0 else plan["second_root"]
            return root if onset < 4 else plan["second_root"]

        returning = local == 0 and section > 0
        subject = theme if section % 2 == 0 or returning else companion
        offset = 0 if returning else (plan["motif_offset"] + choose(drive[1], (0, 1, 2))) % len(subject)
        phrase_subject = subject[offset:] + subject[:offset]
        transpose = 0 if returning else choose(drive[3], (-2, 0, 2))
        if direction == "nocturne":
            # The two foreground voices take turns, including a reversed dialogue
            # in the middle. Harmony is absent for one deliberate exposed phrase.
            exchange = section == 1 and local % 2 == 1
            lead_begin = length * .47 if exchange else choose(drive[0], (0., .25, .5))
            lead_end = length - .7 if exchange else length * .50
            lead_count = 3 + int(energy > .45) + int(energy > .7) + int(drive[4] > .25)
            if index == 0:
                lead_count = 3
        elif direction == "tide":
            exchange = section == 2 and local % 3 == 1
            lead_begin = 1.5 if exchange else choose(drive[0], (0., .5, 1.))
            lead_end = length - (1.5 if local % 2 == 0 else .75)
            lead_count = 4 + int(energy > .5) + int(energy > .8) + int(drive[4] > .15)
        else:
            exchange = section == 2 or (section == 3 and local == 1)
            lead_begin = choose(drive[0], (.25, .75, 1.25))
            lead_end = length - (.75 if exchange else 1.5)
            lead_count = 3 + int(energy > .5) * 2 + int(drive[4] > .10)
            if length == 4:
                lead_count = min(lead_count, 4)
            if section == 2:
                lead_count = 2 + int(local != 0)
        if final:
            lead_begin, lead_end, lead_count = .5, max(2., length - 3.2), 3
        rhythm = _rhythm(lead_begin, lead_end, lead_count, drive,
                         plan["rhythm_bias"], direction)
        actual_degrees = []
        for slot, (onset, duration) in enumerate(rhythm):
            active_root = chord_at(onset)
            degree = phrase_subject[slot % len(phrase_subject)] + transpose
            if slot == len(rhythm) // 2 and not returning:
                degree += choose(drive[2], (-2, 0, 2))
            if slot == 0:
                degree = nearest_chord(degree, active_root)
            if slot == len(rhythm) - 1 and index % 2:
                degree = nearest_chord(degree, active_root)
            # Accented phrases open their register at the crest without making
            # every seed carry a permanent high ostinato.
            if energy > .8 and slot == 2:
                degree += 2
            actual_degrees.append(degree)
            note("lead", start + onset, degree, duration,
                 .46 + .21 * energy + .075 * drive[5] - .025 * (slot % 2), section, active_root)
        if final:
            # Lead tonic, an actual breath, and a coordinated release for all
            # layers; the tail remains inside the complete render bounds.
            note("lead", start + length - 2.8, 0, 2.35, .46, section, 0, anchor=True)

        # A related reply uses the actual phrase tail in reverse or continuation,
        # with long theremin notes and end anchors rather than a duplicate scale.
        answer_notes = []
        if index > 0 and not final:
            if direction == "nocturne":
                answer_begin, answer_end = ((.4, length * .42) if exchange else
                                            (length * .57, length - .3))
                count = 2 if energy < .65 else 3
            elif direction == "tide":
                answer_begin, answer_end = (max(0., lead_end + .20), length - .2)
                count = 1 if answer_end - answer_begin < 1.4 else 2
                if exchange:
                    answer_begin, answer_end, count = .0, 1.25, 1
            else:
                answer_begin, answer_end = (lead_end + .18, length - .15)
                count = 1
                if exchange:
                    answer_begin, answer_end, count = .0, min(2., length - .2), 1
            if answer_end - answer_begin > .45:
                spacing = (answer_end - answer_begin) / count
                response = list(reversed(actual_degrees[-3:])) or [root + 2]
                for slot in range(count):
                    onset = answer_begin + slot * spacing
                    active_root = chord_at(onset)
                    degree = response[slot % len(response)] + choose(drive[6], (-2, 0, 2))
                    if slot == 0 or slot == count - 1:
                        degree = nearest_chord(degree, active_root)
                    note("answer", start + onset, degree, spacing * .85,
                         .39 + .075 * energy, section, active_root)
                    answer_notes.append(onset)
        elif final:
            note("answer", start + .15, 2, min(2., length * .32), .33, section, 0, anchor=True)
            answer_notes.append(.15)

        # Harmonic pace and accompaniment grammar deliberately differ by study.
        if direction == "nocturne":
            exposed = section == 1 and local == 1
            if not exposed:
                harmony(start + .12, root, length * (.91 if energy < .7 else .46),
                        .38 + energy * .08, section, dyad=energy < .65, spread=.10)
                if energy >= .7 and not final:
                    harmony(start + length * .53, plan["second_root"], length * .40,
                            .40, section, dyad=False, spread=.08)
            note("bass", start, root, min(3.2, length * .4), .49 + .10 * energy, section, root)
            if index > 0 and not final and not exposed:
                onset = length * .54
                active_root = chord_at(onset)
                note("bass", start + onset, active_root + choose(drive[2], (0, 4, 7)),
                     min(2.4, length * .27), .42, section, active_root)
            if index > 1 and not final and (energy > .7 or local % 2 == 0):
                onset = length * .5 + .25
                active_root = chord_at(onset)
                note("texture", start + onset, active_root + choose(drive[7], (2, 4, 7)),
                     1.2, .31, section, active_root)
                if energy > .7:
                    note("texture", start + length - 1.1, plan["second_root"] + 4,
                         .7, .28, section, plan["second_root"])
        elif direction == "tide":
            for bar in range(plan["bars"]):
                bar_start = start + bar * 3
                chord = root if final or bar == 0 or local % 2 == 0 else plan["second_root"]
                if not (index == 0 or section == 3 and local == 0):
                    harmony(bar_start + .05, chord, 2.66 if energy > .5 else 2.25,
                            .39 + energy * .08, section, dyad=energy < .8, spread=.025)
                note("bass", bar_start, chord, 1.1 if not final else 2.6,
                     .54, section, chord)
                if not final and energy > .45 and bar % 2 == 0:
                    note("bass", bar_start + 1.5, chord + 4, .95, .42, section, chord)
                if index > 0 and not final and not (section == 3 and local == 0):
                    order = (2, 4, 7) if drive[7] > 0 else (7, 4, 2)
                    positions = (.5, 1., 2.) if energy > .55 else (1., 2.)
                    for slot, onset in enumerate(positions):
                        note("texture", bar_start + onset, chord + order[slot],
                             .40 if slot < 2 else .62, .29 + .045 * energy, section, chord)
        else:
            breakdown = section == 2
            for bar in range(plan["bars"]):
                bar_start = start + bar * 4
                chord = root if final or bar == 0 else plan["second_root"]
                if final:
                    harmony(bar_start, 0, 3.4, .39, section, dyad=bar == 0)
                    note("bass", bar_start, 0, 2.8, .49, section, 0)
                    continue
                if breakdown:
                    if bar == 0:
                        harmony(bar_start + .35, chord, 3.1, .35, section, dyad=True)
                        note("bass", bar_start, chord, 1.65, .46, section, chord)
                    if local > 0:
                        note("texture", bar_start + 2.75, chord + 4, .55, .28, section, chord)
                    continue
                if index > 0:
                    for onset in (.5, 2.5) if energy > .7 else (1.5,):
                        harmony(bar_start + onset, chord, .64 if energy > .7 else 1.1,
                                .41 + energy * .07, section, dyad=energy < .8)
                bass_positions = (0., 1.5, 2.75) if drive[2] >= 0 else (0., 1.75, 3.)
                texture_positions = (.75, 2.25, 3.5) if drive[2] >= 0 else (.5, 1.25, 2.5)
                if index == 0:
                    bass_positions, texture_positions = bass_positions[:2], ()
                for slot, onset in enumerate(bass_positions):
                    degree = chord + (0, 4, choose(drive[3], (0, 2, 7)))[slot]
                    note("bass", bar_start + onset, degree, .60 if slot == 0 else .34,
                         .61 if slot == 0 else .47, section, chord)
                if index > 0 and (energy > .6 or bar == 0):
                    for slot, onset in enumerate(texture_positions):
                        degree = chord + choose(drive[(slot + 4) % 8], (2, 4, 7, 9))
                        note("texture", bar_start + onset, degree, .23 if slot < 2 else .38,
                             .31 + .04 * energy, section, chord)

        sounding = [e for e in events if e["layer"] == "lead" and start <= e["start_beat"] < start + length]
        fingerprint = [(e["midi"], round(e["start_beat"] - start, 5), e["length_beats"]) for e in sounding]
        phrases.append({"index": index, "section": section, "root": root,
                        "start_beat": start, "length_beats": length,
                        "lead_signature": hashlib.sha256(json.dumps(fingerprint).encode()).hexdigest(),
                        "lead_notes": len(sounding), "activity": control.tolist(),
                        "lead_window": [lead_begin, lead_end], "answer_onsets": answer_notes,
                        "thematic_return": returning, "motif_offset": offset})

    # Sort all explicit starts for simple scheduling and deterministic note IDs.
    events.sort(key=lambda e: (e["start_seconds"], ROLES.index(e["layer"]), e["midi"]))
    for index, event in enumerate(events):
        event["id"] = index
    for section in sections:
        section.update(start_seconds=section["start_beat"] * beat_seconds,
                       end_seconds=section["end_beat"] * beat_seconds)
    event_hash = hashlib.sha256(json.dumps(events, sort_keys=True).encode()).hexdigest()
    score = {"version": 1, "arrangement_version": "development_v1", "direction": direction,
            "title": f"{DIRECTIONS[direction]['name']} {seed}", "seed": seed, "condition": condition,
            "tonic": ("C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B")[tonic],
            "mode": mode, "bpm": bpm, "meter": config["meter"], "beats_per_bar": config["bar"],
            "pulse_bpm": bpm * 2 / 3 if direction == "tide" else bpm,
            "bars": int(total_beats / config["bar"]), "duration_seconds": total_beats * beat_seconds + 1.,
            "voices": voice_map, "motif": theme, "companion_motif": companion,
            "progression": [plan["root"] for plan in plans], "events": events, "phrases": phrases,
            "sections": sections, "event_sha256": event_hash,
            "controller": {**controller.identity, "musical_readout_gain": 1.8},
            "neural_trace": controller.trace, "circuit_metadata": circuit.get("metadata", {}),
            "composition_rules": {"form": "Seed-selected sectional plan with extensions and contractions",
                                  "direction": DIRECTIONS[direction]["description"],
                                  "harmonic_plan": "Seed-selected diatonic routes and final dominant-to-tonic cadence",
                                  "scalar_run_redirections": scalar_repairs,
                                  "neural_choices": "Foreground timing, articulation, contour development, response intervals, accompaniment rhythm and dynamics",
                                  "listening_claim": "Musical differences are deliberate; quality requires listener judgment"}}
    return _connect_sparks_phrases(score) if revision is not None else score


def _connect_sparks_phrases(score):
    """Develop an existing Sparks performance while retaining its identity.

    This explicit score transformation follows listener feedback about fragmented
    phrases. It moves/holds existing foreground notes; it adds no notes or backing
    beds. The original seed, notes from the circuit, harmonic route, accompaniment
    and cadence remain the starting material. Listening must judge the result.
    """
    source_hash = score["event_sha256"]
    events, phrases = score["events"], score["phrases"]
    beat_seconds = 60 / score["bpm"]
    tonic = ("C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B").index(score["tonic"])
    scale = SCALES[score["mode"]]
    degrees_by_pitch = {midi(d, tonic, scale, 60): d for d in range(-21, 28)}
    foreground = []
    for phrase in phrases:
        begin, end = phrase["start_beat"], phrase["start_beat"] + phrase["length_beats"]
        foreground.append({role: [e for e in events if e["layer"] == role and begin <= e["start_beat"] < end]
                           for role in ("lead", "answer")})
    section_heads = {}
    for phrase, parts in zip(phrases, foreground):
        section_heads.setdefault(phrase["section"], (phrase["index"],
                                 [degrees_by_pitch[e["midi"]] for e in parts["lead"][:2]]))
    changes = {"opening_note_count": len(foreground[0]["lead"]),
               "paired_theme_echoes": [], "foreground_handoffs": [],
               "breakdown_exit_phrase": None}

    def timing(event, beat=None, length=None):
        if beat is not None:
            event["start_beat"] = round(float(beat), 5)
            event["start_seconds"] = float(beat) * beat_seconds
        if length is not None:
            event["length_beats"] = round(float(length), 5)
            event["duration_seconds"] = float(length) * beat_seconds

    def end_beat(event):
        # Full-precision seconds are authoritative, as in the renderer.
        return (event["start_seconds"] + event["duration_seconds"]) / beat_seconds

    # Give the existing opening gesture one bar to speak. The short first bar's
    # motif pitches survive exactly, including its offbeat shape and final breath.
    opening = foreground[0]["lead"]
    opening_end = phrases[0]["length_beats"]
    opening_spacing = min(.85, (opening_end - 1.) / max(1, len(opening)))
    for slot, event in enumerate(opening):
        onset = .5 + slot * opening_spacing
        duration = opening_spacing * .83 if slot < len(opening) - 1 else min(.78, opening_end - onset - .4)
        timing(event, onset, duration)

    for phrase, parts in zip(phrases, foreground):
        index, section = phrase["index"], phrase["section"]
        lead, answer = parts["lead"], parts["answer"]
        begin, length = phrase["start_beat"], phrase["length_beats"]
        final = index == len(phrases) - 1
        head_index, head = section_heads[section]
        paired = (index - head_index) % 2 == 1 and not final

        if paired and len(lead) >= 2 and len(head) == 2:
            # Reuse the section's short diatonic contour instead of introducing
            # another rotated opening. Choose a chordal transposition and octave
            # together so a head echo does not introduce a new register jump.
            difference = head[1] - head[0]
            root = lead[0]["harmonic_root"]
            previous = foreground[index - 1]["lead"][-1]["midi"]
            candidates = []
            for degree in range(-14, 21):
                if (degree - root) % 7 not in (0, 2, 4):
                    continue
                pitches = [midi(degree + step, tonic, scale, 60) for step in (0, difference)]
                if all(58 <= pitch <= 82 for pitch in pitches):
                    cost = .6 * abs(pitches[0] - previous)
                    cost += .4 * abs(pitches[0] - lead[0]["midi"]) + .1 * abs(pitches[1] - lead[1]["midi"])
                    candidates.append((cost, pitches))
            if candidates:
                _, pitches = min(candidates, key=lambda row: (row[0], row[1]))
                for event, pitch in zip(lead[:2], pitches):
                    event["midi"] = int(pitch)
                changes["paired_theme_echoes"].append({"phrase": index, "source_phrase": head_index,
                                                       "diatonic_interval": difference})

        if index > 0 and not final:
            early_answer = bool(answer and answer[0]["start_beat"] < lead[0]["start_beat"])
            if section == 2:
                # Keep the breakdown's low note count. An opening response hands
                # over before the melody, instead of speaking on top of it.
                if answer:
                    timing(answer[0], begin, 1.50)
                timing(lead[0], begin + 1.75)
                if length == 4 and len(lead) == 2:
                    timing(lead[1], begin + 2.75)
                # The final held melody carries the quiet idea toward re-entry;
                # it still releases a quarter beat before the next phrase.
                last = lead[-1]
                timing(last, length=begin + length - .25 - last["start_beat"])
                changes["breakdown_exit_phrase"] = index
            elif not early_answer:
                # A late randomized first attack created a second large gap after
                # the previous response. Advance only that attack and let it ring.
                first = lead[0]
                if first["start_beat"] > begin + 1.:
                    timing(first, begin + .75)
                    available = lead[1]["start_beat"] - first["start_beat"] if len(lead) > 1 else 1.5
                    timing(first, length=min(1.4, available - .12))
                # The phrase's final articulation joins its own answer, while
                # internal syncopations and rests retain their existing lengths.
                last = lead[-1]
                limit = begin + phrase["lead_window"][1] - .10
                wanted_end = min(limit, end_beat(last) + .35)
                if wanted_end > end_beat(last):
                    timing(last, length=wanted_end - last["start_beat"])

            # Timing changes above may shorten a first-note slot. Keep the real
            # lead monophonic; the short breath at every such join is intentional.
            for a, b in zip(lead, lead[1:]):
                if end_beat(a) > b["start_beat"] + 1e-8:
                    timing(a, length=b["start_beat"] - a["start_beat"] - .08)

        for event in answer:
            comes_first = event["start_beat"] < lead[0]["start_beat"]
            if comes_first:
                prior_parts = foreground[index - 1] if index else None
                prior = max((e for notes in prior_parts.values() for e in notes), key=end_beat)["midi"] if prior_parts else lead[0]["midi"]
                target = (prior + lead[0]["midi"]) / 2
            else:
                last = lead[-1]
                target = last["midi"]
                if not final:
                    onset = max(end_beat(last) + .14, event["start_beat"] - .40)
                    ending = begin + length - .12
                    # Retain a late answering phrase, without an octave detour or
                    # a second detached gap after the lead has already breathed.
                    if ending - onset > .12:
                        timing(event, onset, min(1.8, ending - onset))
            choices = [p for p in range(60, 84) if p % 12 == event["midi"] % 12]
            event["midi"] = min(choices, key=lambda p: (abs(p - target), abs(p - event["midi"])))
            changes["foreground_handoffs"].append({"phrase": index, "answer_first": comes_first})

    # New head echoes can redirect a contour. Reapply the existing anti-staircase
    # constraint to the actual revised notes, not a metric-only phrase signature.
    lead = sorted((e for e in events if e["layer"] == "lead"), key=lambda e: e["start_seconds"])
    redirected = 0
    for index in range(6, len(lead)):
        intervals = np.diff([e["midi"] for e in lead[index - 6:index + 1]])
        if all(x in (1, 2) for x in intervals) or all(x in (-1, -2) for x in intervals):
            event, previous = lead[index], lead[index - 1]["midi"]
            roots = {_pitch_class(event["harmonic_root"] + d, tonic, scale) for d in (0, 2, 4)}
            choices = [p for p in range(58, 83) if p % 12 in roots and abs(p - previous) not in (1, 2)]
            event["midi"] = min(choices, key=lambda p: (abs(p - event["midi"]), abs(p - previous)))
            redirected += 1

    events.sort(key=lambda e: (e["start_seconds"], ROLES.index(e["layer"]), e["midi"]))
    for index, event in enumerate(events):
        event["id"] = index
    for phrase, parts in zip(phrases, foreground):
        start = phrase["start_beat"]
        data = [(e["midi"], round(e["start_beat"] - start, 5), e["length_beats"]) for e in parts["lead"]]
        phrase["lead_signature"] = hashlib.sha256(json.dumps(data).encode()).hexdigest()
        phrase["lead_window"] = [parts["lead"][0]["start_beat"] - start,
                                  max(end_beat(e) for e in parts["lead"]) - start]
        phrase["answer_onsets"] = [e["start_beat"] - start for e in parts["answer"]]
    score.update(revision="connected_phrases_v1", arrangement_version="development_connected_phrases_v1",
                 revision_source_event_sha256=source_hash,
                 title=f"Crossing Sparks {score['seed']} - connected phrases",
                 event_sha256=hashlib.sha256(json.dumps(events, sort_keys=True).encode()).hexdigest())
    score["composition_rules"].update(
        continuity_revision={**changes, "added_notes": 0, "retained_accompaniment": ["bass", "harmony", "texture"],
                             "description": "Opening spread, related phrase heads, closer response register and a held breakdown handoff"},
        scalar_run_redirections=score["composition_rules"]["scalar_run_redirections"] + redirected)
    return score
