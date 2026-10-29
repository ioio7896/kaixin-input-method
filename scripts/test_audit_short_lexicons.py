from pathlib import Path
import tempfile
import unittest

import audit_short_lexicons as audit


class ShortLexiconAuditTests(unittest.TestCase):
    def test_risk_flag_does_not_reject_complete_terms_names_or_expressions(self):
        for phrase, longer in (("张量分析", "张量分析方法"), ("谱分析", "频谱分析"),
                               ("名作家", "著名作家"), ("越来越", "越来越好"),
                               ("芯片组", "芯片组驱动")):
            self.assertTrue(audit.risk_flags(phrase, {longer}))
        # Flags deliberately include valid words. No rejection decision is
        # inferred from substring membership or from functional characters.
        self.assertIn("comparative_frame", audit.risk_flags("越来越", set()))

    def test_scan_preserves_all_readings_and_records_without_rewriting_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "lexicon/zh/sample.txt"
            path.parent.mkdir(parents=True)
            text = ("# source\n计算机\tji suan ji\t9000\n"
                    "张量分析\tzhang liang fen xi\t3000\n"
                    "我也不知道\two ye bu zhi dao\t8000\n"
                    "朝阳门\tchao yang men\t1000\n朝阳门\tzhao yang men\t100\n"
                    "甲\tjia\t1\n")
            path.write_text(text, encoding="utf-8")
            before = path.read_bytes()
            result = audit.scan(root)
            self.assertEqual(path.read_bytes(), before)
            self.assertEqual(result["summary"]["all_records_scanned"], 6)
            self.assertEqual(result["summary"]["short_records_scanned"], 5)
            self.assertEqual(result["summary"]["short_unique_spellings"], 4)
            self.assertEqual(sum(row["phrase"] == "朝阳门" for row in result["records"]), 2)


if __name__ == "__main__":
    unittest.main()
