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
from lexicon_quality import corrected_reading, phrase_rejection_reason


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

    # Preserve the original source window; reviewed deletions are not a quota
    # to refill with other unreviewed tail words.
    candidates = [word for word in candidates if not phrase_rejection_reason(word)]

    output.parent.mkdir(parents=True, exist_ok=True)
    from normalize_lexicon_freq10k import score_by_rank
    weights = score_by_rank([max(1, int(round(10 ** zipf_frequency(word, "zh"))))
                             for word in candidates], 1, 10_000)
    with output.open("w", encoding="utf-8", newline="\n") as stream:
        stream.write(f"# 开心输入法原生词库：{len(candidates)} 个生活常用双字词\n")
        stream.write(f"# Source: wordfreq 中文频率排序（取前 {size} 个两字简体中文词）。\n")
        stream.write("# 格式：词语<TAB>全拼<TAB>1-10000 分数；由来源频率排序归一化。\n")
        for word, weight in zip(candidates, weights):
            pinyin = corrected_reading(word, " ".join(lazy_pinyin(word, style=Style.NORMAL)))
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
