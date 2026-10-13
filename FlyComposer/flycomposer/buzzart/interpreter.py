"""Buzzart's own note interpreter.

A copy of the Fly engine's autonomous_rate_performance_v1 interpreter, owned by
Buzzart so it can take a musical foundation without changing the Fly engine.
With no foundation it produces exactly the Fly engine's notes (tests pin this).
With a foundation, three hand-picked tables give way to real-music statistics:
melodic intervals, note lengths and rests keep the songbook's proportions while
the circuit's drive decides where each note falls in them; chord roots lean
toward real progressions; and musical memory opens with real motifs to recall,
continue and invert.
"""
from __future__ import annotations

import hashlib
import json
import math

import numpy as np

from ..composition import ROLES, SCALES, midi, nearest_chord
from .circuit import BrainController
from .console_style import DRUM_LENGTH, DRUM_PITCH, DRUM_VELOCITY

ENGINE = "buzzart_interpreter_v1"
VOICE_POOLS = {
    "lead": ("electric_piano", "piano", "marimba", "clavinet"),
    "answer": ("theremin", "clarinet", "oboe", "pan_flute"),
    "harmony": ("cello", "viola", "warm_pad", "string_ensemble"),
    "bass": ("classic:bass", "double_bass", "bass", "bassoon"),
    "texture": ("vibraphone", "harp", "celesta", "mandolin"),
}
REGISTERS = {"lead": (58, 82, 60), "answer": (60, 83, 60), "harmony": (48, 69, 48),
             "bass": (36, 53, 36), "texture": (72, 88, 72)}
TONICS = ("C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B")
# Fixed seeds outside every training, validation and test range. With a songbook or
# console style, a brain's opening is ranked against its openings for these seeds.
REFERENCE_SEEDS = tuple(range(2_000_000_000, 2_000_000_024))
_REFERENCE_OPENINGS = {}


def _select(value, choices):
    index = min(len(choices) - 1, max(0, int((float(value) + 1.) * .5 * len(choices))))
    return choices[index]


def _opening_shares(circuit, brain, condition, first):
    """Where each opening output falls among the same brain's openings for the reference seeds, in 0..1.

    The opening outputs choose key, mode, tempo and voices, and they saturate: untrained
    and drilled brains alike put 13 or 14 of 16 seeds in A minor. Ranking an opening
    against the brain's own range of openings spreads those choices across seeds.
    """
    key = (brain.sha256(), condition, id(circuit))
    reference = _REFERENCE_OPENINGS.get(key)
    if reference is None:
        reference = np.array([np.asarray(BrainController(circuit, s, condition, brain).phrase(0, .45, 0), dtype=float)
                              for s in REFERENCE_SEEDS])
        if len(_REFERENCE_OPENINGS) >= 32:
            _REFERENCE_OPENINGS.clear()
        _REFERENCE_OPENINGS[key] = reference
    below = (reference < first).sum(axis=0) + .5 * (reference == first).sum(axis=0)
    return (below + .5) / (len(REFERENCE_SEEDS) + 1)


def write(circuit, brain, seed=2026, seconds=75, voices=None, condition="connectome", foundation=None, style=None, direction=None):
    """One performance from ``brain``.

    ``foundation`` is an optional songbook :class:`Foundation`. ``style`` is an
    optional :class:`ConsoleStyle`: arpeggios replace held chords, the bass
    pulses, the texture part becomes a kick/snare/hat groove, and registers,
    tempo range and default voices follow classic sound chips.
    """
    if type(seed) is not int or not 0 <= seed <= 2**31 - 1:
        raise ValueError("Seed must be an integer in 0..2147483647")
    if isinstance(seconds, bool) or not isinstance(seconds, (int, float)) or not math.isfinite(seconds) or not 10 <= seconds <= 180:
        raise ValueError("Capture duration must be finite and between 10 and 180 seconds")
    if voices is not None and (not isinstance(voices, dict) or set(voices) != set(ROLES)
                               or any(not isinstance(v, str) or not v for v in voices.values())):
        raise ValueError("Provide five named instrumental voices")
    seconds = float(seconds)
    controller = BrainController(circuit, seed, condition, brain)
    feedback_energy, harmonic_root = .45, 0
    first = np.asarray(controller.phrase(0, feedback_energy, harmonic_root), dtype=float)
    initial = np.tanh(2. * first)
    calibrated = foundation is not None or style is not None
    opening_shares = _opening_shares(circuit, brain, condition, first) if calibrated else None
    if calibrated:
        initial = 2. * opening_shares - 1.
    bpm = style.tempo(initial[7]) if style else int(round(96 + 18 * float(initial[7])))
    beat_seconds = 60. / bpm
    # A style may narrow the menus the opening outputs choose from. The brain
    # still makes the choice; the style only says which choices exist, which is
    # how one trained brain can play several distinct styles.
    traits = style.traits if style else {}
    tonic = int(_select(initial[0], traits.get("tonics", (0, 2, 5, 7, 9))))
    mode = str(_select(initial[1], traits.get("modes", ("minor", "dorian", "major"))))
    scale = SCALES[mode]
    degree_by_pitch = {midi(degree, tonic, scale, 60): degree for degree in range(-35, 42)}
    if voices is not None:
        voice_map = dict(voices)
    elif style:
        voice_map = dict(style.palette)
    else:
        voice_map = {role: _select(np.tanh(initial[i] + .5 * initial[(i + 3) % 8]), VOICE_POOLS[role])
                     for i, role in enumerate(ROLES)}
    registers = style.registers if style else REGISTERS
    if traits.get("registers"):
        registers = {**registers, **{role: tuple(span) for role, span in traits["registers"].items()}}
    drum_pitch = ({name: min(degree_by_pitch, key=lambda p, target=target: abs(p - target)) for name, target in DRUM_PITCH.items()}
                  if style else {})
    stop_beat = (seconds - 1.) / beat_seconds
    events, phrases, decisions = [], [], []
    memory = foundation.opening_memory(seed, mode) if foundation else []
    choices = foundation.session() if foundation else None
    opening_motifs = [cell["source"] for cell in memory]
    last_pitch, last_degree, last_end = {}, {}, {}
    idle = {role: 0 for role in ROLES}
    recent_roots, span_history = [], []
    previous_voicing = [midi(d, tonic, scale, 48) for d in (0, 2, 4)]
    foreground_pitch = midi(0, tonic, scale, 60)
    foreground_history = []
    position, index, scalar_redirections = 0., 0, 0
    capture_exhausted = False

    def note(role, beat, degree, duration, velocity, root, decision, primary=False, direct_pitch=None):
        nonlocal foreground_pitch, scalar_redirections
        swing = float(traits.get("swing", 0.))
        if swing:
            # Late off-beats are the whole difference between a lofi groove and a
            # march. Eighths lean by the swing amount, sixteenths by half of it.
            fraction = float(beat) % 1.
            if abs(fraction - .5) < .02:
                beat = float(beat) + .5 * swing
            elif abs(fraction - .25) < .02 or abs(fraction - .75) < .02:
                beat = float(beat) + .25 * swing
        if role in ("lead", "answer") and traits.get("lead_gate"):
            # Legato or clipped melody changes the character before a single pitch does.
            duration = max(.12, float(duration) * float(traits["lead_gate"]))
        low, high, base = registers[role]
        raw_pitch = midi(degree, tonic, scale, base)
        candidates = [p for p in range(low, high + 1) if p % 12 == raw_pitch % 12]
        target = foreground_pitch if primary else last_pitch.get(role, raw_pitch)
        if primary and foundation:
            # Songbook intervals keep their size and direction, octave leaps included; the register still folds them.
            target += midi(degree, tonic, scale, 60) - midi(degree_by_pitch[foreground_pitch], tonic, scale, 60)
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
        selected = voiced_chord(root, drive)
        # A wide roll is a strum; a narrow one is a keyboard chord.
        spread = _select(drive[6], traits.get("roll", (0., .06, .18)))
        held = span * (traits.get("hold_share", .65) + .25 * (drive[5] + 1.) / 2)
        for slot, (degree, pitch) in enumerate(selected):
            # A wide strum over a short chord would otherwise give the top note a
            # negative length; a strummed note still has to ring.
            note("harmony", begin + .10 + spread * slot, degree, max(.25, held - spread * slot),
                 .37 + .10 * feedback_energy, root, decision, direct_pitch=pitch)

    def voiced_chord(root, drive):
        nonlocal previous_voicing
        options = []
        # Sevenths are what give lofi harmony its colour; a triad is a folk chord.
        chord = [root, root + 2, root + 4] + ([root + 6] if traits.get("sevenths") else [])
        ceiling = 74 if traits.get("sevenths") else 69
        for inversion in range(len(chord)):
            ordered = chord[inversion:] + [d + 7 for d in chord[:inversion]]
            for shift in (-7, 0, 7):
                degrees = [d + shift for d in ordered]
                pitches = [midi(d, tonic, scale, 48) for d in degrees]
                if min(pitches) >= 48 and max(pitches) <= ceiling:
                    cost = sum(abs(a - b) for a, b in zip(previous_voicing, pitches))
                    options.append((cost, degrees, pitches))
        _, degrees, previous_voicing = min(options, key=lambda option: option[0])
        selected = list(zip(degrees, previous_voicing))
        if drive[5] < .2:
            selected = [(d, p) for d, p in selected if (d - root) % 7 != 0]
        return selected

    def play_stabs(begin, root, span, drive, decision):
        # Comping: short chord hits that leave the beat to the drums.
        chord = voiced_chord(root, drive)
        pattern = _select(drive[6], ((.5, 1.5, 2.5, 3.5), (.75, 2.5), (0., 1.5, 3.)))
        for bar in range(int(math.ceil(span / 4.))):
            for offset in pattern:
                when = bar * 4. + offset
                if when < span - .1:
                    for degree, pitch in chord:
                        note("harmony", begin + when, degree, .35, .55 + .08 * feedback_energy, root, decision,
                             direct_pitch=pitch)

    def play_broken(begin, root, span, drive, decision):
        # A chord laid out one note per beat, each left ringing to the end of the
        # gesture, so it builds layers instead of replacing them.
        chord = voiced_chord(root, drive)
        step = float(_select(drive[5], (1., 1., .5)))
        for slot in range(int(span / step)):
            degree, pitch = chord[slot % len(chord)]
            when = slot * step
            note("harmony", begin + when, degree, max(.4, span - when - .1), .40 + .08 * feedback_energy, root,
                 decision, direct_pitch=pitch)

    def play_chord_pulse(begin, root, span, drive, decision):
        # The same chord struck on every beat, held just short of the next.
        chord = voiced_chord(root, drive)
        for beat_index in range(int(span)):
            for degree, pitch in chord:
                note("harmony", begin + beat_index, degree, .8, .45 + .08 * feedback_energy, root, decision,
                     direct_pitch=pitch)

    def play_arpeggio(begin, root, span, drive, decision):
        rate = style.arpeggio_rate(drive[5])
        low, high, base = registers["harmony"]
        pitches = sorted({midi(root + d, tonic, scale, base) for d in (0, 2, 4, 7)})
        pitches = [p for p in pitches if low <= p <= high] or pitches
        shape = _select(drive[6], ("down", "updown", "up"))
        cycle = pitches if shape == "up" else pitches[::-1] if shape == "down" else pitches + pitches[-2:0:-1]
        for k in range(int(span * rate)):
            # The renderer mixes the harmony part far below the lead, as it was set for held chords;
            # a chip arpeggio channel plays near full volume, so its notes carry a high velocity.
            note("harmony", begin + k / rate, root, .9 / rate, .78 + .08 * feedback_energy, root, decision,
                 direct_pitch=cycle[k % len(cycle)])

    def bass_pitch(degree, octave=0):
        low, high, base = registers["bass"]
        pitch = midi(degree, tonic, scale, base)
        while pitch > high - (12 if octave else 0):
            pitch -= 12
        while pitch < low:
            pitch += 12
        return pitch + 12 * octave

    def play_bass_pattern(kind, begin, root, span, drive, decision):
        energy = .50 + .10 * feedback_energy
        if kind == "drone":
            # One root held under the whole gesture.
            note("bass", begin, root, span - .1, energy, root, decision, direct_pitch=bass_pitch(root))
        elif kind == "octave":
            # Root and octave bouncing in eighths: the disco and adventure bass.
            for k in range(int(span * 2)):
                note("bass", begin + k * .5, root, .40, energy, root, decision,
                     direct_pitch=bass_pitch(root, octave=k % 2))
        elif kind == "syncopated":
            # The downbeat, then pushes ahead of beats two and three.
            for bar in range(int(math.ceil(span / 4.))):
                for offset, length, degree in ((0., 1.2, root), (1.75, .6, root), (2.5, .9, root + 4)):
                    when = bar * 4. + offset
                    if when < span - .1:
                        note("bass", begin + when, degree, length, energy, root, decision,
                             direct_pitch=bass_pitch(degree))
        elif kind == "walking":
            # Quarter notes stepping through the chord toward the next root.
            line = (root, root + 2, root + 4, root + 5)
            for k in range(int(span)):
                degree = line[k % len(line)]
                note("bass", begin + k, degree, .9, energy, root, decision, direct_pitch=bass_pitch(degree))

    def play_pulse_bass(begin, root, span, drive, decision):
        for k in range(max(1, int(span * 2))):
            degree = root + 4 if (k % 4 == 3 and drive[3] > .25) else root
            note("bass", begin + k * .5, degree, .42, .50 + .10 * feedback_energy, root, decision)

    def play_drums(begin, root, span, drive, decision):
        grid = style.bar_grid(drive)
        for k in range(int(span * 4)):
            beat = begin + k * .25
            for name in grid.get(int(round(beat * 4)) % 16, ()):
                note("texture", beat, root, DRUM_LENGTH[name], DRUM_VELOCITY[name] + .05 * feedback_energy, root, decision,
                     direct_pitch=drum_pitch[name])

    while not (position >= stop_beat - 1. or capture_exhausted):
        # Optional, explicit direction for a commissioned longer arrangement.
        # The ordinary performer and all existing training runs remain unchanged.
        controller_index, directed_energy = index, feedback_energy
        if direction is not None:
            controller_index, directed_energy = direction(index, position * beat_seconds, feedback_energy)
            if not math.isfinite(controller_index) or not math.isfinite(directed_energy):
                raise ValueError("Performance direction must return finite inputs")
        control = first if index == 0 else np.asarray(controller.phrase(controller_index, directed_energy, harmonic_root), dtype=float)
        drive = np.tanh(2. * control)
        prior_feedback, prior_root = feedback_energy, harmonic_root
        root_scores = [1.5 * (drive[0] * math.cos(root * 2 * math.pi / 7)
                              + drive[1] * math.sin(root * 2 * math.pi / 7))
                       - .24 * recent_roots[-4:].count(root)
                       + (.18 if root == harmonic_root else 0.) for root in range(7)]
        if foundation:
            root_scores = [score + foundation.root_bias(harmonic_root, root, mode) for root, score in enumerate(root_scores)]
        harmonic_root = int(np.argmax(root_scores))
        recent_roots.append(harmonic_root)
        # Shorter gestures mean more phrases and faster harmony per minute. The
        # style sets the menu; the drive still picks from it.
        span_options = traits.get("spans", (2., 3., 4., 5., 6., 8.))
        desired_span = 2. + 6. * (drive[2] + 1.) / 2
        span_scores = [-abs(span - desired_span) - .38 * span_history[-3:].count(span) for span in span_options]
        chosen_span = span_options[int(np.argmax(span_scores))]
        span = min(chosen_span, stop_beat - position - .12)
        span_history.append(chosen_span)
        if span < .9:
            capture_exhausted = True
            continue
        feedback_energy = float(.60 * feedback_energy + .40 * np.clip(.20 + np.mean(np.abs(control)) * 1.3, .15, .9))
        # How readily every part joins in. Measured across five styles, rest share
        # sat at 0.40 whatever the style did to tempo, voices or drums, because
        # which parts play was fixed here; a style that wants a fuller or sparser
        # texture has to be able to lean on it. The drive still decides.
        presence = traits.get("presence", 0.)
        role_scores = {
            "lead": .28 + presence + .75 * drive[4] + .16 * idle["lead"],
            "answer": .12 + presence - .75 * drive[4] + .19 * idle["answer"],
            "harmony": .20 + presence + .75 * drive[5] + .16 * idle["harmony"],
            "bass": .33 + presence + .72 * drive[6] + .14 * idle["bass"],
            "texture": -.20 + presence + .90 * drive[7] + .19 * idle["texture"],
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
        # Melody rhythm grammar. Across sixty drawn grammars the accompaniment
        # varied widely but the melody's rhythm barely moved (0.20 of an audible
        # unit), because every melody drew its lengths and rests from the same
        # songbook and started from the same three positions. The ear follows the
        # melody first, so a tune may now run it in double time, stretch it into
        # long notes, lock it to a triplet grid, push it off the beat, or build it
        # from a repeated rhythmic cell.
        melody_time = float(traits.get("melody_time", 1.))
        melody_grid = traits.get("melody_grid")
        riff = int(traits.get("riff", 0))
        riff_steps, riff_rests = [], []
        cursor = float(_select(drive[0] - drive[1] * .15, traits.get("melody_offsets", (0., .25, .50))))
        gesture, articulation, actual_primary = [], [], []
        prior_degree = last_degree.get(primary, harmonic_root)
        slot = 0
        # Chip leads run in 16ths; twelve notes per gesture left them sparse at console tempos.
        while cursor < primary_end - .22 and slot < traits.get("melody_slots", 24 if style else 12):
            channel = (slot + 1) % 8
            pulse_drive = float(np.tanh(drive[channel] + .35 * drive[(channel + 3) % 8] * (-1 if slot % 2 else 1)))
            interval = choices.interval(pulse_drive) if foundation else _select(pulse_drive, (-4, -2, -1, 1, 2, 4))
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
            if slot == 0:
                degree = nearest_chord(degree, harmonic_root)
            if foundation:
                # Lengths read other channels than intervals, so long notes are not tied to leaps.
                step_drive = np.tanh(drive[(channel + 4) % 8] - .35 * drive[(channel + 6) % 8] * (-1 if slot % 2 else 1))
                step = float(choices.step(step_drive))
            else:
                step = float(_select(pulse_drive, (.5, .75, 1., 1.25, 1.5)))
            recalled = bool(remembered and operation == "recall")
            if recalled:
                step = remembered["steps"][slot % len(remembered["steps"])]
            else:
                # Recalled steps were measured from a performance already in this
                # time, so only fresh steps are scaled.
                step *= melody_time
                if riff:
                    if slot < riff:
                        riff_steps.append(step)
                    else:
                        step = riff_steps[slot % riff]
            if primary == "answer":
                # A console answer is a second pulse channel and may move as fast as the lead.
                step = max(.5 if style else .85, step)
            # Legato holds each note to nearly its full step; the default ratio is
            # computed exactly as before so an unstyled melody is unchanged.
            ratio = traits.get("legato") or (.65 + .20 * (drive[5] + 1.) / 2)
            length = min(step * ratio, primary_end - cursor)
            event = note(primary, position + cursor, degree, length,
                         .46 + .20 * feedback_energy + .05 * drive[(slot + 4) % 8],
                         harmonic_root, index, primary=True)
            if event:
                gesture.append(event["scale_degree"])
                actual_primary.append(event)
            prior_degree = event["scale_degree"] if event else degree
            if remembered and operation == "recall":
                rest = 0.
            else:
                rest_drive = drive[(slot + 6) % 8]
                rest = float(choices.rest(rest_drive) if foundation else _select(rest_drive, (0., .15, .30, .5)))
                rest *= melody_time
                if riff:
                    if slot < riff:
                        riff_rests.append(rest)
                    else:
                        rest = riff_rests[slot % riff]
            following = cursor + (step + rest)  # same association as the original, bit for bit
            if melody_grid:
                # Snap to the grid, but always move forward by at least one cell.
                snapped = round(following / melody_grid) * melody_grid
                following = snapped if snapped > cursor + 1e-6 else cursor + melody_grid
            cursor = following
            slot += 1
        if not actual_primary:
            event = note(primary, position + .08, nearest_chord(prior_degree, harmonic_root),
                         max(.25, span * .45), .47, harmonic_root, index, primary=True)
            if event:
                actual_primary.append(event)
                gesture = [event["scale_degree"]]
        if actual_primary:
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
            harmony_kind = style.traits.get("harmony", "arpeggio") if style else "hold"
            if style and harmony_kind == "arpeggio":
                play_arpeggio(position, harmonic_root, span, drive, index)
            elif harmony_kind == "stabs":
                play_stabs(position, harmonic_root, span, drive, index)
            elif harmony_kind == "broken":
                play_broken(position, harmonic_root, span, drive, index)
            elif harmony_kind == "pulse":
                play_chord_pulse(position, harmonic_root, span, drive, index)
            else:
                hold_harmony(position, harmonic_root, span, drive, index)
        bass_kind = traits.get("bass", "pulse") if style else None
        if "bass" in active and style and bass_kind == "pulse":
            play_pulse_bass(position, harmonic_root, span, drive, index)
        elif "bass" in active and style and bass_kind in ("drone", "octave", "syncopated", "walking"):
            play_bass_pattern(bass_kind, position, harmonic_root, span, drive, index)
        elif "bass" in active:
            note("bass", position, harmonic_root, min(2.1, span * .44),
                 .51 + .11 * feedback_energy, harmonic_root, index)
            if span > 3 and drive[6] > -.20:
                onset = span * float(_select(drive[2], (.45, .55, .67)))
                note("bass", position + onset, harmonic_root + int(_select(drive[3], (0, 4, 7))),
                     min(1.1, span - onset - .2), .43, harmonic_root, index)
        if "texture" in active and style and traits.get("drums", True):
            play_drums(position, harmonic_root, span, drive, index)
        elif "texture" in active:
            count = int(_select(drive[7], (1, 1, 2, 3)))
            for texture_slot in range(count):
                onset = (texture_slot + .65) * span / (count + .5)
                note("texture", position + onset, harmonic_root + int(_select(drive[(texture_slot + 4) % 8], (2, 4, 7))),
                     min(.8, span - onset - .13), .29 + .06 * feedback_energy, harmonic_root, index)
        written = [e for e in events if e["decision"] == index]
        sounded = {e["layer"] for e in written}
        for role in ROLES:
            idle[role] = 0 if role in sounded else idle[role] + 1
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
        if foundation:
            decision["memory_source"] = remembered.get("source") if remembered else None
        if direction is not None:
            decision["direction"] = {"controller_index": controller_index, "feedback_energy": directed_energy}
        if len(controller.trace) - 1 != index:
            decision["controller_trace_index"] = len(controller.trace) - 1
        decisions.append(decision)
        position += span
        index += 1

    events.sort(key=lambda e: (e["start_seconds"], ROLES.index(e["layer"]), e["midi"]))
    for number, event in enumerate(events):
        event["id"] = number
    score = {"version": 1, "kind": "performance", "performer": "buzzart",
             "arrangement_version": ENGINE, "title": f"Buzzart performance {seed}",
             "seed": seed, "condition": condition, "duration_seconds": seconds, "bpm": bpm,
             "tonic": TONICS[tonic], "mode": mode, "meter": "4/4", "beats_per_bar": 4,
             "bars": int(math.ceil(stop_beat / 4)), "voices": voice_map, "events": events,
             "phrases": phrases, "progression": recent_roots,
             "sections": [{"name": "Continuous capture", "start_seconds": 0., "end_seconds": seconds - 1.,
                           "scope": "Audio measurement window; not a planned musical section"}],
             "event_sha256": hashlib.sha256(json.dumps(events, sort_keys=True).encode()).hexdigest(),
             "controller": controller.identity, "neural_trace": controller.trace,
             "decision_log": decisions, "musical_memory": memory, "circuit_metadata": circuit.get("metadata", {}),
             "buzzart": {"brain": brain.identity(), "interpreter": ENGINE,
                         "foundation": foundation.identity() if foundation else None,
                         "style": style.identity() if style else None,
                         "opening_motifs": opening_motifs,
                         "opening_shares": opening_shares.tolist() if calibrated else None},
             "decoder_provenance": {
                 "engine": "Buzzart brain over the fixed connectome, read by Buzzart's copy of the Fly note interpreter",
                 "planned_form": None, "planned_progression": None, "prewritten_melody": None,
                 "voice_selection": "explicit override" if voices is not None else "initial neural readout selects from role-compatible pools",
                 "voice_pools": {role: list(pool) for role, pool in VOICE_POOLS.items()},
                 "initial_outputs": first.tolist(), "readout_gain": 2.,
                 "opening_calibration": (f"Key, mode, tempo and voices read each opening output's rank among this brain's openings "
                                         f"for {len(REFERENCE_SEEDS)} fixed reference seeds") if calibrated else None,
                 "foundation": ("Intervals, lengths and rests keep songbook proportions, the circuit's drive ranking where each note falls; chord roots "
                                "lean toward songbook transitions; memory opens with songbook motifs") if foundation else None,
                 "scalar_run_redirections": scalar_redirections,
                 "limitations": ("Projections, bias and dynamics scalars were optimized against designed and corpus-derived "
                                 "measurements, which are not listener approval. The note interpreter is engineered.")}}
    if traits.get("mix"):
        # Which part is out in front is part of the tune, not of the renderer.
        score["mix"] = {role: float(level) for role, level in traits["mix"].items()}
    return score
