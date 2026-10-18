"""Regression checks for phrase-scoped polyphonic readings."""

from pathlib import Path
import tempfile
import unittest

from polyphone_corrections import corrected_reading, load_corrections


class PolyphoneCorrectionsTest(unittest.TestCase):
    def test_exact_rule_does_not_override_compound(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "corrections.tsv"
            path.write_text("朝阳\tchao yang\texact\n朝阳市\tchao yang shi\n", encoding="utf-8")
            rules = load_corrections(path)
            self.assertEqual(corrected_reading("朝阳", rules), ("chao", "yang"))
            self.assertEqual(corrected_reading("朝阳市", rules), ("chao", "yang", "shi"))
            self.assertEqual(corrected_reading("张朝阳", rules), ("zhang", "zhao", "yang"))

    def test_longest_substring_and_invalid_scope(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "corrections.tsv"
            path.write_text("重庆\tchong qing\n重庆市\tchong qing shi\n", encoding="utf-8")
            self.assertEqual(corrected_reading("重庆市区", load_corrections(path)),
                             ("chong", "qing", "shi", "qu"))
            path.write_text("朝阳\tchao yang\tglobal\n", encoding="utf-8")
            with self.assertRaises(ValueError):
                load_corrections(path)

    def test_repo_keeps_contextual_readings(self):
        root = Path(__file__).resolve().parents[1]
        rules = load_corrections(root / "data_sources/kaixin/polyphone_corrections.tsv")
        for phrase in ("朝阳", "流血", "穿着"):
            self.assertEqual(next(rule.scope for rule in rules if rule.phrase == phrase), "exact")
        three_chars = (root / "lexicon/zh/life_common_3char.txt").read_text(encoding="utf-8")
        for phrase, reading in (("朝阳市", "chao yang shi"),
                                ("朝阳县", "chao yang xian"),
                                ("朝阳门", "chao yang men")):
            self.assertIn(f"{phrase}\t{reading}\t", three_chars)


if __name__ == "__main__":
    unittest.main()
