import unittest
from input_benchmark_report import percentiles
from build_input_cases_sqlite import read_rows
from pathlib import Path


class BenchmarkContractTests(unittest.TestCase):
    def test_tail_latency_keeps_outlier_and_rejects_empty_samples(self):
        metric = percentiles([100.0] * 99 + [9000.0])
        self.assertEqual(metric["p95_us"], 100.0)
        self.assertEqual(metric["max_us"], 9000.0)
        with self.assertRaises(ValueError):
            percentiles([])

    def test_corpus_keeps_all_required_groups_and_distinct_cases(self):
        rows = read_rows(Path(__file__).resolve().parents[1] / "tests/category_ranking_cases.tsv")
        groups = {row[1] for row in rows}
        self.assertEqual(groups, {"daily", "office", "professional", "mixed", "correction"})
        self.assertGreaterEqual(len(rows), 300)
        self.assertTrue(all(sum(row[1] == group for row in rows) >= 50 for group in groups))


if __name__ == "__main__":
    unittest.main()
