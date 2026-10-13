import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from flycomposer.benchmark import author_llm, validate_authoring
from flycomposer.buzzart.chip import PALETTES
from flycomposer.buzzart.showcase import render
from flycomposer.benchmark_metrics import measure


def authored():
    return {'title': 'A deliberately short test fixture', 'bpm': 96,
            'voices': dict(PALETTES['genesis']),
            'clips': [{'part': part, 'events': [[60, 0, 1, .5]], 'placements': [[0, 0]]}
                      for part in PALETTES['genesis']]}


class BenchmarkAuthoringTests(unittest.TestCase):
    def test_short_result_is_retained_exactly_without_filling_or_quality_retry(self):
        source = authored()
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / 'orator'
            with patch('flycomposer.benchmark._stream_request', return_value=(json.dumps(source), {'model': 'test-model'})) as request:
                score, evidence = author_llm(Path(temporary), directory, 'orator', 202609171,
                                             'test-model', b'Office persona fixture', lambda _: None)
            self.assertEqual(request.call_count, 1)
            self.assertEqual(score['authoring'], source)
            self.assertEqual(score['performer'], 'orator')
            self.assertEqual(len(score['events']), 5)
            self.assertEqual(score['musical_end_seconds'], .625)
            self.assertEqual(evidence['status'], 'authored')
            payload = request.call_args.args[1]
            self.assertEqual(payload['options']['seed'], 202609171)
            self.assertIn('64 quarter-note beats', payload['messages'][1]['content'])

    def test_only_model_supplies_the_single_technical_repair(self):
        bad, good = authored(), authored()
        bad['clips'][0]['placements'] = [[64, 0]]
        good['clips'][0]['events'] = [[67, 0, 2, .65]]
        replies = [(json.dumps(bad), {'model': 'test-model'}), (json.dumps(good), {'model': 'test-model'})]
        with tempfile.TemporaryDirectory() as temporary:
            with patch('flycomposer.benchmark._stream_request', side_effect=replies) as request:
                score, evidence = author_llm(Path(temporary), Path(temporary) / 'orator', 'orator', 1,
                                             'test-model', b'persona', lambda _: None)
            self.assertEqual(request.call_count, 2)
            self.assertEqual(score['authoring'], good)
            self.assertEqual([x['status'] for x in evidence['attempts']], ['failed', 'valid'])

    def test_wrong_controls_are_refused_and_failed_trial_retains_evidence(self):
        value = authored()
        value['bpm'] = 120
        with self.assertRaisesRegex(ValueError, '96 BPM'):
            validate_authoring(value, 'orator', 1)
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / 'orator'
            with patch('flycomposer.benchmark._stream_request', return_value=(json.dumps(value), {'model': 'test-model'})) as request:
                score, evidence = author_llm(Path(temporary), directory, 'orator', 1,
                                             'test-model', b'persona', lambda _: None)
            self.assertIsNone(score)
            self.assertEqual(request.call_count, 2)
            self.assertEqual(evidence['status'], 'failed')
            self.assertTrue((directory / 'result.json').is_file())
            self.assertFalse((directory / 'score.json').exists())

    def test_chip_delivery_is_identical_for_same_notes_regardless_of_author(self):
        score = validate_authoring(authored(), 'orator', 1)
        other = copy.deepcopy(score)
        other['performer'] = other['condition'] = 'buzzart'
        _, _, a = render(score, progress=lambda _: None)
        _, _, b = render(other, progress=lambda _: None)
        self.assertEqual([e['note_pcm_sha256'] for e in a['events']], [e['note_pcm_sha256'] for e in b['events']])
        facts = measure(score, a)
        self.assertTrue(facts['delivery']['complete'])
        self.assertAlmostEqual(facts['trailing_gap_seconds'], 39.375)


if __name__ == '__main__':
    unittest.main()
