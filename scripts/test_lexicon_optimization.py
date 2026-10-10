"""Regression checks for category membership, migration coverage and weights."""
from pathlib import Path
from contextlib import contextmanager
import shutil
import uuid
import unittest
from unittest.mock import patch

import optimize_lexicons as optimize
from build_modern_it_lexicons import write as write_technology
from merge_zh_ext_lexicons import CATEGORIES, merge_category

ROOT = Path(__file__).resolve().parents[1]


@contextmanager
def fixture_directory():
    # Explicit workspace fixtures inherit Windows workspace permissions.
    target_root = (ROOT / "pinyin-ime/target").resolve()
    path = target_root / ("lexicon-test-" + uuid.uuid4().hex)
    path.mkdir()
    try:
        yield path
    finally:
        path.resolve().relative_to(target_root)
        shutil.rmtree(path)


class LexiconOptimizationTests(unittest.TestCase):
    def test_frequency_pool_cannot_fill_a_technology_category(self):
        with fixture_directory() as directory:
            output = Path(directory) / "tech.txt"
            write_technology(output, "technology", ["机器学习"], ("学习",),
                             [("妈妈", 1), ("吃饭", 2), ("我爱你", 3)])
            self.assertEqual([row[0] for row in optimize.read_rows(output)], ["机器学习"])

    def test_three_char_union_preserves_readings_and_first_source_weights(self):
        with fixture_directory() as directory:
            source = Path(directory) / "source"
            zh = Path(directory) / "zh"
            source.mkdir()
            zh.mkdir()
            (source / "life_common_3char_full.txt").write_text(
                "计算机\tji suan ji\t9200\n重庆市\tchong qing shi\t7000\n", encoding="utf-8")
            (source / "life_common_3char_hot.txt").write_text(
                "计算机\tji suan ji\t9999\n重庆市\tzhong qing shi\t100\n没问题\tmei wen ti\t8000\n", encoding="utf-8")
            with patch.object(optimize, "SOURCE", source), patch.object(optimize, "ZH", zh):
                self.assertEqual(optimize.consolidate_three_char(), 4)
            rows = optimize.read_rows(zh / "life_common_3char.txt")
            self.assertIn(("计算机", "ji suan ji", 9200), rows)
            self.assertIn(("重庆市", "zhong qing shi", 100), rows)
            self.assertIn(("没问题", "mei wen ti", 8000), rows)

    def test_published_categories_exclude_known_noise_and_keep_useful_phrases(self):
        for name, excluded, required in (
            ("technology.txt", {"妈妈", "吃饭", "我爱你", "云梦县"}, {"机器学习", "数据库", "大语言模型", "ChatGPT"}),
            ("daily_communication.txt", {"贝多芬", "运载火箭", "毛泽东"}, {"稍后回复你", "附件请查收", "辛苦了"}),
        ):
            rows = optimize.read_rows(ROOT / "lexicon/zh-ext" / name)
            words = {word for word, _, _ in rows}
            self.assertFalse(words & excluded)
            self.assertTrue(required <= words)
            self.assertTrue(all(0 < weight <= 10000 for _, _, weight in rows))

    def test_all_three_char_and_drug_readings_survive_consolidation(self):
        def keys(path):
            return {(word, code) for word, code, _ in optimize.read_rows(path)}
        source = ROOT / "data_sources/lexicon_fragments"
        expected = keys(source / "zh/life_common_3char_full.txt") | keys(source / "zh/life_common_3char_hot.txt")
        actual = keys(ROOT / "lexicon/zh/life_common_3char.txt")
        self.assertEqual(actual, expected)
        expected = keys(source / "zh-ext/yaowu.txt") | keys(source / "zh-ext/medicine_supplement.txt")
        self.assertEqual(keys(ROOT / "lexicon/zh-ext/medicine.txt"), expected)
        self.assertEqual(len(expected), 400)

    def test_category_regeneration_matches_published_contents(self):
        for name in ("technology.txt", "daily_communication.txt", "medicine.txt"):
            self.assertEqual(optimize.read_rows(ROOT / "lexicon/zh-ext" / name), merge_category(CATEGORIES[name]))


if __name__ == "__main__":
    unittest.main()
