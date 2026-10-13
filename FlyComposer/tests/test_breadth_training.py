import copy
from pathlib import Path
import unittest

import numpy as np

from flycomposer.buzzart.breadth_training import (
    TRAIN_SEEDS, VALID_SEEDS, TEST_SEEDS, VALID_CONTEXTS, make_options, ranks_with_ties, eligible,
)

ROOT = Path(__file__).resolve().parents[1]


class BreadthTrainingTests(unittest.TestCase):
    def test_flat_and_tied_rewards_do_not_prefer_candidate_order(self):
        np.testing.assert_array_equal(ranks_with_ties([.5]*8), np.zeros(8))
        np.testing.assert_allclose(ranks_with_ties([1, 3, 3, 1]), [-1/3, 1/3, 1/3, -1/3])

    def test_aggregate_gain_cannot_hide_one_context_regression(self):
        baseline = {'listener': .8, 'contexts': {c: {'listener': .8, 'critic': .8,
                    'naturalness': .8, 'diversity': .8} for c in VALID_CONTEXTS}}
        changed = copy.deepcopy(baseline)
        changed['listener'] = .9
        changed['contexts']['dreamy']['critic'] = .7
        self.assertEqual(len(eligible(changed, baseline)), 1)
        self.assertIn('dreamy', eligible(changed, baseline)[0])
        self.assertEqual(eligible(baseline, baseline), [])

    def test_disjoint_seed_sets_and_unstyled_transfer(self):
        self.assertFalse(set(TRAIN_SEEDS)&set(VALID_SEEDS))
        self.assertFalse(set(TRAIN_SEEDS)&set(TEST_SEEDS))
        self.assertFalse(set(VALID_SEEDS)&set(TEST_SEEDS))
        self.assertNotIn('acoustic', VALID_CONTEXTS)
        self.assertIsNone(make_options(ROOT, 'acoustic', TEST_SEEDS[0])['style'])

    def test_same_seed_preserves_grammar_and_broad_contexts_differ(self):
        first = make_options(ROOT, 'genome', VALID_SEEDS[0])
        again = make_options(ROOT, 'genome', VALID_SEEDS[0])
        self.assertEqual(first['style'].identity(), again['style'].identity())
        self.assertEqual(first['foundation'].sha256, again['foundation'].sha256)
        options = [make_options(ROOT, c, VALID_SEEDS[0]) for c in VALID_CONTEXTS]
        self.assertGreaterEqual(len({str(o['style'].palette) for o in options}), 6)
        self.assertGreaterEqual(len({o['foundation'].sha256 for o in options}), 4)


if __name__ == '__main__':
    unittest.main()
