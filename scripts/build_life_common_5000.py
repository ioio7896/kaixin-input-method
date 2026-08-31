#!/usr/bin/env python3
"""Build the most common two-character Chinese daily words.

The wordfreq package supplies the frequency ordering and Zipf scores.  We
keep only two simplified-CJK-character entries, then emit the native lexicon
format: phrase, space-separated full pinyin, integer weight.
"""

from __future__ import annotations

import argparse
from pathlib import Path

from pypinyin import Style, lazy_pinyin
from wordfreq import top_n_list, zipf_frequency


def is_two_cjk(text: str) -> bool:
    return len(text) == 2 and all("\u4e00" <= ch <= "\u9fff" for ch in text)


def build(output: Path, size: int = 125_000) -> None:
    # A generous pool is needed because top_n_list also contains one-character
    # words, Latin terms, numbers, and longer phrases.
    candidates: list[str] = []
    seen: set[str] = set()
    pool_size = max(250_000, size * 3)
    for word in top_n_list("zh", pool_size):
        if is_two_cjk(word) and word not in seen:
            seen.add(word)
            candidates.append(word)
            if len(candidates) >= size:
                break
    if len(candidates) != size:
        raise RuntimeError(f"only found {len(candidates)} two-character words")

    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("w", encoding="utf-8", newline="\n") as stream:
        stream.write(f"# 开心输入法原生词库：{size} 个生活常用双字词\n")
        stream.write(f"# Source: wordfreq 中文频率排序（取前 {size} 个两字简体中文词）。\n")
        stream.write("# 格式：词语<TAB>全拼<TAB>词频；词频为 10^Zipf 估计值，保持相对频率顺序。\n")
        for word in candidates:
            pinyin = " ".join(lazy_pinyin(word, style=Style.NORMAL))
            zipf = zipf_frequency(word, "zh")
            # Zipf is log10 occurrences per billion; retaining that scale gives
            # useful integer weights (roughly 1e4--1e7 for this list).
            weight = max(1, int(round(10 ** zipf)))
            stream.write(f"{word}\t{pinyin}\t{weight}\n")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(__file__).resolve().parents[1]
        / "lexicon"
        / "zh"
        / "life_common_2char_125000.txt",
    )
    parser.add_argument("--size", type=int, default=125_000)
    args = parser.parse_args()
    build(args.output, args.size)


if __name__ == "__main__":
    main()
