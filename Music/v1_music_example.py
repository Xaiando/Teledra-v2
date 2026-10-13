import numpy as np
from teledra_synth import (
    apply_automation, automation_curve, delay, lowpass_filter, make_seamless_loop,
    mix_waves, play_sound, reverb, soft_limiter, stereo_pan, stereo_width, synth_note,
)

SR = 44100
SEED = 4187
np.random.seed(SEED)
TITLE = "Fivefold Court Engine"
STYLE = "retro adventure court score"
BEAT = 0.55
BPM = 60.0 / BEAT
BEATS_PER_BAR = 4
chords = [["A3","C4","E4"],["D4","F4","A4"],["E4","G#4","B4"],["A3","C4","E4"]]
bass_notes = ["A1","D2","E2","A1"]
lead_motif = ["E5","A5","C6","B5","G#5","B5","A5","E5"]
KEY = "A harmonic minor"
SECTION_NAMES = ["arrival", "statement", "development", "apex", "return"]
SECTION_BARS = 8
BARS = SECTION_BARS * len(SECTION_NAMES)
BAR_SECONDS = BEAT * 4
SECTION_SECONDS = SECTION_BARS * BAR_SECONDS
TOTAL_SECONDS = BARS * BAR_SECONDS
SAMPLES = int(TOTAL_SECONDS * SR)
MASTER_GAIN = 6.5

TELEDRA_SCORE = {
    "title": TITLE,
    "key": KEY,
    "bpm": BPM,
    "bars": BARS,
    "motif": "an eight-note rising question transformed by fragmentation, reversal, register, and rhythmic displacement",
    "sections": SECTION_NAMES,
    "depth_roles": {
        "foreground": ["lead"],
        "midground": ["harmony", "counterline", "percussion"],
        "background": ["bass", "texture"],
    },
}
TELEDRA_AUTOMATION = {
    "energy": [0.34, 0.56, 0.74, 0.96, 0.48],
    "pad_cutoff_hz": [850, 1250, 1750, 2600, 1100],
    "stereo_width": [0.72, 0.9, 1.08, 1.22, 0.82],
    "master_gain": MASTER_GAIN,
}
TELEDRA_COMPOSER = {
    "seed": SEED,
    "style_profile": "retro_adventure",
    "tonal_center": "A",
    "mode": "harmonic_minor",
    "progression_degrees": [1, 4, 5, 1],
    "chord_voicings": chords,
    "motif_notes": lead_motif,
    "phrase_bars": 8,
    "transformations": ["fragmentation", "call_and_response", "rhythmic_displacement", "register_return"],
    "swing": 0.06,
    "registers": {"bass": [1, 2], "harmony": [3, 4], "lead": [4, 6]},
    "section_density": TELEDRA_AUTOMATION["energy"],
    "intentional_tensions": [],
    "tension_policy": "The raised seventh resolves to A; other color tones return by step before the loop cadence.",
}

layers = {
    "bass": np.zeros(SAMPLES),
    "harmony": np.zeros(SAMPLES),
    "counterline": np.zeros(SAMPLES),
    "lead": np.zeros(SAMPLES),
    "kick": np.zeros(SAMPLES),
    "percussion": np.zeros(SAMPLES),
    "texture": np.zeros(SAMPLES),
    "transitions": np.zeros(SAMPLES),
}
TELEDRA_EVENTS = []

def record_event(kind, track, role, start_time, duration_seconds, velocity, pitch=None, motif="", transform=""):
    start_time = max(0.0, float(start_time))
    end_time = min(TOTAL_SECONDS, start_time + max(0.001, float(duration_seconds)))
    start_beat = start_time / BEAT
    section_idx = min(int(start_time / SECTION_SECONDS), len(SECTION_NAMES) - 1)
    event = {
        "kind": kind,
        "track": track,
        "role": role,
        "start_beat": start_beat,
        "duration_beats": max(0.001, (end_time - start_time) / BEAT),
        "velocity": float(np.clip(velocity, 0.001, 1.0)),
        "section": SECTION_NAMES[section_idx],
        "motif": motif,
        "transform": transform,
    }
    if pitch is not None:
        event["pitch"] = pitch
    TELEDRA_EVENTS.append(event)

def place(layer_name, wave, start_time, level=1.0):
    start = max(0, int(start_time * SR))
    end = min(SAMPLES, start + len(wave))
    if end > start:
        layers[layer_name][start:end] += wave[:end - start] * level

section_energy = TELEDRA_AUTOMATION["energy"]
counter_motif = list(reversed(lead_motif))
motif_forms = [
    lead_motif[:4],
    lead_motif,
    lead_motif[2:] + lead_motif[:2],
    counter_motif + lead_motif[:4],
    [lead_motif[0], lead_motif[2], lead_motif[4], lead_motif[1]],
]
motif_event_transforms = [
    "fragmentation", "prime", "rhythmic_displacement", "call_and_response", "register_return",
]

for section_idx, section_name in enumerate(SECTION_NAMES):
    section_start = section_idx * SECTION_SECONDS
    energy = section_energy[section_idx]
    for bar in range(SECTION_BARS):
        bar_start = section_start + bar * BAR_SECONDS
        chord = chords[(bar + section_idx) % len(chords)]
        pad_wave = "triangle" if section_idx in (0, 4) else "sawtooth"
        for chord_note in chord:
            pad = synth_note(
                chord_note, BAR_SECONDS * 0.96, wave_type=pad_wave,
                attack=0.22 + section_idx * 0.05, decay=0.12, sustain=0.62,
                release=0.55, volume=0.055 * energy,
            )
            pad = lowpass_filter(pad, cutoff=TELEDRA_AUTOMATION["pad_cutoff_hz"][section_idx])
            place("harmony", pad, bar_start)
            record_event("note", "harmony", "harmony", bar_start, BAR_SECONDS * 0.96, energy, pitch=chord_note)

        for beat_idx in range(4):
            if section_idx == 0 and beat_idx in (1, 3):
                continue
            bass_note = bass_notes[(bar + section_idx) % len(bass_notes)]
            bass = synth_note(
                bass_note, BEAT * 0.82, wave_type="sawtooth",
                attack=0.008, decay=0.05, sustain=0.52, release=0.12,
                volume=0.09 + 0.055 * energy,
            )
            bass_start = bar_start + beat_idx * BEAT
            place("bass", lowpass_filter(bass, cutoff=680 + section_idx * 120), bass_start)
            record_event("note", "bass", "bass", bass_start, BEAT * 0.82, 0.5 + 0.4 * energy, pitch=bass_note)

        if section_idx > 0:
            for beat_idx in range(4):
                kick = synth_note(
                    bass_notes[0], BEAT * 0.42, wave_type="sine",
                    attack=0.002, decay=0.045, sustain=0.0, release=0.11,
                    volume=0.16 + 0.08 * energy,
                )
                if beat_idx == 0 or (section_idx >= 2 and beat_idx == 2):
                    kick_start = bar_start + beat_idx * BEAT
                    place("kick", kick, kick_start)
                    record_event("drum", "kick", "percussion", kick_start, BEAT * 0.09, 0.62 + 0.3 * energy)
                if beat_idx in (1, 3):
                    snare = synth_note(
                        "D3", BEAT * 0.24, wave_type="white_noise",
                        attack=0.002, decay=0.025, sustain=0.0, release=0.07,
                        volume=0.045 + 0.035 * energy,
                    )
                    snare_start = bar_start + beat_idx * BEAT
                    place("percussion", snare, snare_start)
                    record_event("drum", "percussion", "percussion", snare_start, BEAT * 0.055, 0.44 + 0.3 * energy)
                if section_idx >= 2 or beat_idx % 2 == 0:
                    hat = synth_note(
                        "C6", BEAT * 0.1, wave_type="white_noise",
                        attack=0.001, decay=0.01, sustain=0.0, release=0.025,
                        volume=0.012 + 0.016 * energy,
                    )
                    hat_start = bar_start + (beat_idx + 0.5) * BEAT
                    place("percussion", hat, hat_start)
                    record_event("drum", "percussion", "percussion", hat_start, BEAT * 0.03, 0.28 + 0.24 * energy)

    motif = motif_forms[section_idx]
    step = BEAT if section_idx == 0 else BEAT * (0.5 if section_idx in (2, 3) else 0.75)
    phrase_start = section_start + (BAR_SECONDS * (2 if section_idx == 0 else 1))
    phrase_end = section_start + SECTION_SECONDS
    cursor = phrase_start
    note_idx = 0
    while cursor < phrase_end:
        phrase_slot = note_idx % (len(motif) + 2)
        note = motif[phrase_slot % len(motif)]
        phrase_breath = phrase_slot >= len(motif)
        if not phrase_breath and not (section_idx == 0 and note_idx % 3 == 1):
            lead = synth_note(
                note, step * 0.82, wave_type="sawtooth",
                attack=0.018, decay=0.055, sustain=0.66, release=0.14,
                volume=0.035 + 0.035 * energy,
            )
            if section_idx >= 2:
                lead = delay(lead, delay_time=BEAT * 0.5, feedback=0.24, mix=0.18)
            place("lead", lead, cursor)
            record_event(
                "note", "lead", "lead", cursor, step * 0.82, 0.58 + 0.32 * energy,
                pitch=note, motif="fivefold_call", transform=motif_event_transforms[section_idx],
            )
        if section_idx in (2, 4) and note_idx % 8 == 2:
            answer_note = counter_motif[note_idx % len(counter_motif)]
            answer = synth_note(
                answer_note, step * 1.4, wave_type="triangle",
                attack=0.04, decay=0.08, sustain=0.55, release=0.28,
                volume=0.024 + 0.018 * energy,
            )
            answer_start = cursor + step * 0.5
            place("counterline", answer, answer_start)
            record_event(
                "note", "counterline", "motion", answer_start, step * 1.4, 0.36 + 0.28 * energy,
                pitch=answer_note, motif="fivefold_call", transform="call_and_response",
            )
        cursor += step
        note_idx += 1

    texture = synth_note(
        bass_notes[section_idx % len(bass_notes)], SECTION_SECONDS,
        wave_type="pink_noise", attack=1.2, decay=0.2, sustain=0.42,
        release=1.4, volume=0.012 + 0.009 * energy,
    )
    place("texture", lowpass_filter(texture, cutoff=420 + section_idx * 110), section_start)
    record_event("fx", "texture", "texture", section_start, SECTION_SECONDS, 0.12 + 0.24 * energy)
    if section_idx > 0:
        transition = synth_note(
            "C5", BEAT * 1.8, wave_type="white_noise",
            attack=0.01, decay=0.08, sustain=0.25, release=0.7,
            volume=0.025 + 0.012 * energy,
        )
        transition *= np.linspace(0.0, 1.0, len(transition))
        transition_start = section_start - BEAT * 1.5
        place("transitions", transition, transition_start)
        record_event("fx", "transitions", "texture", transition_start, BEAT * 1.8, 0.25 + 0.28 * energy)

layers["lead"] = delay(layers["lead"], delay_time=BEAT * 0.75, feedback=0.2, mix=0.14)
layers["counterline"] = reverb(layers["counterline"], room_size=0.58, mix=0.2)
layers["texture"] = reverb(layers["texture"], room_size=0.82, mix=0.34)
layers["transitions"] = reverb(layers["transitions"], room_size=0.9, mix=0.38)

pan = {
    "bass": 0.0, "harmony": -0.16, "counterline": 0.34, "lead": -0.28,
    "kick": 0.0, "percussion": 0.22, "texture": 0.46, "transitions": -0.52,
}
mix_level = {
    "bass": 0.72, "harmony": 0.7, "counterline": 0.64, "lead": 0.78,
    "kick": 0.78, "percussion": 0.62, "texture": 0.44, "transitions": 0.52,
}
full_track = np.zeros(SAMPLES)
for layer_name, layer in layers.items():
    full_track = mix_waves(full_track, stereo_pan(layer, pan[layer_name]), volume_b=mix_level[layer_name])

energy_points = []
for section_idx, energy in enumerate(section_energy):
    energy_points.append((section_idx * SECTION_SECONDS, 0.58 + energy * 0.32))
energy_points.append((TOTAL_SECONDS, 0.7))
full_track = apply_automation(full_track, automation_curve(TOTAL_SECONDS, energy_points, sr=SR))
full_track = lowpass_filter(full_track, cutoff=2900)
full_track = reverb(full_track, room_size=0.58, mix=0.16)
full_track = stereo_width(full_track, width=1.08)
full_track *= MASTER_GAIN
full_track = soft_limiter(full_track, drive=1.2, ceiling=0.90)
full_track = make_seamless_loop(full_track, crossfade_seconds=0.1, sr=SR)

TELEDRA_LAYERS = {name: layer for name, layer in layers.items()}
TELEDRA_SECTIONS = {}
for section_idx, section_name in enumerate(SECTION_NAMES):
    start = int(section_idx * SECTION_SECONDS * SR)
    end = int((section_idx + 1) * SECTION_SECONDS * SR)
    TELEDRA_SECTIONS[section_name] = full_track[start:end]

play_sound(full_track, loop=True)
