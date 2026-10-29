"""Acceptance-boundary tests for the paired parity bound.

These exercise the exact failure/success semantics publication relies on:
faster, slower, and inconclusive (too-uncertain) candidate samples on the
same shape the benchmark produces. They are not CLI forwarding tests: no
subprocesses, no media, no source-text assertions.
"""

import importlib.util
from pathlib import Path
import random
import unittest


SPEC = importlib.util.spec_from_file_location("verify_linux_asset",
                                              Path(__file__).with_name(
                                                  "verify_linux_asset.py"))
verifier = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verifier)

SEED = 20260117
PAIRS = verifier.PAIRS


class ParityUpperBoundTests(unittest.TestCase):
    def samples(self, offset, scale=0.02, baseline_mean=100.0):
        """Deterministic baseline/candidate sample lists."""
        rng = random.Random(SEED)
        baseline = [baseline_mean * (1 + offset + scale * rng.uniform(-1, 1))
                    for _ in range(PAIRS)]
        candidate = [bv + offset * baseline_mean *
                     (1 + scale * rng.uniform(-1, 1))
                     for bv in baseline]
        return baseline, candidate

    def test_identical_samples_measure_zero(self):
        rng = random.Random(SEED)
        baseline = [100.0 + 5.0 * rng.uniform(-1, 1) for _ in range(PAIRS)]
        self.assertEqual(verifier.parity_upper_bound(baseline, baseline), 0.0)

    def test_consistently_faster_candidate_passes(self):
        baseline, candidate = self.samples(offset=-0.20)
        self.assertLess(verifier.parity_upper_bound(baseline, candidate), 0.0)

    def test_consistently_slower_candidate_fails(self):
        baseline, candidate = self.samples(offset=+0.20)
        self.assertGreater(verifier.parity_upper_bound(baseline, candidate), 0.0)

    def test_faster_mean_with_slow_outlier_fails(self):
        # Point estimate is faster, but one slow outlier makes the
        # one-sided 95% bound cross zero: inconclusive means failure.
        baseline = [100.0 for _ in range(PAIRS)]
        candidate = [98.0 for _ in range(PAIRS - 1)] + [30.0]
        bound = verifier.parity_upper_bound(baseline, candidate)
        mean_ratio = sum(candidate) / sum(baseline)
        self.assertLess(mean_ratio, 1.0)
        self.assertGreater(bound, 0.0)

    def test_too_few_pairs_fails_closed(self):
        with self.assertRaises(ValueError):
            verifier.parity_upper_bound([1.0] * (PAIRS - 1), [1.0] * (PAIRS - 1))

    def test_unequal_sample_lengths_fails_closed(self):
        with self.assertRaises(ValueError):
            verifier.parity_upper_bound([1.0] * PAIRS, [1.0] * (PAIRS + 1))

    def test_nonpositive_timing_fails_closed(self):
        baseline = [1.0] * PAIRS
        candidate = [1.0] * (PAIRS - 1) + [0.0]
        with self.assertRaises(ValueError):
            verifier.parity_upper_bound(baseline, candidate)


if __name__ == "__main__":
    unittest.main()
