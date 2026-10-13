"""Runtime isolation checks without starting trainer workers or rendering audio."""
import copy
import datetime as dt
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from concurrent.futures import Future, TimeoutError

import numpy as np

from flycomposer.buzzart import autonomy_train as runner
from flycomposer.buzzart.autonomy_checkpoint import load_checkpoint, save_checkpoint
from flycomposer.buzzart.brain import Brain


class InlineExecutor:
    def __init__(self, **kwargs):
        self._processes = {}

    def submit(self, function, argument):
        future = Future()
        try:
            future.set_result(function(argument))
        except BaseException as error:
            future.set_exception(error)
        return future

    def shutdown(self, **kwargs):
        pass


def summary(groups):
    rows = {context: {'objective': .6, 'critic': .7, 'legacy': .7, 'naturalness': .7,
                      'diversity': .7, 'metrics': {'bass_credited_coverage': .8,
                                                 'two_pitched_roles_share': .8}}
            for context, _, _ in groups}
    return {'objective': .6, 'critic': .7, 'contexts': rows}


class AutonomyRuntimeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'root'
        self.root.mkdir()
        brain = Brain(np.full((8,8), .1), np.full((8,8), .2), np.zeros((8,4)), np.zeros(8))
        for name in runner.PARENTS.values():
            brain.save(self.root / name)
        self.config = runner.configuration(hours=8, workers=1, population=4,
                                            max_generations=4, max_candidates=0)
        self.addCleanup(patch.stopall)
        patch.object(runner, 'frozen_inputs', return_value={}).start()
        patch.object(runner, 'ProcessPoolExecutor', InlineExecutor).start()
        self.evaluate = patch.object(runner, 'evaluate_groups',
            side_effect=lambda pool, vector, groups, details=True: summary(groups)).start()
        self.original_evaluate_task = runner.evaluate_task
        patch.object(runner, 'evaluate_task',
            side_effect=lambda task: {'objective': float(np.dot(task[0], np.arange(len(task[0]))))}).start()

    def test_split_run_resume_matches_uninterrupted_optimizer_and_rng(self):
        whole, split = self.root / 'whole', self.root / 'split'
        runner.run(self.root, whole, self.config)
        original_record = runner.record
        def stopping_record(run_dir, row):
            original_record(run_dir, row)
            if row.get('type') == 'generation' and row['generation'] == 2:
                (run_dir / 'STOP').touch()
        with patch.object(runner, 'record', side_effect=stopping_record):
            paused = runner.run(self.root, split, self.config)
        self.assertEqual(paused['status'], 'stopped')
        self.assertEqual(paused['generation'], 2)
        deadline = paused['deadline_at']
        (split / 'STOP').unlink()
        result = runner.run(self.root, split, resume=True)
        whole_state, whole_arrays = load_checkpoint(whole)
        split_state, split_arrays = load_checkpoint(split)
        self.assertEqual(result['status'], 'completed')
        self.assertEqual(split_state['deadline_at'], deadline)
        self.assertEqual(split_state['rng_state'], whole_state['rng_state'])
        self.assertEqual(split_state['generation'], 4)
        for key, value in whole_arrays.items():
            np.testing.assert_array_equal(value, split_arrays[key])
        for lane in runner.PARENTS:
            self.assertEqual(split_state['lanes'][lane]['optimizer_step'], 2)

    def test_stop_before_baseline_does_no_evaluation(self):
        run_dir = self.root / 'stop'
        run_dir.mkdir()
        (run_dir / 'STOP').touch()
        state = runner.run(self.root, run_dir, self.config)
        self.assertEqual(state['status'], 'stopped')
        self.evaluate.assert_not_called()
        self.assertFalse((run_dir / 'final-test.json').exists())
        self.assertEqual(load_checkpoint(run_dir)[0]['generation'], 0)

    def test_expired_resume_never_starts_new_final_evaluation(self):
        run_dir = self.root / 'expired'
        state, arrays = runner.initialize(self.root, run_dir, self.config)
        state['deadline_at'] = (dt.datetime.now(dt.timezone.utc)-dt.timedelta(seconds=1)).isoformat()
        save_checkpoint(run_dir, state, arrays)
        result = runner.run(self.root, run_dir, resume=True)
        self.assertEqual(result['status'], 'completed_with_incomplete_test')
        self.evaluate.assert_not_called()
        self.assertFalse((run_dir / 'final-test.json').exists())

    def test_resume_rejects_changed_config_and_changed_frozen_sources(self):
        run_dir = self.root / 'resume'
        state, arrays = runner.initialize(self.root, run_dir, self.config)
        save_checkpoint(run_dir, state, arrays)
        changed = {**self.config, 'population': 6}
        with self.assertRaisesRegex(ValueError, 'configuration differs'):
            runner.run(self.root, run_dir, changed, resume=True)
        state['input_hashes'] = {'missing.py': 'not-the-current-hash'}
        save_checkpoint(run_dir, state, arrays)
        with self.assertRaisesRegex(RuntimeError, 'Frozen inputs changed'):
            runner.run(self.root, run_dir, resume=True)
        self.evaluate.assert_not_called()

    def test_pause_before_baseline_waits_and_can_be_stopped(self):
        run_dir = self.root / 'pause'
        run_dir.mkdir()
        (run_dir / 'PAUSE').touch()
        def stop_on_sleep(seconds):
            self.assertLessEqual(seconds, 2.)
            (run_dir / 'STOP').touch()
        with patch.object(runner.time, 'sleep', side_effect=stop_on_sleep):
            result = runner.run(self.root, run_dir, self.config)
        self.assertEqual(result['status'], 'stopped')
        self.evaluate.assert_not_called()

    def test_seed_partitions_and_fresh_challenges_do_not_overlap(self):
        seen = set()
        for kind in ('validation', 'final'):
            seeds = {seed for _, values, _ in runner.validation_groups(kind) for seed in values}
            self.assertFalse(seen & seeds)
            seen |= seeds
        for generation in (1, 626, 1251, 19999, 20000):
            seeds = {seed for _, values, _ in runner.validation_groups('challenge', generation*8, 4, 180)
                     for seed in values}
            self.assertFalse(seen & seeds)
            self.assertEqual(len(seeds), len(runner.CONTEXTS)*4)
            seen |= seeds
        low, high = runner.SEEDS['train']
        self.assertFalse(any(low <= seed < high for seed in seen))
        self.assertTrue(all(seed < runner.SEEDS['listening'] for seed in seen))

    def test_eligibility_cannot_hide_context_regression_or_invalid_metrics(self):
        baseline = summary(runner.validation_groups())
        candidate = copy.deepcopy(baseline)
        candidate['objective'] = .9
        candidate['contexts']['dusk']['metrics']['bass_credited_coverage'] = .1
        self.assertTrue(any('bass_credited_coverage' in x for x in runner.eligible(candidate, baseline)))
        candidate = copy.deepcopy(baseline)
        del candidate['contexts']['dreamy']
        self.assertEqual(runner.eligible(candidate, baseline), ['evaluation context inventory changed'])
        candidate = copy.deepcopy(baseline)
        candidate['critic'] = float('nan')
        self.assertEqual(runner.eligible(candidate, baseline), ['nonfinite evaluation summary'])

    def test_invalid_runtime_bounds_fail_before_launch(self):
        for args in ({'preview_minutes': float('nan')}, {'max_generations': 2.5},
                     {'max_generations': 20001}, {'validate_every': True}):
            with self.subTest(args=args), self.assertRaises(ValueError):
                runner.configuration(**args)

    def test_hung_batch_deadline_terminates_only_owned_worker(self):
        clock = [0.]
        class Worker:
            alive = True
            terminated = False
            def is_alive(self): return self.alive
            def terminate(self): self.alive=False; self.terminated=True
            def join(self, timeout): pass
        worker = Worker()
        class Waiting:
            def result(self, timeout):
                clock[0] += timeout
                raise TimeoutError()
        class Hanging(InlineExecutor):
            def __init__(self, **kwargs): self._processes={1: worker}
            def submit(self, function, argument): return Waiting()
        with patch.object(runner, 'ProcessPoolExecutor', Hanging), patch.object(runner.time, 'time', side_effect=lambda: clock[0]):
            with self.assertRaises(runner.RunInterrupted):
                with runner.BoundedPool(self.root, self.root, 1, 3.) as pool:
                    list(pool.map(lambda x:x, [1]))
        self.assertTrue(worker.terminated)
        self.assertLessEqual(clock[0], 3.)

    def test_preview_attempt_allocation_survives_process_interruption(self):
        run_dir = self.root / 'preview'
        state, arrays = runner.initialize(self.root, run_dir, self.config)
        save_checkpoint(run_dir, state, arrays)
        with patch.object(runner.subprocess, 'run', side_effect=KeyboardInterrupt('simulated process interruption')):
            with self.assertRaises(KeyboardInterrupt):
                runner.preview(self.root, run_dir, state, arrays)
        restored, restored_arrays = load_checkpoint(run_dir)
        self.assertEqual(restored['preview_attempts'], 1)
        first_job = list((run_dir / 'jobs').iterdir())
        self.assertEqual(len(first_job), 1)
        # An unsuccessful next job remains evidence and consumes a fresh ID.
        class Failure:
            returncode = 1
        with patch.object(runner.subprocess, 'run', return_value=Failure()):
            runner.preview(self.root, run_dir, restored, restored_arrays)
        self.assertEqual(restored['preview_attempts'], 2)
        self.assertEqual(len(list((run_dir / 'jobs').iterdir())), 2)
        self.assertTrue(first_job[0].exists())


if __name__ == '__main__':
    unittest.main()
