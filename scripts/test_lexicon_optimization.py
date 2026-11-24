"""Regression checks for category membership, migration coverage and weights."""

from pathlib import Path
from contextlib import contextmanager
import shutil
import uuid
import unittest
from unittest.mock import patch

import optimize_lexicons as optimize
from build_modern_it_lexicons import write as write_technology
from build_short_hot_lexicons import build_one

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
            write_technology(
                output,
                "technology",
                ["机器学习"],
                ("学习",),
                [("妈妈", 1), ("吃饭", 2), ("我爱你", 3)],
            )
            self.assertEqual([row[0] for row in optimize.read_rows(output)], ["机器学习"])

    def test_three_char_union_preserves_readings_and_first_source_weights(self):
        with fixture_directory() as directory:
            source = Path(directory) / "source"
            zh = Path(directory) / "zh"
            source.mkdir()
            zh.mkdir()
            (source / "life_common_3char_full.txt").write_text(
                "计算机\tji suan ji\t9200\n重庆市\tchong qing shi\t7000\n", encoding="utf-8"
            )
            (source / "life_common_3char_hot.txt").write_text(
                "计算机\tji suan ji\t9999\n重庆市\tzhong qing shi\t100\n没问题\tmei wen ti\t8000\n",
                encoding="utf-8",
            )
            with (
                patch.object(optimize, "SOURCE", source),
                patch.object(optimize, "ZH", zh),
                patch("lexicon_maintenance.hot_sources", return_value=([], [])),
            ):
                self.assertEqual(optimize.consolidate_three_char(), 4)
            rows = optimize.read_rows(zh / "life_common_3char.txt")
            self.assertIn(("计算机", "ji suan ji", 9200), rows)
            self.assertIn(("重庆市", "zhong qing shi", 100), rows)
            self.assertIn(("没问题", "mei wen ti", 8000), rows)

    def test_published_categories_exclude_known_noise_and_keep_useful_phrases(self):
        for name, excluded, required in (
            (
                "technology.txt",
                {"妈妈", "吃饭", "我爱你", "云梦县"},
                {"机器学习", "数据库", "大语言模型", "ChatGPT"},
            ),
            (
                "daily_communication.txt",
                {"贝多芬", "运载火箭", "毛泽东"},
                {"稍后回复你", "附件请查收", "辛苦了"},
            ),
        ):
            rows = optimize.read_rows(ROOT / "lexicon/zh-ext" / name)
            words = {word for word, _, _ in rows}
            self.assertFalse(words & excluded)
            self.assertTrue(required <= words)
            self.assertTrue(all(0 < weight <= 10000 for _, _, weight in rows))

    def test_three_char_regeneration_matches_published_readings_and_weights(self):
        with fixture_directory() as directory, patch.object(optimize, "ZH", directory):
            optimize.consolidate_three_char()
            actual = optimize.read_rows(directory / "life_common_3char.txt")
        expected = optimize.read_rows(ROOT / "lexicon/zh/life_common_3char.txt")
        self.assertEqual(actual, expected)
        self.assertIn(("给排水", "ji pai shui", 3936), actual)
        self.assertIn(("他妈的", "ta ma de", 1200), actual)
        self.assertIn(("短视频", "duan shi pin", 7600), actual)

    def test_core_overlay_rejects_blocked_curated_rows_and_refills(self):
        with fixture_directory() as directory:
            output = directory / "life_hot_3char_curated.txt"
            source = directory / "life_common_3char.txt"
            output.write_text(
                "他妈的\tta ma de\t9000\n没问题\tmei wen ti\t8600\n", encoding="utf-8"
            )
            source.write_text(
                "草泥马\tcao ni ma\t9900\n计算机\tji suan ji\t9800\n", encoding="utf-8"
            )
            build_one(output, (source,), 2, 2, 3, 8400, 9000, "三字精选")
            self.assertEqual(
                optimize.read_rows(output),
                [("没问题", "mei wen ti", 8600), ("计算机", "ji suan ji", 9000)],
            )


if __name__ == "__main__":
    unittest.main()
