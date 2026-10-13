"""A complete Python-authored study: free note placement, layers and return."""
import numpy as np
from teledra_synth import (
    instrument_note, synth_note, mix_waves, stereo_pan, stereo_width,
    automation_curve, apply_automation, delay, reverb, soft_limiter,
    make_seamless_loop, play_sound,
)

SR = 44100
BPM = 96
BEAT = 60 / BPM
BARS = 32
DURATION = BARS * 4 * BEAT
TITLE = "Lantern Rooms"
TELEDRA_SCORE = {
    "title": TITLE, "key": "D minor", "bpm": BPM, "bars": BARS,
    "sections": ["statement", "answer", "contrast", "return"],
}

# Every catalog instrument can be used through instrument_note. These voices
# have distinct jobs; the palette is available without a quota to use it all.
layers = {name: np.zeros(int(DURATION * SR))
          for name in ["bass", "harmony", "melody", "answer", "pulse"]}
chords = [["D3", "F3", "A3"], ["Bb2", "D3", "F3"],
          ["G2", "Bb2", "D3"], ["A2", "C#3", "E3"]]
roots = ["D2", "Bb1", "G1", "A1"]
question = [(0, "D4", .65), (1.5, "F4", .45), (2.5, "E4", .40),
            (4, "A4", .75), (5.5, "F4", .40), (6.5, "D4", .80)]
answer = [(0.5, "F4", .4), (1.25, "E4", .35), (3, "D4", .85),
          (4.5, "C#4", .40), (6, "D4", 1.1)]
contrast = [(0, "Bb4", .65), (2, "A4", .7), (3.5, "G4", .4),
            (5, "F4", .55), (6.5, "E4", .7)]

for bar in range(BARS):
    section = bar // 8
    harmony_index = (bar // 2) % 4
    start = bar * 4 * BEAT
    bass = instrument_note(roots[harmony_index], BEAT * 2.5, "double_bass", volume=.22, sr=SR)
    layers["bass"] = mix_waves(layers["bass"], bass, start_time=start, sr=SR)
    if bar % 2 == 0:
        for note in chords[harmony_index]:
            held = instrument_note(note, BEAT * 6.8, "warm_pad", volume=.055, sr=SR)
            layers["harmony"] = mix_waves(layers["harmony"], held, start_time=start, sr=SR)
    if bar % 2 == 0:
        phrase = question if section in (0, 3) else answer if section == 1 else contrast
        for position, note, length in phrase:
            wave = instrument_note(note, length * BEAT, "piano", volume=.16, sr=SR)
            layers["melody"] = mix_waves(layers["melody"], wave, start_time=start + position * BEAT, sr=SR)
    if section in (1, 2) and bar % 4 == 3:
        # The answering colour enters during a foreground rest.
        voice = instrument_note("D5" if section == 1 else "F5", BEAT * .8,
                                "clarinet", volume=.08, sr=SR)
        layers["answer"] = mix_waves(layers["answer"], voice, start_time=start + 3 * BEAT, sr=SR)
    if section != 0 or bar >= 4:
        for beat in [0, 1.5, 2, 3.5]:
            hit = instrument_note(440, .08, "shaker", volume=.09, sr=SR,
                                  seed=bar * 4 + int(beat * 2))
            layers["pulse"] = mix_waves(layers["pulse"], hit, start_time=start + beat * BEAT, sr=SR)

# A freely authored waveform remains available alongside the 72 named patches.
arrival = synth_note("D3", 2 * BEAT, wave_type="triangle", attack=.04,
                     decay=.3, sustain=.45, release=.3, volume=.07, sr=SR)
layers["bass"] = mix_waves(layers["bass"], arrival, start_time=DURATION - 2 * BEAT, sr=SR)
layers["answer"] = delay(layers["answer"], delay_time=BEAT * .75, feedback=.2, mix=.12, sr=SR)
layers["harmony"] = reverb(layers["harmony"], room_size=.55, mix=.16, sr=SR)
full_track = np.zeros((int(DURATION * SR), 2))
for name, pan in [("bass", 0), ("harmony", -.12), ("melody", -.25), ("answer", .3), ("pulse", .4)]:
    full_track = mix_waves(full_track, stereo_pan(layers[name], pan), sr=SR)
energy = automation_curve(DURATION, [(0, .8), (20, .95), (40, 1), (60, .85), (DURATION, .8)], sr=SR)
full_track = apply_automation(full_track, energy)
full_track = stereo_width(full_track, 1.05)
full_track = soft_limiter(full_track, drive=1.2, ceiling=.90)
full_track = make_seamless_loop(full_track, .04, sr=SR)
play_sound(full_track, sr=SR, loop=True)
