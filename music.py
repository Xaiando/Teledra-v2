import numpy as np
import time
from teledra_synth import *

STYLE = "generative gothic electronica"

variants = [
    {
        "tempo": 96,
        "bass": ["C2", "G2", "Eb2", "Bb2"],
        "chords": ["C3", "Eb3", "G3", "Bb3"],
        "lead": ["G4", "Bb4", "C5", "Eb5", "D5", "Bb4", "G4", "C5"],
        "pad_wave": "sawtooth",
        "lead_wave": "sine",
        "texture_note": "C2",
        "pad_cutoff": 900.0,
        "hat_cutoff": 6800.0,
        "final_cutoff": 1800.0,
        "room": 0.62,
    },
    {
        "tempo": 112,
        "bass": ["A1", "E2", "G2", "D2"],
        "chords": ["A3", "C4", "E4", "G4"],
        "lead": ["E5", "G5", "A5", "C6", "B5", "G5", "E5", "A5"],
        "pad_wave": "triangle",
        "lead_wave": "sawtooth",
        "texture_note": "A2",
        "pad_cutoff": 1200.0,
        "hat_cutoff": 7600.0,
        "final_cutoff": 2300.0,
        "room": 0.48,
    },
    {
        "tempo": 84,
        "bass": ["D2", "A2", "F2", "C3"],
        "chords": ["D3", "F3", "A3", "C4"],
        "lead": ["F4", "A4", "C5", "E5", "D5", "A4", "F4", "E4"],
        "pad_wave": "square",
        "lead_wave": "sine",
        "texture_note": "D2",
        "pad_cutoff": 720.0,
        "hat_cutoff": 5200.0,
        "final_cutoff": 1500.0,
        "room": 0.72,
    },
    {
        "tempo": 128,
        "bass": ["G1", "D2", "Bb2", "F2"],
        "chords": ["G3", "Bb3", "D4", "F4"],
        "lead": ["Bb4", "C5", "D5", "F5", "G5", "F5", "D5", "C5"],
        "pad_wave": "sawtooth",
        "lead_wave": "triangle",
        "texture_note": "G2",
        "pad_cutoff": 1350.0,
        "hat_cutoff": 8200.0,
        "final_cutoff": 2600.0,
        "room": 0.54,
    },
]

variant = variants[int(time.time()) % len(variants)]
tempo = variant["tempo"]
beat = 60.0 / tempo

def melodic_line(notes, dur, wave_type, volume):
    return np.concatenate([
        synth_note(note, dur, wave_type=wave_type, attack=0.04, decay=0.08, sustain=0.65, release=0.18, volume=volume)
        for note in notes
    ])

bass_notes = variant["bass"] * 10
chord_roots = variant["chords"] * 10
lead_notes = variant["lead"] * 8

bass = melodic_line(bass_notes, beat, "triangle", 0.10)
pad = melodic_line(chord_roots, beat * 2.0, variant["pad_wave"], 0.045)
lead = melodic_line(lead_notes, beat * 0.5, variant["lead_wave"], 0.065)

kick = np.concatenate([
    synth_note("C2", beat * 0.5, wave_type="sine", attack=0.002, decay=0.05, sustain=0.0, release=0.14, volume=0.34),
    np.zeros(int(beat * 1.5 * 44100)),
] * 8)
snare = np.concatenate([
    np.zeros(int(beat * 44100)),
    synth_note("D3", beat * 0.35, wave_type="white_noise", attack=0.002, decay=0.04, sustain=0.0, release=0.10, volume=0.10),
    np.zeros(int(beat * 0.65 * 44100)),
] * 8)
hat = np.concatenate([
    synth_note("C6", beat * 0.18, wave_type="white_noise", attack=0.001, decay=0.01, sustain=0.0, release=0.04, volume=0.035),
    np.zeros(int(beat * 0.32 * 44100)),
] * 32)

target = max(len(bass), len(pad), len(lead), len(kick), len(snare), len(hat))
bass = fit_to_length(bass, target, mode="loop")
pad = fit_to_length(lowpass_filter(pad, cutoff=variant["pad_cutoff"]), target, mode="loop")
lead = fit_to_length(delay(lead, delay_time=0.22, feedback=0.28, mix=0.25), target, mode="loop")
kick = fit_to_length(kick, target, mode="loop")
snare = fit_to_length(snare, target, mode="loop")
hat = fit_to_length(lowpass_filter(hat, cutoff=variant["hat_cutoff"]), target, mode="loop")

full_track = mix_waves(bass, pad, start_time=0.0, volume_b=0.75)
full_track = mix_waves(full_track, lead, start_time=0.0, volume_b=0.9)
full_track = mix_waves(full_track, kick, start_time=0.0, volume_b=0.75)
full_track = mix_waves(full_track, snare, start_time=0.0, volume_b=0.8)
full_track = mix_waves(full_track, hat, start_time=0.0, volume_b=0.65)
texture = lowpass_filter(synth_note(variant["texture_note"], beat * 4.0, wave_type="pink_noise", attack=0.5, decay=0.2, sustain=0.5, release=0.8, volume=0.035), cutoff=620.0)
texture = fit_to_length(granular_synthesis(texture, grain_size=0.08, overlap=0.45, jitter=0.015), len(full_track), mode="loop")
full_track = mix_waves(full_track, texture, start_time=0.0, volume_b=0.55)
full_track = reverb(lowpass_filter(full_track, cutoff=variant["final_cutoff"]), room_size=variant["room"], mix=0.22)

full_track = fit_to_length(full_track, int(180.0 * 44100), mode="loop")
full_track = make_seamless_loop(full_track, crossfade_seconds=0.08, sr=44100)
TELEDRA_LAYERS = {
    "bass": fit_to_length(bass, len(full_track), mode="loop"),
    "pad": fit_to_length(pad, len(full_track), mode="loop"),
    "lead": fit_to_length(lead, len(full_track), mode="loop"),
    "kick": fit_to_length(kick, len(full_track), mode="loop"),
    "snare": fit_to_length(snare, len(full_track), mode="loop"),
    "hat": fit_to_length(hat, len(full_track), mode="loop"),
    "texture": fit_to_length(texture, len(full_track), mode="loop"),
}
play_sound(full_track, loop=True)
