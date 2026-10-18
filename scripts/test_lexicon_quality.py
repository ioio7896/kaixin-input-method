"""Regression gates for sentence cleanup and source regeneration."""
from pathlib import Path
import json
import unittest

import lexicon_quality as quality


class LexiconQualityTests(unittest.TestCase):
    def test_reviewed_fragment_rejection_is_exact_and_preserves_complete_phrases(self):
        for word in ("的并", "人际关", "绰绰有", "令狐冲微微", "之一百二十",
                     "显而易", "精疲力", "心平气", "牵一发而", "慕容复笑"):
            self.assertEqual(quality.phrase_rejection_reason(word), "reviewed_lexical_fragment")
        for word in ("人际关系", "绰绰有余", "石破天惊", "令狐冲", "令狐冲公子", "一发不可收", "于都",
                     "显而易见", "精疲力尽", "心平气和", "令狐绝"):
            self.assertIsNone(quality.phrase_rejection_reason(word), word)

    def test_archived_fragments_cannot_return_during_category_regeneration(self):
        self.assertEqual(quality.clean_rows([
            ("人际关", "ren ji guan", 8000),
            ("人际关系", "ren ji guan xi", 7000),
            ("鹏笑道", "peng xiao dao", 3000),
            ("石破天惊", "shi po tian jing", 6000),
        ]), [("人际关系", "ren ji guan xi", 7000), ("石破天惊", "shi po tian jing", 6000)])

    def test_phrase_generator_emits_native_scores_without_a_second_normalization(self):
        from build_common_phrase_lexicons import make_rows
        rows = make_rows([("学习资料", 1), ("工作安排", 100), ("会议记录", 10000)], 9_000_000)
        scores = [int(row.split("\t")[2]) for row in rows]
        self.assertEqual(scores[0], 10000)
        self.assertEqual(scores[-1], 1)
        self.assertGreater(scores[0], scores[1])
        self.assertGreater(scores[1], scores[2])

    def test_rejects_bad_collocations_without_rejecting_natural_requests(self):
        for phrase in ("你现在先去文件", "我明天先去事情", "请发送文件一下", "麻烦帮我准备申请材料一下"):
            self.assertIsNotNone(quality.phrase_rejection_reason(phrase), phrase)
        for phrase in ("我先去医院", "请看一下文件", "帮我看一下", "请尽快处理一下", "麻烦发送申请材料"):
            self.assertIsNone(quality.phrase_rejection_reason(phrase), phrase)

    def test_reading_corrections_are_exact_and_preserve_legitimate_alternates(self):
        self.assertEqual(quality.corrected_reading("还款", "hai kuan"), "huan kuan")
        self.assertEqual(quality.corrected_reading("校史", "jiao shi"), "xiao shi")
        self.assertEqual(quality.corrected_reading("穿着", "chuan zhe"), "chuan zhe")
        self.assertEqual(quality.corrected_reading("朝阳", "chao yang"), "chao yang")
        self.assertEqual(quality.corrected_reading("朝阳", "zhao yang"), "zhao yang")

    def test_import_cleanup_keeps_useful_requests_and_merges_corrected_duplicates(self):
        cleaned = quality.clean_rows([
            ("请发送文件一下", "qing fa song wen jian yi xia", 4900),
            ("请发送文件", "qing fa song wen jian", 4900),
            ("都匀", "dou yun", 3000),
            ("都匀", "du yun", 6500),
        ])
        self.assertEqual(cleaned, [("请发送文件", "qing fa song wen jian", 4900), ("都匀", "du yun", 6500)])

    def test_runtime_sources_pass_quality_and_keep_useful_expressions(self):
        self.assertEqual(quality.check_quality(quality.ROOT), [])
        phrases = {p for p, r, s in quality.reviewed_long_rows()}
        self.assertTrue({"最近怎么样", "路上注意安全", "请准时参加会议", "附件请查收"} <= phrases)
        # Natural everyday expressions remain in their independently switchable
        # category even when unreviewed combinations leave the base lexicon.
        life = (quality.ROOT / "lexicon/zh-ext/category_life_daily.txt").read_text(encoding="utf-8")
        self.assertIn("我刚看到消息\t", life)
        self.assertIn("我去门口拿个包裹\t", life)

    def test_regenerated_categories_match_published_rows_and_manifest(self):
        import hashlib
        from merge_zh_ext_lexicons import CATEGORIES, merge_category
        manifest = json.loads((quality.ROOT / "shared/category_lexicons.json").read_text(encoding="utf-8"))
        for item in manifest["categories"]:
            path = quality.ROOT / "lexicon/zh-ext" / item["file"]
            data = path.read_bytes()
            rows = [tuple(line.split("\t")) for line in data.decode("utf-8").splitlines() if line and not line.startswith("#")]
            self.assertEqual([(p, r, int(s)) for p, r, s in rows], merge_category(CATEGORIES[item["file"]]), item["file"])
            self.assertEqual(len(rows), item["rows"])
            self.assertEqual(hashlib.sha256(data).hexdigest(), item["sha256"])

    def test_short_hot_lists_respect_their_length_contract(self):
        for name, length in (("life_short_hot_2char.txt", 2), ("life_hot_3char_curated.txt", 3), ("life_hot_4char_curated.txt", 4)):
            text = (quality.ROOT / "lexicon/zh" / name).read_text(encoding="utf-8")
            for line in text.splitlines():
                if line and not line.startswith("#"):
                    self.assertEqual(len(line.split("\t")[0]), length, line)


if __name__ == "__main__":
    unittest.main()
