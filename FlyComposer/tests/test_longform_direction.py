"""Regression checks for optional direction of commissioned long performances."""
import json
from pathlib import Path
import unittest

from flycomposer.buzzart.brain import Brain
from flycomposer.buzzart.breadth_training import make_options
from flycomposer.buzzart.longform import Direction
from flycomposer.buzzart.performer import perform
from flycomposer.connectome import load_circuit


ROOT = Path(__file__).resolve().parents[1]


class LongformDirectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.circuit = load_circuit()
        cls.brain = Brain.from_controller(cls.circuit)

    def test_identity_direction_preserves_events_and_controller_trace(self):
        baseline = perform(self.circuit, self.brain, seed=173, seconds=90)
        identity = perform(self.circuit, self.brain, seed=173, seconds=90,
                           direction=lambda index, seconds, energy: (index, energy))
        self.assertEqual(identity['events'], baseline['events'])
        self.assertEqual(identity['event_sha256'], baseline['event_sha256'])
        self.assertEqual(identity['neural_trace'], baseline['neural_trace'])
        self.assertTrue(all('direction' not in d for d in baseline['decision_log']))
        self.assertTrue(all('direction' in d for d in identity['decision_log']))

    def test_direction_starts_after_one_minute_without_leaking_to_default(self):
        baseline = perform(self.circuit, self.brain, seed=173, seconds=180)
        directed = perform(self.circuit, self.brain, seed=173, seconds=180,
                           direction=Direction('dusk'))
        before = [d for d in directed['decision_log'] if d['at_seconds'] <= 60]
        self.assertGreater(len(before), 1)
        for d, original in zip(before, baseline['decision_log']):
            self.assertEqual(d['at_seconds'], original['at_seconds'])
            self.assertEqual(d['outputs'], original['outputs'])
            self.assertEqual(d['direction']['controller_index'], d['index'])
            self.assertEqual(d['direction']['feedback_energy'], d['input_feedback_energy'])
        first_changed = next(d for d in directed['decision_log'] if d['at_seconds'] > 60)
        self.assertNotEqual(first_changed['direction']['controller_index'], first_changed['index'])
        self.assertNotEqual(first_changed['outputs'], baseline['decision_log'][first_changed['index']]['outputs'])
        self.assertNotEqual(directed['event_sha256'], baseline['event_sha256'])
        again = perform(self.circuit, self.brain, seed=173, seconds=180)
        self.assertEqual(again['events'], baseline['events'])
        self.assertEqual(again['neural_trace'], baseline['neural_trace'])

    def test_selected_dusk_raw_extension_reproduces_original_first_55_seconds(self):
        source = json.loads((ROOT / 'library/d4906349cc154ac1/score.json').read_bytes())
        brain = Brain.load(ROOT / 'brains/buzzart-wide-candidate.npz')
        options = make_options(ROOT, 'dusk', source['seed'])
        options['style'].tempos = [source['bpm']]
        raw = perform(self.circuit, brain, seed=source['seed'], seconds=180,
                      voices=source['voices'], direction=Direction('dusk'), **options)
        # Stay away from the original recording's capture-boundary truncation.
        source_opening = [e for e in source['events'] if e['start_seconds'] < 55]
        extended_opening = [e for e in raw['events'] if e['start_seconds'] < 55]
        self.assertGreater(len(source_opening), 250)
        self.assertEqual(extended_opening, source_opening)
        self.assertEqual(raw['controller']['brain']['brain_sha256'],
                         source['controller']['brain']['brain_sha256'])

    def test_nonfinite_direction_inputs_are_rejected(self):
        for invalid in ((float('nan'), .5), (1., float('inf'))):
            with self.subTest(invalid=invalid), self.assertRaisesRegex(ValueError, 'finite inputs'):
                perform(self.circuit, self.brain, seed=173, seconds=30,
                        direction=lambda index, seconds, energy: invalid)


if __name__ == '__main__':
    unittest.main()
