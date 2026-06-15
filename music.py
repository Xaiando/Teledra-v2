import numpy as np
from teledra_synth import synth_note, mix_waves, fit_to_length, lowpass_filter, reverb, delay, play_sound

SR = 44100
BEAT = 0.45
chords = [["D4","F4","A4"],["A3","C4","E4"],["B3","D4","F4"],["G3","B3","D4"]]
bass_notes = ["D2","A1","B1","G1"]
lead_motif = ["A5","F5","E5","D5","A4","D5","F5","A5"]
counter_motif = list(reversed(lead_motif))
sections = ["intro", "body", "mutation", "coda", "afterglow"]
bar_seconds = BEAT * 4
total_bars = len(chords) * len(sections)
total_seconds = total_bars * bar_seconds
full_track = np.zeros(int(total_seconds * SR))

for section_idx, section_name in enumerate(sections):
    section_start = section_idx * len(chords) * bar_seconds
    section_gain = [0.46, 0.64, 0.86, 0.96, 0.72][section_idx]
    pad_wave = "triangle" if section_idx in (0, 3, 4) else "triangle"
    for i, chord in enumerate(chords):
        bar_start = section_start + i * bar_seconds
        for note in chord:
            pad = synth_note(note, bar_seconds * 1.4, wave_type=pad_wave, attack=0.35, decay=0.12, sustain=0.62, release=0.7, volume=0.10 * section_gain)
            pad = lowpass_filter(pad, cutoff=900.0 + 450.0 * section_idx)
            full_track = mix_waves(full_track, pad, start_time=bar_start, volume_b=0.95)
        for beat_idx in range(4):
            bass_note = bass_notes[(i + section_idx) % len(bass_notes)]
            bass = synth_note(bass_note, BEAT * 0.9, wave_type="sawtooth", attack=0.01, decay=0.05, sustain=0.55, release=0.14, volume=0.18 + 0.03 * section_idx)
            if section_idx >= 2 and beat_idx % 2 == 1:
                bass = delay(bass, delay_time=BEAT * 0.25, feedback=0.18, mix=0.18)
            full_track = mix_waves(full_track, bass, start_time=bar_start + beat_idx * BEAT)

for section_idx, motif in enumerate([lead_motif[:4], lead_motif, counter_motif, lead_motif[2:] + counter_motif[:4], counter_motif[2:] + lead_motif[:4]]):
    section_start = section_idx * len(chords) * bar_seconds
    section_end = section_start + len(chords) * bar_seconds
    step = BEAT if section_idx == 0 else BEAT * 0.5
    repeats = 2 if section_idx == 0 else 4
    for j, note in enumerate(motif * repeats):
        t = section_start + j * step
        if t >= section_end:
            break
        voice = synth_note(note, step * 0.88, wave_type="triangle", attack=0.02, decay=0.06, sustain=0.7, release=0.16, volume=0.055 + 0.025 * section_idx)
        voice = delay(voice, delay_time=BEAT / 2, feedback=0.28 + 0.04 * section_idx, mix=0.24)
        full_track = mix_waves(full_track, voice, start_time=t)

for step_idx in range(total_bars * 4):
    t = step_idx * BEAT
    if step_idx % 4 == 0:
        kick = synth_note("C2", BEAT * 0.45, wave_type="sine", attack=0.002, decay=0.05, sustain=0.0, release=0.12, volume=0.26)
        full_track = mix_waves(full_track, kick, start_time=t, volume_b=0.75)
    if step_idx % 4 == 2:
        snare = synth_note("D3", BEAT * 0.28, wave_type="white_noise", attack=0.002, decay=0.03, sustain=0.0, release=0.08, volume=0.075)
        full_track = mix_waves(full_track, snare, start_time=t, volume_b=0.7)
    hat = synth_note("C6", BEAT * 0.12, wave_type="white_noise", attack=0.001, decay=0.01, sustain=0.0, release=0.03, volume=0.025)
    full_track = mix_waves(full_track, hat, start_time=t + BEAT * 0.5, volume_b=0.55)

texture = synth_note(bass_notes[0], bar_seconds * 2.0, wave_type="pink_noise", attack=0.6, decay=0.2, sustain=0.5, release=0.9, volume=0.025)
texture = lowpass_filter(texture, cutoff=620.0)
texture = fit_to_length(texture, len(full_track), mode="loop")
full_track = mix_waves(full_track, texture, start_time=0.0, volume_b=0.45)
full_track = lowpass_filter(full_track, cutoff=2600)
full_track = reverb(full_track, room_size=0.68, mix=0.24)
play_sound(full_track, loop=True)
