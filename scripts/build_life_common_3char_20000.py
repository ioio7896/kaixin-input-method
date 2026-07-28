#!/usr/bin/env python3
"""Build the 20,000 most frequent three-character Simplified Chinese words."""

from __future__ import annotations

import argparse
from pathlib import Path

from opencc import OpenCC
from pypinyin import Style, lazy_pinyin
from wordfreq import top_n_list, zipf_frequency


def is_three_cjk(text: str) -> bool:
    return len(text) == 3 and all("\u4e00" <= ch <= "\u9fff" for ch in text)


def build(output: Path, size: int = 20_000) -> None:
    converter = OpenCC("t2s")
    candidates: list[str] = []
    seen: set[str] = set()
    pool_size = max(150_000, size * 5)

    for word in top_n_list("zh", pool_size):
        if (
            is_three_cjk(word)
            and converter.convert(word) == word
            and word not in seen
        ):
            seen.add(word)
            candidates.append(word)
            if len(candidates) >= size:
                break

    if len(candidates) != size:
        raise RuntimeError(f"only found {len(candidates)} three-character words")

    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("w", encoding="utf-8", newline="\n") as stream:
        stream.write(f"# 开心输入法原生词库：{size} 个热门三字词\n")
        stream.write(
            f"# Source: wordfreq 中文频率排序（取前 {size} 个三字简体中文词）。\n"
        )
        stream.write(
            "# 格式：词语<TAB>全拼<TAB>词频；"
            "词频为 10^Zipf 估计值，保持相对频率顺序。\n"
        )
        for word in candidates:
            pinyin = " ".join(lazy_pinyin(word, style=Style.NORMAL))
            weight = max(1, int(round(10 ** zipf_frequency(word, "zh"))))
            stream.write(f"{word}\t{pinyin}\t{weight}\n")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(__file__).resolve().parents[1]
        / "lexicon"
        / "base"
        / "life_common_3char_20000.txt",
    )
    parser.add_argument("--size", type=int, default=20_000)
    args = parser.parse_args()
    build(args.output, args.size)


if __name__ == "__main__":
    main()
