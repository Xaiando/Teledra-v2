from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import numpy as np

from composer_harness import audio_quality_metrics
from music_verify import verify_file


GOOD = """
import numpy as np
from teledra_synth import *
sr = 8000
n = sr * 4
t = np.arange(n, dtype=float) / sr
edge = np.sin(np.pi * np.arange(n) / (n - 1)) ** 0.5
form = np.repeat([0.55, 0.82, 1.0, 0.64], sr) * edge
bass = 0.050 * np.sin(2 * np.pi * 55 * t) * form
body = 0.045 * np.sin(2 * np.pi * 110 * t) * form
pad = 0.038 * np.sin(2 * np.pi * 165 * t) * form
lead = 0.040 * np.sin(2 * np.pi * 220 * t) * form
air = 0.025 * np.sin(2 * np.pi * 330 * t) * form
full_track = bass + body + pad + lead + air
TELEDRA_LAYERS = {"bass": bass, "body": body, "pad": pad, "lead": lead, "air": air}
TELEDRA_SECTIONS = {
    "arrival": full_track[:sr],
    "development": full_track[sr:2*sr],
    "peak": full_track[2*sr:3*sr],
    "release": full_track[3*sr:],
}
TELEDRA_SCORE = {
    "motif": ["A3", "C4", "E4"],
    "sections": ["arrival", "development", "peak", "release"],
    "depth_roles": {"foreground": "lead", "midground": "pad", "background": "air"},
}
TELEDRA_AUTOMATION = {
    "energy_arc": "applied by form envelope",
    "edge_fade": "applied by edge envelope",
    "layer_balance": "applied by distinct layer gains",
}
BPM = 96
TELEDRA_COMPOSER = {
    "style_profile": "retro_adventure",
    "tonal_center": "A",
    "mode": "natural_minor",
    "progression_degrees": [1, 6, 3, 7],
    "chord_voicings": [["A3", "C4", "E4"], ["F3", "A3", "C4"], ["C4", "E4", "G4"], ["G3", "B3", "D4"]],
    "motif_notes": ["A4", "C5", "B4", "A4", "E5", "D5", "C5", "A4"],
    "phrase_bars": 4,
    "transformations": ["fragment", "answer", "register_shift"],
    "swing": 0.08,
    "registers": {"bass": [1, 2], "harmony": [3, 4], "lead": [4, 6]},
    "section_density": [0.3, 0.55, 0.9, 0.45],
    "intentional_tensions": [],
    "tension_policy": "Diatonic suspensions resolve downward before each cadence.",
}
play_sound(full_track, sr=sr, loop=True)
"""


COMPOSER_GOOD = """
import numpy as np
from teledra_synth import play_sound, synth_note

sr = 8000
BPM = 120
BARS = 4
BEATS_PER_BAR = 4
BEAT_SECONDS = 60.0 / BPM
TOTAL_BEATS = BARS * BEATS_PER_BAR
n = int(TOTAL_BEATS * BEAT_SECONDS * sr)
SECTION_NAMES = ["arrival", "development", "peak", "release"]
RENDER_EVENTS = True

motif = ["A4", "C5", "B4", "A4", "E5", "D5", "C5", "A4"]
chords = [
    ["A3", "C4", "E4"],
    ["F3", "A3", "C4"],
    ["C4", "E4", "G4"],
    ["G3", "B3", "D4"],
]
TELEDRA_SCORE = {
    "motif": motif,
    "sections": SECTION_NAMES,
    "meter": [4, 4],
    "depth_roles": {
        "foreground": ["lead"],
        "midground": ["harmony", "motion", "drums"],
        "background": ["bass", "air"],
    },
}
TELEDRA_AUTOMATION = {
    "energy_arc": [0.55, 0.76, 1.0, 0.48],
    "lead_density": [0.5, 0.75, 1.0, 0.25],
    "motion_density": [0.0, 0.5, 1.0, 0.0],
}
TELEDRA_COMPOSER = {
    "style_profile": "retro_adventure",
    "tonal_center": "A",
    "mode": "natural_minor",
    "progression_degrees": [1, 6, 3, 7],
    "chord_voicings": chords,
    "motif_notes": motif,
    "phrase_bars": 4,
    "transformations": ["fragment", "answer", "register_shift"],
    "swing": 0.08,
    "registers": {"bass": [1, 2], "harmony": [3, 4], "lead": [4, 6]},
    "section_density": [0.35, 0.67, 1.0, 0.25],
    "intentional_tensions": [],
    "tension_policy": "Diatonic suspensions resolve downward before each cadence.",
}

layers = {
    "bass": np.zeros(n),
    "harmony": np.zeros(n),
    "lead": np.zeros(n),
    "motion": np.zeros(n),
    "drums": np.zeros(n),
    "air": np.zeros(n),
}
events = []

def section_for(start_beat):
    return SECTION_NAMES[min(int(start_beat // 4), len(SECTION_NAMES) - 1)]

def place(track, wave, start_beat):
    start = int(start_beat * BEAT_SECONDS * sr)
    end = min(n, start + len(wave))
    if RENDER_EVENTS and end > start:
        layers[track][start:end] += wave[:end-start]

def add_note(track, role, pitch, start_beat, duration_beats, velocity, transform="prime"):
    wave = synth_note(
        pitch,
        duration_beats * BEAT_SECONDS,
        wave_type="triangle" if role in {"bass", "harmony"} else "square",
        attack=0.004,
        decay=0.025,
        sustain=0.58,
        release=0.04,
        volume=0.24 * velocity,
        sr=sr,
    )
    place(track, wave, start_beat)
    events.append({
        "kind": "note",
        "track": track,
        "role": role,
        "pitch": pitch,
        "start_beat": float(start_beat),
        "duration_beats": float(duration_beats),
        "velocity": float(velocity),
        "section": section_for(start_beat),
        "motif": "royal_call" if role in {"lead", "motion"} else "",
        "transform": transform,
    })

def add_drum(start_beat, velocity):
    duration_beats = 0.18
    wave = synth_note(
        "C6",
        duration_beats * BEAT_SECONDS,
        wave_type="white_noise",
        attack=0.001,
        decay=0.008,
        sustain=0.0,
        release=0.025,
        volume=0.18 * velocity,
        sr=sr,
    )
    place("drums", wave, start_beat)
    events.append({
        "kind": "drum",
        "track": "drums",
        "role": "percussion",
        "start_beat": float(start_beat),
        "duration_beats": duration_beats,
        "velocity": float(velocity),
        "section": section_for(start_beat),
        "motif": "",
        "transform": "",
    })

lead_sequences = [motif, motif + motif[:4], motif + motif, motif[:4]]
motion_counts = [0, 4, 8, 0]
drum_counts = [2, 4, 8, 2]
transforms = ["prime", "fragment", "register_shift", "answer"]
for section_index, section_start in enumerate((0.0, 4.0, 8.0, 12.0)):
    chord = chords[section_index]
    for pitch in chord:
        add_note("harmony", "harmony", pitch, section_start, 3.82, 0.45, "statement")
    add_note("air", "texture", "A5", section_start, 3.72, 0.16, "statement")
    bass_pitch = ["A2", "F2", "C2", "G2"][section_index]
    for beat_offset in range(4):
        add_note("bass", "bass", bass_pitch, section_start + beat_offset, 0.64, 0.74, "statement")
    sequence = lead_sequences[section_index]
    lead_step = 4.0 / len(sequence)
    for note_index, pitch in enumerate(sequence):
        add_note(
            "lead", "lead", pitch,
            section_start + note_index * lead_step,
            lead_step * 0.58,
            0.72,
            transforms[section_index],
        )
    motion_count = motion_counts[section_index]
    for motion_index in range(motion_count):
        add_note(
            "motion", "motion", ["E5", "C5", "B4", "A4"][motion_index % 4],
            section_start + motion_index * (4.0 / motion_count) + 0.18,
            0.22,
            0.34,
            "fragment" if section_index == 1 else "register_shift",
        )
    drum_count = drum_counts[section_index]
    for drum_index in range(drum_count):
        add_drum(section_start + drum_index * (4.0 / drum_count) + 0.08, 0.62)

if not RENDER_EVENTS:
    timeline = np.arange(n, dtype=float) / sr
    for index, track in enumerate(layers):
        layers[track] = 0.055 * np.sin(2 * np.pi * (55.0 * (index + 1)) * timeline)

edge = np.sin(np.pi * np.arange(n) / (n - 1)) ** 0.5
form = np.repeat([0.55, 0.76, 1.0, 0.48], n // 4)
for track in layers:
    layers[track] *= edge * form
full_track = 0.72 * sum(layers.values())
TELEDRA_LAYERS = layers
TELEDRA_EVENTS = events
TELEDRA_SECTIONS = {
    name: full_track[index * (n // 4):(index + 1) * (n // 4)]
    for index, name in enumerate(SECTION_NAMES)
}
play_sound(full_track, sr=sr, loop=True)
"""

TYPO = """
import numpy as np
from teledra_synth import *
sr = 8000
lead = synth_note("H4", 1.0, sr=sr)
pad = synth_note("C4", 1.0, sr=sr, volume=0.1)
full_track = lead + pad
TELEDRA_LAYERS = {"lead": lead, "pad": pad}
play_sound(full_track, sr=sr, loop=True)
"""

DEAD = """
import numpy as np
from teledra_synth import *
sr = 8000
t = np.arange(sr, dtype=float) / sr
lead = 0.2 * np.sin(2 * np.pi * 220 * t)
counterline = np.zeros_like(lead)
full_track = lead + counterline
TELEDRA_LAYERS = {"lead": lead, "counterline": counterline}
play_sound(full_track, sr=sr, loop=True)
"""

CLIPPING = """
import numpy as np
from teledra_synth import *
sr = 8000
t = np.arange(sr, dtype=float) / sr
lead = 0.8 * np.sin(2 * np.pi * 220 * t)
bass = 0.8 * np.sin(2 * np.pi * 220 * t)
full_track = lead + bass
TELEDRA_LAYERS = {"lead": lead, "bass": bass}
play_sound(full_track, sr=sr, loop=True)
"""


class MusicVerifierTests(unittest.TestCase):
    def verify(self, source: str, *, composer_grade: bool = False):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "candidate.py"
            path.write_text(source, encoding="utf-8")
            return verify_file(path, composer_grade=composer_grade)

    def test_good_track_passes(self):
        report = self.verify(GOOD)
        self.assertTrue(report["ok"], report)

    def test_typoed_note_reports_invalid_note(self):
        report = self.verify(TYPO)
        self.assertFalse(report["ok"])
        self.assertIn("invalid_note", {issue["code"] for issue in report["issues"]})

    def test_dead_layer_reports_layer_name(self):
        report = self.verify(DEAD)
        self.assertFalse(report["ok"])
        issue = next(issue for issue in report["issues"] if issue["code"] == "dead_layer")
        self.assertEqual("counterline", issue["layer"])

    def test_clipping_reports_peak(self):
        report = self.verify(CLIPPING)
        self.assertFalse(report["ok"])
        issue = next(issue for issue in report["issues"] if issue["code"] == "clipping")
        self.assertGreaterEqual(issue["peak"], issue["threshold"])

    def test_composer_grade_accepts_coherent_retro_plan(self):
        report = self.verify(COMPOSER_GOOD, composer_grade=True)
        self.assertTrue(report["ok"], report)
        self.assertEqual("retro_adventure", report["composer_metrics"]["style_profile"])
        self.assertTrue(report["composer_events"])
        self.assertGreaterEqual(report["composer_metrics"]["performed_scale_fit"], 0.9)
        self.assertGreaterEqual(report["composer_metrics"]["motif_trace_occurrences"], 1)
        self.assertGreaterEqual(report["quality_score"], 90)

    def test_composer_grade_accepts_spicy_lofi_seventh_harmony(self):
        lofi = (
            COMPOSER_GOOD
            .replace("BPM = 120", "BPM = 84")
            .replace('"style_profile": "retro_adventure"', '"style_profile": "spicy_lofi"')
            .replace('"swing": 0.08', '"swing": 0.18')
            .replace(
                '''    ["A3", "C4", "E4"],
    ["F3", "A3", "C4"],
    ["C4", "E4", "G4"],
    ["G3", "B3", "D4"],''',
                '''    ["A3", "C4", "E4", "G4"],
    ["F3", "A3", "C4", "E4"],
    ["C4", "E4", "G4", "B4"],
    ["G3", "B3", "D4", "F4"],''',
            )
            .replace("full_track = 0.72 * sum(layers.values())", "full_track = 0.58 * sum(layers.values())")
        )
        report = self.verify(lofi, composer_grade=True)
        self.assertTrue(report["ok"], report)
        self.assertEqual("spicy_lofi", report["composer_metrics"]["style_profile"])
        self.assertEqual(1.0, report["composer_metrics"]["performed_chord_ratio"])

    def test_composer_grade_rejects_missing_musical_plan(self):
        report = self.verify(COMPOSER_GOOD.replace("TELEDRA_COMPOSER = {", "IGNORED_COMPOSER = {"), composer_grade=True)
        self.assertFalse(report["ok"])
        self.assertIn("missing_composer_plan", {issue["code"] for issue in report["issues"]})

    def test_composer_grade_rejects_jagged_motif_and_unresolved_tones(self):
        bad = COMPOSER_GOOD.replace(
            'motif = ["A4", "C5", "B4", "A4", "E5", "D5", "C5", "A4"]',
            'motif = ["A2", "G5", "A2", "F#5", "A2", "G5"]',
        ).replace(
            '["F3", "A3", "C4"]',
            '["F3", "A3", "C#4"]',
        )
        report = self.verify(bad, composer_grade=True)
        codes = {issue["code"] for issue in report["issues"]}
        self.assertIn("jagged_melody", codes)
        self.assertIn("unresolved_dissonance", codes)

    def test_composer_grade_rejects_static_audio_with_only_a_good_plan(self):
        report = self.verify(GOOD, composer_grade=True)
        self.assertFalse(report["ok"])
        self.assertIn("missing_composer_events", {issue["code"] for issue in report["issues"]})

    def test_composer_grade_rejects_false_events_over_stationary_stems(self):
        false_performance = COMPOSER_GOOD.replace("RENDER_EVENTS = True", "RENDER_EVENTS = False")
        report = self.verify(false_performance, composer_grade=True)
        self.assertFalse(report["ok"])
        self.assertIn("event_audio_mismatch", {issue["code"] for issue in report["issues"]})

    def test_composer_grade_rejects_event_outside_declared_score(self):
        bad = COMPOSER_GOOD.replace(
            "play_sound(full_track, sr=sr, loop=True)",
            'TELEDRA_EVENTS[0]["start_beat"] = 99.0\nplay_sound(full_track, sr=sr, loop=True)',
        )
        report = self.verify(bad, composer_grade=True)
        self.assertFalse(report["ok"])
        self.assertIn("event_out_of_bounds", {issue["code"] for issue in report["issues"]})

    def test_composer_grade_rejects_claimed_density_opposite_to_events(self):
        bad = COMPOSER_GOOD.replace(
            '"section_density": [0.35, 0.67, 1.0, 0.25]',
            '"section_density": [1.0, 0.7, 0.4, 0.1]',
        )
        report = self.verify(bad, composer_grade=True)
        self.assertFalse(report["ok"])
        self.assertIn("section_density_mismatch", {issue["code"] for issue in report["issues"]})

    def test_composer_grade_rejects_extremely_underpowered_mix(self):
        bad = COMPOSER_GOOD.replace(
            "play_sound(full_track, sr=sr, loop=True)",
            "full_track *= 0.001\nplay_sound(full_track, sr=sr, loop=True)",
        )
        report = self.verify(bad, composer_grade=True)
        self.assertFalse(report["ok"])
        self.assertIn("underpowered_mix", {issue["code"] for issue in report["issues"]})

    def test_composer_grade_warns_on_quiet_but_usable_mix(self):
        quiet = COMPOSER_GOOD.replace(
            "play_sound(full_track, sr=sr, loop=True)",
            "full_track *= 0.4\nplay_sound(full_track, sr=sr, loop=True)",
        )
        report = self.verify(quiet, composer_grade=True)
        self.assertTrue(report["ok"], report)
        self.assertIn("underpowered_mix", {item["code"] for item in report["composer_advisories"]})

    def test_long_high_frequency_signal_is_measured_without_stride_aliasing(self):
        sr = 44100
        timeline = np.arange(sr * 48, dtype=float) / sr
        wave = 0.2 * np.sin(2 * np.pi * 10000.0 * timeline)
        metrics = audio_quality_metrics(wave, sr)
        self.assertGreater(metrics["high_frequency_ratio"], 0.9)


if __name__ == "__main__":
    unittest.main()
