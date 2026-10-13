"""Pair controls, delivery evidence and preservation at training publication."""
import copy
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from flycomposer.buzzart.brain import Brain
from flycomposer.buzzart.console_style import ConsoleStyle
from flycomposer.buzzart.training_showcase import paired_scores, publish_pairs
from flycomposer.connectome import load_circuit
from flycomposer.experiment import atomic_json


class TrainingShowcaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'data').mkdir()
        for name in ('circuit.npz', 'circuit.json', 'instruments.json'):
            shutil.copyfile(ROOT / 'data' / name, self.root / 'data' / name)
        self.index = self.root / 'library/index.json'
        self.baseline = {'version': 1, 'tracks': [{'id': 'old', 'note': 'Keep my exact comment', 'verdict': 'keep'}],
                         'comparisons': [{'id': 'old-pair', 'feedback': {'note': 'Listen again'}}]}
        atomic_json(self.index, self.baseline)
        (self.root / 'library/old').mkdir()
        (self.root / 'library/old/audio.wav').write_bytes(b'old audio fixture')
        (self.root / 'library/old/lead.wav').write_bytes(b'old stem fixture')
        self.circuit = load_circuit(self.root / 'data/circuit.npz')
        self.parent = Brain.from_controller(self.circuit, seed=98)
        vector = self.parent.vector().copy()
        vector[-1] += .1
        self.trained = self.parent.with_vector(vector)
        self.style = ConsoleStyle.load(ROOT / 'data/foundation/console.json', palette='genesis', temperament='indie')
        self.case = {'context': 'indie', 'seed': 317020000}

    def options(self, root, context, seed):
        return {'foundation': None, 'style': self.style}

    def test_pair_recomposes_at_parent_tempo_without_mutating_context(self):
        original_tempos = list(self.style.tempos)
        before, trained = paired_scores(self.circuit, self.parent, self.trained, self.case, 10, self.options(None, None, None))
        self.assertEqual(before['bpm'], trained['bpm'])
        self.assertEqual(before['voices'], trained['voices'])
        self.assertEqual(before['duration_seconds'], trained['duration_seconds'])
        self.assertEqual(self.style.tempos, original_tempos)
        self.assertEqual(before['buzzart']['style']['tempo_range'], [before['bpm']] * 2)
        with self.assertRaisesRegex(ValueError, 'explicit tempo'):
            paired_scores(self.circuit, self.parent, self.trained, self.case, 10, {'style': None})

    def test_actual_chip_pair_publishes_delivery_and_preserves_records_audio_and_stems(self):
        with patch('flycomposer.buzzart.training_showcase.server_running', return_value=False):
            output = publish_pairs(self.root, self.parent, self.trained, [self.case], 10,
                                   run_dir=self.root / 'run', options_factory=self.options, progress=lambda _: None)
        current = json.loads(self.index.read_text())
        self.assertEqual(current['tracks'][0], self.baseline['tracks'][0])
        self.assertEqual(current['comparisons'][0], self.baseline['comparisons'][0])
        self.assertEqual((self.root / 'library/old/lead.wav').read_bytes(), b'old stem fixture')
        self.assertEqual((self.root / 'library/old/audio.wav').read_bytes(), b'old audio fixture')
        tracks = current['tracks'][1:]
        self.assertEqual(len(tracks), 2)
        self.assertTrue(all(t['benchmark_metrics']['delivery']['complete'] for t in tracks))
        self.assertTrue(all(t['verdict'] is None and t['note'] == '' for t in tracks))
        self.assertAlmostEqual(tracks[0]['metrics']['rms'], tracks[1]['metrics']['rms'], places=7)
        self.assertEqual([v['display_name'] for v in current['comparisons'][-1]['variants'].values()], ['Before', 'Trained'])
        self.assertEqual(json.loads((output / 'manifest.json').read_text())['status'], 'published')
        self.assertEqual(json.loads((output / 'preservation.json').read_text())['wav_files'], 2)

    def test_concurrent_feedback_change_prevents_stale_publication(self):
        calls = 0
        def check_server(port):
            nonlocal calls
            calls += 1
            if calls == 2:
                updated = copy.deepcopy(self.baseline)
                updated['tracks'][0]['note'] = 'New feedback while rendering'
                atomic_json(self.index, updated)
            return False
        with patch('flycomposer.buzzart.training_showcase.server_running', side_effect=check_server):
            with self.assertRaisesRegex(RuntimeError, 'library changed'):
                publish_pairs(self.root, self.parent, self.trained, [self.case], 10,
                              run_dir=self.root / 'run', options_factory=self.options, progress=lambda _: None)
        current = json.loads(self.index.read_text())
        self.assertEqual(len(current['tracks']), 1)
        self.assertEqual(current['tracks'][0]['note'], 'New feedback while rendering')
        manifest = json.loads((self.root / 'run/listening-comparisons/manifest.json').read_text())
        self.assertEqual(manifest['status'], 'not_published')

    def test_running_server_is_refused_before_creating_output(self):
        with patch('flycomposer.buzzart.training_showcase.server_running', return_value=True):
            with self.assertRaisesRegex(RuntimeError, 'Close the Fly Composer server'):
                publish_pairs(self.root, self.parent, self.trained, [self.case], run_dir=self.root / 'run')
        self.assertFalse((self.root / 'run').exists())


if __name__ == '__main__':
    unittest.main()
