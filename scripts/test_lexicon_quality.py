"""Regression gates for sentence cleanup and source regeneration."""
from pathlib import Path
import json
import unittest

import lexicon_quality as quality


class LexiconQualityTests(unittest.TestCase):
    def test_second_review_keeps_real_terms_quantities_and_complete_expressions(self):
        for word in ("唯一性集", "成均馆", "城六区", "葛伦霍", "一干人", "万标箱", "万架次",
                     "每万人", "一百只", "第一分", "一零年", "谱分析", "张量分析", "邦政府",
                     "斯政府", "马行空", "言尽意", "越来越", "愈来愈", "战兢兢"):
            self.assertIsNone(quality.phrase_rejection_reason(word), word)
        for word in ("控系统", "益物权", "周而复", "机号码", "慈禧太", "荣八耻", "的花海", "越做越"):
            self.assertIsNotNone(quality.phrase_rejection_reason(word), word)
        for word in ("火控系统", "数控系统", "用益物权", "周而复始", "三权分立", "慈禧太后",
                     "八荣八耻", "越做越好", "油菜花海", "计算机系统"):
            self.assertIsNone(quality.phrase_rejection_reason(word), word)

    def test_full_spelling_repairs_preserve_meaning_and_correct_the_reading(self):
        rows = quality.clean_rows([
            ("耳聋眼黒", "er long yan hei", 2522),
            ("耳聋眼黑", "er long yan hei", 3000),
            ("含辛茹若", "han xin ru ruo", 2522),
            ("名列前矛", "ming lie qian mao", 2522),
            ("情非得以", "qing fei de yi", 2522),
            ("两眼一摸黑", "liang yan yi mo hei", 3000),
        ])
        self.assertEqual(rows, [
            ("耳聋眼黑", "er long yan hei", 3000),
            ("含辛茹苦", "han xin ru ku", 2522),
            ("名列前茅", "ming lie qian mao", 2522),
            ("情非得已", "qing fei de yi", 2522),
            ("两眼一抹黑", "liang yan yi mo hei", 3000),
        ])
        self.assertEqual(quality.canonical_entry("耳聋眼黒色", "er long yan hei se"),
                         ("耳聋眼黒色", "er long yan hei se"))

    def test_short_fragment_review_covers_three_four_and_five_characters(self):
        for phrase in ("两国均", "检查和", "让你在", "中美关", "半人半",
                       "言简意", "牛头不对", "计算机网", "一失足成", "一分钱一",
                       "以小人之心", "以及人之老", "文件名扩展", "成千成万条"):
            self.assertEqual(quality.phrase_rejection_reason(phrase), "reviewed_lexical_fragment", phrase)
        for phrase in quality.SHORT_POLICY["preserved_phrases"]:
            self.assertIsNone(quality.phrase_rejection_reason(phrase), phrase)
        # Only the reviewed spelling is rejected. Longer real terms and sentences
        # that happen to contain a fragment must not be removed by substring.
        for phrase in ("文件名扩展名", "牛头不对马嘴", "有志者事竟成", "中美关系",
                       "一失足成千古恨", "两国均已签署协议", "言简意赅", "八荣八耻"):
            self.assertIsNone(quality.phrase_rejection_reason(phrase), phrase)

    def test_short_fragments_are_filtered_on_every_import_without_changing_good_rows(self):
        rows = [("两国均", "liang guo jun", 9000), ("牛头不对", "niu tou bu dui", 8000),
                ("文件名扩展", "wen jian ming kuo zhan", 7000),
                ("对不起", "dui bu qi", 9999), ("心平气和", "xin ping qi he", 6500),
                ("我已经到了", "wo yi jing dao le", 6000)]
        self.assertEqual(quality.clean_rows(rows), rows[3:])

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
