"""Temperaments: how hard a style drives, and what the critic asks of it."""
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from flycomposer.buzzart.console_style import (
    DEFAULT_TEMPERAMENT, NO_STYLE_CRITIC, TEMPERAMENTS, ConsoleStyle, critic_bands)
from flycomposer.buzzart.critic import evaluate

CONSOLE = ROOT / "data" / "foundation" / "console.json"


def _legato(seconds=16., gap=.25, bpm=120):
    """A wall of even notes with no rest anywhere: the Mega Man extreme."""
    per_beat = 60. / bpm
    events, beat, index = [], 0., 0
    while beat * per_beat < seconds:
        events.append({"id": index, "layer": "lead", "voice": "chip:pulse25",
                       "midi": 72 + index % 5, "start_beat": beat, "length_beats": gap,
                       "velocity": .7, "section": 0, "start_seconds": beat * per_beat,
                       "duration_seconds": gap * per_beat, "harmonic_root": 0,
                       "scale_degree": index % 5, "decision": index})
        beat += gap
        index += 1
    return {"version": 1, "kind": "performance", "performer": "buzzart", "bpm": bpm,
            "tonic": "C", "mode": "major", "duration_seconds": seconds + 1., "events": events,
            "voices": {"lead": "chip:pulse25"}, "progression": [], "sections": [],
            "buzzart": {"style": None}}


class TemperamentTests(unittest.TestCase):
    def test_a_style_with_no_temperament_still_reads_as_arcade(self):
        # Scores and brains saved before temperaments existed carry a style
        # block with no name. They were trained under arcade, so that is what
        # they must keep scoring as.
        self.assertEqual(critic_bands({"palette": "nes"}), TEMPERAMENTS[DEFAULT_TEMPERAMENT]["critic"])
        self.assertEqual(critic_bands(None), NO_STYLE_CRITIC)

    def test_arcade_forgives_a_wall_of_notes_and_indie_does_not(self):
        # The whole complaint: arcade asks for no breathing at all, so relentless
        # music scores perfectly under it.
        wall = _legato()
        wall["buzzart"]["style"] = {"temperament": "arcade"}
        arcade = evaluate(wall)["components"]["breathing"]
        wall["buzzart"]["style"] = {"temperament": "indie"}
        indie = evaluate(wall)["components"]["breathing"]
        self.assertEqual(arcade, 1.)
        self.assertLess(indie, arcade)

    def test_indie_keeps_the_slower_half_of_the_corpus(self):
        arcade = ConsoleStyle.load(CONSOLE, temperament="arcade")
        indie = ConsoleStyle.load(CONSOLE, temperament="indie")
        self.assertGreater(max(arcade.tempos), 140)
        self.assertLessEqual(max(indie.tempos), 120)
        # The court's kingdom tunes sit at 96 BPM; indie has to be able to get there.
        self.assertIn(96, indie.tempos)
        # Tempos are the corpus's own, never invented.
        self.assertTrue(set(indie.tempos) <= set(arcade.tempos))

    def test_indie_slows_arpeggios_and_thins_the_drums(self):
        arcade = ConsoleStyle.load(CONSOLE, temperament="arcade")
        indie = ConsoleStyle.load(CONSOLE, temperament="indie")
        self.assertLessEqual(max(indie.arpeggio_rates), max(arcade.arpeggio_rates))
        drive = [1.] * 8
        self.assertLessEqual(sum(len(v) for v in indie.bar_grid(drive).values()),
                             sum(len(v) for v in arcade.bar_grid(drive).values()))
        # Thinner, but a groove still has a kick and a snare in it.
        busiest = indie.bar_grid(drive)
        self.assertTrue(any("kick" in hits for hits in busiest.values()))

    def test_the_temperament_is_recorded_so_the_critic_can_read_it_back(self):
        indie = ConsoleStyle.load(CONSOLE, temperament="indie")
        self.assertEqual(indie.identity()["temperament"], "indie")
        self.assertEqual(critic_bands(indie.identity()), TEMPERAMENTS["indie"]["critic"])

    def test_an_unknown_temperament_is_refused_rather_than_guessed(self):
        with self.assertRaises(ValueError):
            ConsoleStyle.load(CONSOLE, temperament="lofi-ish")


    def test_indie_holds_its_chords_and_arcade_still_arpeggiates(self):
        # A chip cannot hold a chord, so chiptune arpeggiates; under arpeggios
        # the harmony part's notes were 0.30 beats and slow pieces sounded thin.
        from flycomposer.buzzart.brain import Brain
        from flycomposer.buzzart.performer import perform
        from flycomposer.connectome import load_circuit
        circuit = load_circuit()
        brain = Brain.from_controller(circuit)
        lengths = {}
        for temperament in ("arcade", "indie"):
            style = ConsoleStyle.load(CONSOLE, temperament=temperament)
            score = perform(circuit, brain, 11, 30, style=style)
            harmony = sorted(e["length_beats"] for e in score["events"] if e["layer"] == "harmony")
            self.assertTrue(harmony, temperament)
            lengths[temperament] = harmony[len(harmony) // 2]
        self.assertLess(lengths["arcade"], 1.)
        self.assertGreater(lengths["indie"], 1.)

    def test_layering_is_measured_on_average_across_every_pitched_part(self):
        # Peak polyphony over lead, answer and bass could never pass 2, because
        # lead and answer take turns. Two parts held together for the whole
        # piece must read as two voices, not as a brief peak.
        score = _legato()
        score["events"] = []
        for layer, midi in (("harmony", 60), ("bass", 36)):
            score["events"].append({"id": len(score["events"]), "layer": layer, "voice": "x", "midi": midi,
                                    "start_beat": 0., "length_beats": 32., "velocity": .6, "section": 0,
                                    "start_seconds": 0., "duration_seconds": 16., "harmonic_root": 0,
                                    "scale_degree": 0, "decision": 0})
        facts = evaluate(score)["facts"]
        self.assertAlmostEqual(facts["mean_voices"], 2., delta=.05)

    def test_indie_scores_layering_by_the_average_and_arcade_keeps_its_peak_band(self):
        self.assertIn("voices", TEMPERAMENTS["indie"]["critic"])
        self.assertNotIn("voices", TEMPERAMENTS["arcade"]["critic"])

    def test_the_indie_palette_chains_chip_voices_with_pymusic(self):
        from flycomposer.buzzart import chip
        palette = chip.PALETTES["indie"]
        self.assertTrue(chip.uses_chip_voices(palette))
        pymusic = chip.pymusic_voices(palette)
        # The sustained parts are the ones handed to PyMusic.
        self.assertEqual(set(pymusic), {"harmony", "answer"})
        catalog = {v["id"] for v in __import__("json").loads((ROOT / "data" / "instruments.json").read_text(encoding="utf-8"))}
        self.assertTrue(set(pymusic.values()) <= catalog)

if __name__ == "__main__":
    unittest.main()
