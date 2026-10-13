"""Preview decisions and archive isolation, with synthesis mocked out."""
import copy
from contextlib import ExitStack
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np

from flycomposer.buzzart.autonomy_render import render_request


class AutonomyRenderTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='buzzart-preview-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.job = self.root / 'run/jobs/001-test'
        self.job.mkdir(parents=True)
        self.destination = self.root / 'run/listening/001-test'
        self.request = {'root': str(self.root), 'job': str(self.job), 'destination': str(self.destination),
                        'context': 'indie', 'seed': 333, 'generation': 4, 'lane': 'heritage', 'id': '001-test'}
        self.request_path = self.job / 'request.json'
        self.score = {'duration_seconds': 180, 'bpm': 108, 'seed': 333, 'events': [{'id': 0}],
                      'event_sha256': 'raw-performance-hash', 'voices': {'lead': 'fm:lead'}}
        self.critic = {'acceptable': True, 'reward': .8, 'metrics': {'bass_coverage': .98}, 'rejection_reasons': []}
        self.audio_critic = {'passed': True, 'rejection_reasons': [], 'metrics': {}}
        self.audio = np.ones((16, 2)) * .02
        self.stems = {'lead': np.ones(16) * .02}
        self.audit = {'raw_rms': .02, 'raw_peak': .02, 'peak': .02, 'rms': .02}
        # An existing library is outside this feature's write surface.
        (self.root / 'library').mkdir()
        self.index = self.root / 'library/index.json'
        self.index.write_bytes(b'preserve the exact original index and comments')
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.mocks = {}
        for name in ('Brain.load', 'make_options', 'load_circuit', 'perform', 'evaluate', 'render', 'assess_audio', 'save_render', 'arrange'):
            self.mocks[name] = self.stack.enter_context(patch('flycomposer.buzzart.autonomy_render.' + name))
        self.mocks['make_options'].return_value = {}
        self.mocks['perform'].return_value = self.score
        self.mocks['evaluate'].return_value = self.critic
        self.mocks['render'].return_value = (self.audio, self.stems, self.audit)
        self.mocks['assess_audio'].return_value = self.audio_critic
        self.mocks['save_render'].side_effect = self.fake_save

    def fake_save(self, destination, score, audio, stems, audit, target):
        destination.mkdir(parents=True, exist_ok=False)
        (destination / 'audio.wav').write_bytes(b'fixture audio, not a real render')
        (destination / 'score.json').write_text(json.dumps(score))
        audit.update(rms=target, peak=.1)
        return audit

    def run_request(self):
        self.request_path.write_text(json.dumps(self.request))
        return render_request(self.request_path)

    def test_score_rejection_never_renders_or_archives(self):
        rejected = {**self.critic, 'acceptable': False, 'rejection_reasons': ['bass_stalls']}
        self.mocks['evaluate'].return_value = rejected
        result = self.run_request()
        self.assertFalse(result['passed'])
        self.assertEqual(result['stage'], 'score')
        self.assertFalse(self.destination.exists())
        self.mocks['render'].assert_not_called()
        self.mocks['save_render'].assert_not_called()
        self.assertEqual(json.loads((self.job / 'result.json').read_text())['rejection_reasons'], ['bass_stalls'])

    def test_audio_rejection_never_archives(self):
        self.mocks['assess_audio'].return_value = {'passed': False, 'rejection_reasons': ['near_silent_authored_gates']}
        result = self.run_request()
        self.assertFalse(result['passed'])
        self.assertEqual(result['stage'], 'audio')
        self.assertFalse(self.destination.exists())
        self.mocks['render'].assert_called_once()
        self.mocks['save_render'].assert_not_called()

    def test_raw_preview_is_labelled_and_preserves_main_index(self):
        result = self.run_request()
        self.assertTrue(result['passed'])
        self.assertEqual(result['candidate']['kind'], 'Raw practice capture')
        self.assertIn('checkpoint generation 4', result['candidate']['title'])
        self.assertNotIn('accepted generation', result['candidate']['title'])
        self.assertEqual(result['candidate']['metrics']['raw_reward'], .8)
        self.assertIsNone(result['candidate']['metrics']['arranged_reward'])
        self.assertIsNone(result['candidate']['listener_verdict'])
        self.assertEqual(self.index.read_bytes(), b'preserve the exact original index and comments')
        self.mocks['arrange'].assert_not_called()
        self.assertEqual(self.mocks['perform'].call_count, 1)

    def test_arranged_preview_uses_its_own_motif_and_separate_raw_fitness(self):
        self.request['context'] = 'overworld'
        raw = copy.deepcopy(self.score)
        directed = {**copy.deepcopy(self.score), 'event_sha256': 'directed-performance-hash'}
        motif = {**copy.deepcopy(self.score), 'duration_seconds': 61, 'event_sha256': 'new-seed-motif-hash'}
        arranged = {**copy.deepcopy(self.score), 'longform': {'source_track_id': 'approved-reference-id',
                     'source_event_sha256': motif['event_sha256'], 'raw_event_sha256': directed['event_sha256']}}
        self.mocks['perform'].side_effect = [raw, directed, motif]
        self.mocks['arrange'].return_value = arranged
        self.mocks['evaluate'].side_effect = [dict(self.critic, acceptable=False, reward=.4), dict(self.critic, reward=.9)]
        result = self.run_request()
        self.assertTrue(result['passed'])
        self.mocks['arrange'].assert_called_once_with(directed, motif, 'overworld')
        self.assertEqual(result['candidate']['kind'], 'Arranged study')
        self.assertEqual(result['candidate']['metrics']['raw_reward'], .4)
        self.assertEqual(result['candidate']['metrics']['arranged_reward'], .9)
        saved = json.loads((self.destination / 'score.json').read_text())
        self.assertNotIn('source_track_id', saved['longform'])
        self.assertEqual(saved['longform']['reference_track_id'], 'approved-reference-id')
        self.assertEqual(saved['longform']['source_event_sha256'], 'new-seed-motif-hash')
        self.assertEqual(json.loads((self.job / 'directed-score.json').read_text())['event_sha256'], saved['longform']['raw_event_sha256'])
        self.assertEqual(json.loads((self.job / 'motif-source-score.json').read_text())['event_sha256'], saved['longform']['source_event_sha256'])
        self.assertIsNone(result['candidate']['listener_verdict'])

    def test_existing_output_is_not_overwritten(self):
        self.destination.mkdir(parents=True)
        (self.destination / 'audio.wav').write_bytes(b'previous recording')
        with self.assertRaises(FileExistsError):
            self.run_request()
        self.assertEqual((self.destination / 'audio.wav').read_bytes(), b'previous recording')
        self.assertFalse((self.destination / 'critic.json').exists())


if __name__ == '__main__':
    unittest.main()
