"""The per-tune grammar: drawn coherently, recorded, and actually heard in the notes."""
import json
import sys
import unittest
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from flycomposer.buzzart import genome
from flycomposer.buzzart.brain import Brain
from flycomposer.buzzart.chip import CHIP_VOICES
from flycomposer.buzzart.console_style import ConsoleStyle, critic_bands
from flycomposer.buzzart.fingerprint import fingerprint
from flycomposer.buzzart.performer import perform
from flycomposer.connectome import load_circuit

DATA = json.loads((ROOT / "data" / "foundation" / "console.json").read_text(encoding="utf-8"))
CATALOG = {v["id"] for v in json.loads((ROOT / "data" / "instruments.json").read_text(encoding="utf-8"))}


class GenomeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.circuit = load_circuit()
        cls.brain = Brain.from_controller(cls.circuit)

    def perform(self, traits, palette, seed=7):
        return perform(self.circuit, self.brain, seed, 30, style=ConsoleStyle(DATA, palette=palette, temperament=traits))

    def test_a_grammar_is_a_pure_function_of_its_seed(self):
        self.assertEqual(genome.draw(4242), genome.draw(4242))
        self.assertNotEqual(genome.draw(4242), genome.draw(4243))

    def test_every_voice_the_genome_can_draw_exists(self):
        # One invented patch name was caught only by checking; this keeps it caught.
        pools = set(genome.LEAD) | set(genome.ANSWER) | set(genome.DRUM_VOICES) | set(genome.SPARKLE)
        for voices in list(genome.HARMONY.values()) + list(genome.BASS.values()):
            pools |= set(voices)
        self.assertEqual(sorted(v for v in pools if v not in CATALOG and v not in CHIP_VOICES), [])
        for songbook in genome.SONGBOOKS.values():
            self.assertTrue((ROOT / songbook).is_file(), songbook)

    def test_draws_cover_every_axis_rather_than_collapsing(self):
        grooves, harmonies, melodies = Counter(), Counter(), Counter()
        for seed in range(400):
            traits, _ = genome.draw(seed)
            grooves[traits.get("drum_pattern") or ("chip" if traits["drums"] else "none")] += 1
            harmonies[traits["harmony"]] += 1
            melodies[traits["melody"]] += 1
        self.assertEqual(set(grooves), set(genome.GROOVES))
        self.assertEqual(set(melodies), set(genome.MELODY_RHYTHMS))
        self.assertGreaterEqual(len(harmonies), 5)
        self.assertLess(max(grooves.values()) / 400, .25)

    def test_coherence_no_drums_means_no_stabs_or_chord_pulses(self):
        for seed in range(400):
            traits, palette = genome.draw(seed)
            if not traits["drums"]:
                self.assertNotIn(traits["harmony"], genome.NEEDS_DRUMS)
                self.assertNotIn(palette["texture"], genome.DRUM_VOICES)

    def test_the_mix_is_recorded_so_the_renderer_can_honour_it(self):
        traits, palette = genome.draw(11)
        score = self.perform(traits, palette)
        self.assertEqual(score["mix"], traits["mix"])
        style = score["buzzart"]["style"]
        self.assertEqual(critic_bands(style), traits["critic"])
        self.assertEqual(style["genome"]["melody"], traits["melody"])

    def test_swing_asked_for_is_swing_heard(self):
        traits, palette = genome.draw(11)
        traits = {**traits, "swing": .33, "drums": True, "drum_pattern": "boom_bap"}
        heard = fingerprint(self.perform(traits, palette))["swing"]
        straight = fingerprint(self.perform({**traits, "swing": 0.}, palette))["swing"]
        self.assertGreater(heard, straight + .15)

    def test_a_long_melody_really_is_long_and_a_running_one_really_runs(self):
        traits, palette = genome.draw(11)
        base = {k: v for k, v in traits.items()
                if k not in ("melody_time", "melody_grid", "melody_offsets", "riff", "melody_slots", "legato")}
        lengths = {}
        for name in ("long", "running"):
            score = self.perform({**base, **genome.MELODY_RHYTHMS[name],
                                  "lead_gate": 1.35 if name == "long" else .75}, palette)
            notes = sorted(e["length_beats"] for e in score["events"] if e["layer"] in ("lead", "answer"))
            lengths[name] = notes[len(notes) // 2]
        self.assertGreater(lengths["long"], 3 * lengths["running"])

    def test_a_named_drum_pattern_is_played_as_written(self):
        traits, palette = genome.draw(11)
        style = ConsoleStyle(DATA, palette=palette, temperament={**traits, "drums": True, "drum_pattern": "four_floor"})
        grid = style.bar_grid([0.] * 8)
        kicks = sorted(step for step, hits in grid.items() if "kick" in hits)
        self.assertEqual(kicks, [0, 4, 8, 12])


if __name__ == "__main__":
    unittest.main()
