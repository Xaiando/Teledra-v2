import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np

from flycomposer.buzzart.autonomy_checkpoint import (
    atomic_json, load_checkpoint, run_lock, save_checkpoint,
)


class AutonomyCheckpointTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def test_roundtrip_preserves_optimizer_rng_shapes_and_dtypes(self):
        rng = np.random.default_rng(281)
        arrays = {'theta': rng.normal(size=13), 'moments': np.zeros((2, 13)),
                  'step': np.array(23, dtype=np.int64)}
        state = {'generation': 23, 'rng': rng.bit_generator.state, 'remaining_seconds': 21200,
                 'config': {'population': 32, 'sigma': .07}}
        save_checkpoint(self.root, state, arrays)
        found, restored = load_checkpoint(self.root)
        self.assertEqual(found, state)
        for key in arrays:
            np.testing.assert_array_equal(restored[key], arrays[key])
            self.assertEqual(restored[key].dtype, arrays[key].dtype)
        clone = np.random.default_rng()
        clone.bit_generator.state = found['rng']
        np.testing.assert_array_equal(clone.normal(size=20), rng.normal(size=20))

    def test_missing_checkpoint_and_corruption_fail_closed(self):
        with self.assertRaises(FileNotFoundError):
            load_checkpoint(self.root)
        checkpoint = save_checkpoint(self.root, {'generation': 1}, {'theta': np.ones(4)})
        (self.root / 'checkpoints' / checkpoint / 'arrays.npz').write_bytes(b'altered')
        with self.assertRaisesRegex(ValueError, 'hash mismatch'):
            load_checkpoint(self.root)

    def test_interrupted_pointer_install_preserves_previous_checkpoint(self):
        previous = save_checkpoint(self.root, {'generation': 1}, {'theta': np.ones(4)})
        with patch('flycomposer.buzzart.autonomy_checkpoint.atomic_json', side_effect=OSError('interrupted')):
            with self.assertRaisesRegex(OSError, 'interrupted'):
                save_checkpoint(self.root, {'generation': 2}, {'theta': np.zeros(4)})
        state, arrays = load_checkpoint(self.root)
        self.assertEqual(state['generation'], 1)
        np.testing.assert_array_equal(arrays['theta'], np.ones(4))
        self.assertEqual(json.loads((self.root / 'checkpoint.json').read_bytes())['checkpoint_id'], previous)

    def test_retention_keeps_four_bundles_and_unrelated_files(self):
        unrelated = self.root / 'unrelated.txt'
        unrelated.write_text('preserve')
        ids = [save_checkpoint(self.root, {'generation': n}, {'theta': np.array([n])}) for n in range(7)]
        self.assertEqual({p.name for p in (self.root / 'checkpoints').iterdir()}, set(ids[-4:]))
        self.assertEqual(load_checkpoint(self.root)[0]['generation'], 6)
        self.assertEqual(unrelated.read_text(), 'preserve')

    def test_invalid_arrays_and_json_never_replace_committed_state(self):
        save_checkpoint(self.root, {'generation': 1}, {'theta': np.ones(4)})
        for values in (np.array([np.nan]), np.array([np.inf]), np.array([object()], dtype=object), np.array(['x'])):
            with self.subTest(dtype=values.dtype), self.assertRaises(ValueError):
                save_checkpoint(self.root, {'generation': 2}, {'theta': values})
        with self.assertRaises(ValueError):
            save_checkpoint(self.root, {'bad': float('nan')}, {'theta': np.ones(4)})
        self.assertEqual(load_checkpoint(self.root)[0]['generation'], 1)

    def test_pointer_traversal_and_manifest_tamper_rejected(self):
        checkpoint = save_checkpoint(self.root, {}, {'theta': np.ones(4)})
        manifest = self.root / 'checkpoints' / checkpoint / 'manifest.json'
        manifest.write_text('{}')
        with self.assertRaisesRegex(ValueError, 'manifest hash mismatch'):
            load_checkpoint(self.root)
        atomic_json(self.root / 'checkpoint.json', {'version': 1, 'checkpoint_id': '../outside'})
        with self.assertRaisesRegex(ValueError, 'Invalid checkpoint pointer'):
            load_checkpoint(self.root)

    def test_live_os_lock_excludes_second_holder_and_releases_on_error(self):
        with self.assertRaisesRegex(ValueError, 'training failed'):
            with run_lock(self.root):
                with self.assertRaisesRegex(RuntimeError, 'holds this run lock'):
                    with run_lock(self.root):
                        self.fail('Second holder acquired lock')
                raise ValueError('training failed')
        # A leftover lock file is not evidence of a live owner.
        with run_lock(self.root):
            self.assertTrue((self.root / 'run.lock').exists())

    def test_atomic_json_retries_brief_windows_reader_locks(self):
        target = self.root / 'status.json'
        atomic_json(target, {'generation': 1})
        original_replace = os.replace
        attempts = []
        def sharing_error(source, destination):
            attempts.append(Path(source))
            self.assertTrue(Path(source).exists())
            self.assertEqual(json.loads(target.read_bytes()), {'generation': 1})
            if len(attempts) <= 2:
                error = PermissionError('reader sharing violation')
                error.winerror = 32
                raise error
            original_replace(source, destination)
        with patch('flycomposer.buzzart.autonomy_checkpoint.os.replace', side_effect=sharing_error), \
             patch('flycomposer.buzzart.autonomy_checkpoint.time.sleep') as sleep:
            atomic_json(target, {'generation': 2})
        self.assertEqual(len(attempts), 3)
        self.assertEqual(len(set(attempts)), 1)
        self.assertEqual([call.args[0] for call in sleep.call_args_list], [.01, .02])
        self.assertEqual(json.loads(target.read_bytes()), {'generation': 2})
        self.assertFalse(list(self.root.glob('*.next')))

    def test_atomic_json_persistent_sharing_failure_preserves_old_target(self):
        target = self.root / 'status.json'
        atomic_json(target, {'generation': 1})
        original = target.read_bytes()
        error = PermissionError('persistent sharing violation')
        error.winerror = 33
        clock = [0.]
        def advance(seconds):
            self.assertGreater(seconds, 0)
            self.assertLessEqual(seconds, .2)
            clock[0] += seconds
        with patch('flycomposer.buzzart.autonomy_checkpoint.os.replace', side_effect=error) as replace, \
             patch('flycomposer.buzzart.autonomy_checkpoint.time.monotonic', side_effect=lambda:clock[0]), \
             patch('flycomposer.buzzart.autonomy_checkpoint.time.sleep', side_effect=advance):
            with self.assertRaises(PermissionError):
                atomic_json(target, {'generation': 2})
        self.assertGreater(replace.call_count, 2)
        self.assertAlmostEqual(clock[0], 2.)
        self.assertEqual(target.read_bytes(), original)
        self.assertFalse(list(self.root.glob('*.next')))

    def test_atomic_json_other_failures_are_not_retried(self):
        for error in (OSError('unrelated failure'), PermissionError('not a Windows sharing error')):
            with self.subTest(error=error), \
                 patch('flycomposer.buzzart.autonomy_checkpoint.os.replace', side_effect=error) as replace, \
                 patch('flycomposer.buzzart.autonomy_checkpoint.time.sleep') as sleep:
                with self.assertRaises(type(error)):
                    atomic_json(self.root / 'status.json', {'generation': 2})
            self.assertEqual(replace.call_count, 1)
            sleep.assert_not_called()
            self.assertFalse(list(self.root.glob('*.next')))


if __name__ == '__main__':
    unittest.main()
