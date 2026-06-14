import numpy as np
from teledra_synth import synth_note, mix_waves, fit_to_length, lowpass_filter, reverb, delay, play_sound

SR = 44100
BEAT = 0.45
chords = [["D4","F4","A4"],["A3","C4","E4"],["B3","D4","F4"],["G3","B3","D4"]]
bass_notes = ["D2","A1","B1","G1"]
lead_motif = ["A5","F5","E5","D5","A4","D5","F5","A5"]
bar_seconds = BEAT * 4
bar_len = int(bar_seconds * SR)
full_track = np.zeros(bar_len * len(chords))
for i, chord in enumerate(chords):
    bar_start = i * bar_seconds
    for note in chord:
        pad = synth_note(note, bar_seconds, wave_type="triangle", attack=0.4, release=0.6, volume=0.16)
        full_track = mix_waves(full_track, pad, start_time=bar_start)
    for beat in range(4):
        bass = synth_note(bass_notes[i], BEAT * 0.9, wave_type="sawtooth", attack=0.01, release=0.1, volume=0.22)
        full_track = mix_waves(full_track, bass, start_time=bar_start + beat * BEAT)
for j, note in enumerate(lead_motif * len(chords)):
    t = j * BEAT
    if t * SR >= len(full_track):
        break
    voice = synth_note(note, BEAT * 0.8, wave_type="triangle", attack=0.02, release=0.15, volume=0.12)
    voice = delay(voice, delay_time=BEAT / 2, feedback=0.35, mix=0.3)
    full_track = mix_waves(full_track, voice, start_time=t)
full_track = lowpass_filter(full_track, cutoff=2600)
full_track = reverb(full_track, room_size=0.6, mix=0.25)
full_track = fit_to_length(full_track, len(full_track))
play_sound(full_track, loop=True)
