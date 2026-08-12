#!/usr/bin/env python3
"""Build ranked three-character Simplified Chinese lexicons.

The first ``hot_size`` safe, word-like phrases are written to the base layer.
The remaining phrases are written to the large layer.  This matches the
existing loader profiles: Hot reads ``base`` while Standard/Full also read
``large``.

``wordfreq`` deliberately rounds low Zipf scores, which used to collapse most
of this list to weight 1.  We instead map the source's original global rank to
an inverse-square-root weight.  The mapping is deterministic and monotone for
the complete list, including its long tail.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import math
from pathlib import Path
from typing import Iterable, Sequence

from opencc import OpenCC
from pypinyin import Style, lazy_pinyin
from wordfreq import top_n_list


DEFAULT_SIZE = 20_000
DEFAULT_HOT_SIZE = 5_000
RANK_WEIGHT_SCALE = 6_000_000

# These fragments are unsuitable in either shipped layer.  Substring matching
# deliberately catches inflected profanity such as "你妈的" without trying to
# maintain every possible three-character spelling.
BLOCKED_SUBSTRINGS = frozenset(
    {
        "他妈",
        "你妈",
        "妈逼",
        "傻逼",
        "煞笔",
        "操你",
        "艹你",
        "草你",
        "狗日",
        "王八蛋",
    }
)

# wordfreq contains frequent connective snippets as standalone tokens.  They
# can still be useful for recall, so they are kept in the long tail but cannot
# consume a hot-head slot.  Keep this conservative and phrase-specific.
HOT_HEAD_FRAGMENT_DEMOTIONS = frozenset(
    {
        "不得不",
        "称之为",
        "从来不",
        "是因为",
        "无论是",
        "有利于",
        "有助于",
        "之所以",
    }
)


@dataclass(frozen=True)
class RankedPhrase:
    phrase: str
    source_rank: int


def is_three_cjk(text: str) -> bool:
    return len(text) == 3 and all("\u4e00" <= char <= "\u9fff" for char in text)


def is_blocked_phrase(phrase: str) -> bool:
    return any(fragment in phrase for fragment in BLOCKED_SUBSTRINGS)


def is_hot_head_fragment(phrase: str) -> bool:
    return phrase in HOT_HEAD_FRAGMENT_DEMOTIONS


def rank_weight(source_rank: int) -> int:
    """Map a one-based wordfreq rank to a positive monotone integer weight."""

    if source_rank < 1:
        raise ValueError("source_rank must be positive")
    return max(1, int(round(RANK_WEIGHT_SCALE / math.sqrt(source_rank))))


def collect_candidates(
    size: int = DEFAULT_SIZE,
    *,
    pool_size: int | None = None,
) -> list[RankedPhrase]:
    """Return safe simplified three-character phrases in wordfreq rank order."""

    if size < 1:
        raise ValueError("size must be positive")
    converter = OpenCC("t2s")
    candidates: list[RankedPhrase] = []
    seen: set[str] = set()
    source_pool_size = pool_size or max(150_000, size * 6)

    for source_rank, word in enumerate(top_n_list("zh", source_pool_size), 1):
        if (
            is_three_cjk(word)
            and converter.convert(word) == word
            and word not in seen
            and not is_blocked_phrase(word)
        ):
            seen.add(word)
            candidates.append(RankedPhrase(word, source_rank))
            if len(candidates) >= size:
                break

    if len(candidates) != size:
        raise RuntimeError(
            f"only found {len(candidates)} safe three-character words "
            f"in the top {source_pool_size} wordfreq entries"
        )
    return candidates


def partition_candidates(
    candidates: Sequence[RankedPhrase],
    hot_size: int = DEFAULT_HOT_SIZE,
) -> tuple[list[RankedPhrase], list[RankedPhrase]]:
    """Split candidates into a fragment-free hot head and a lossless long tail."""

    if not 0 <= hot_size <= len(candidates):
        raise ValueError("hot_size must be between zero and total candidate count")

    hot: list[RankedPhrase] = []
    for candidate in candidates:
        if len(hot) >= hot_size:
            break
        if not is_hot_head_fragment(candidate.phrase):
            hot.append(candidate)

    if len(hot) != hot_size:
        raise RuntimeError(f"only found {len(hot)} hot-head eligible phrases")
    hot_phrases = {candidate.phrase for candidate in hot}
    tail = [
        candidate for candidate in candidates if candidate.phrase not in hot_phrases
    ]
    return hot, tail


def validate_partition(
    candidates: Sequence[RankedPhrase],
    hot: Sequence[RankedPhrase],
    tail: Sequence[RankedPhrase],
    *,
    hot_size: int,
) -> None:
    """Fail before writing if layer membership or rank invariants drift."""

    if len(hot) != hot_size or len(tail) != len(candidates) - hot_size:
        raise RuntimeError(
            "invalid hot/tail sizes: "
            f"hot={len(hot)}, tail={len(tail)}, total={len(candidates)}"
        )
    candidate_phrases = [item.phrase for item in candidates]
    hot_phrases = {item.phrase for item in hot}
    tail_phrases = {item.phrase for item in tail}
    if len(candidate_phrases) != len(set(candidate_phrases)):
        raise RuntimeError("candidate phrases are not unique")
    if hot_phrases & tail_phrases:
        raise RuntimeError("hot and tail outputs overlap")
    if hot_phrases | tail_phrases != set(candidate_phrases):
        raise RuntimeError("hot and tail outputs do not cover all candidates")
    if any(is_blocked_phrase(phrase) for phrase in candidate_phrases):
        raise RuntimeError("blocked phrase reached output partition")
    if any(is_hot_head_fragment(phrase) for phrase in hot_phrases):
        raise RuntimeError("sentence fragment reached hot output")

    source_ranks = [item.source_rank for item in candidates]
    if source_ranks != sorted(source_ranks) or len(source_ranks) != len(
        set(source_ranks)
    ):
        raise RuntimeError("candidate source ranks must be unique and increasing")
    weights = [rank_weight(rank) for rank in source_ranks]
    if weights != sorted(weights, reverse=True):
        raise RuntimeError("rank weights are not monotone")


def render_reading(phrase: str) -> str:
    return " ".join(lazy_pinyin(phrase, style=Style.NORMAL))


def write_lexicon(
    output: Path,
    candidates: Iterable[RankedPhrase],
    *,
    label: str,
    total_size: int,
) -> int:
    rows = list(candidates)
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("w", encoding="utf-8", newline="\n") as stream:
        stream.write(f"# 开心输入法原生词库：{len(rows)} 个热门三字词（{label}）\n")
        stream.write(
            f"# Source: wordfreq 中文原始全局排名；完整集合共 {total_size} 条。\n"
        )
        stream.write(
            "# 格式：词语<TAB>全拼<TAB>词频；"
            "词频 = round(6000000 / sqrt(wordfreq_rank))，按原始排名单调递减。\n"
        )
        if label == "热头":
            stream.write("# 粗俗词已剔除；明显句段碎片降级到长尾。\n")
        else:
            stream.write("# 粗俗词已剔除；热头降级的句段碎片保留在本层。\n")
        for candidate in rows:
            stream.write(
                f"{candidate.phrase}\t{render_reading(candidate.phrase)}\t"
                f"{rank_weight(candidate.source_rank)}\n"
            )
    return len(rows)


def default_tail_output_for(hot_output: Path) -> Path:
    if hot_output.parent.name == "zh" and hot_output.parent.parent.name == "lexicon":
        return hot_output.parent / "life_common_3char_tail_15000.txt"
    return hot_output.with_name(f"{hot_output.stem}_tail.txt")


def build(
    output: Path,
    size: int = DEFAULT_SIZE,
    *,
    hot_size: int = DEFAULT_HOT_SIZE,
    tail_output: Path | None = None,
    pool_size: int | None = None,
) -> tuple[int, int]:
    """Build both layers, preserving the historical ``output`` argument."""

    if size < 1:
        raise ValueError("size must be positive")
    if not 0 <= hot_size <= size:
        raise ValueError("hot_size must be between zero and size")
    resolved_tail = tail_output or default_tail_output_for(output)
    if output.resolve() == resolved_tail.resolve():
        raise ValueError("hot and tail outputs must be different files")

    candidates = collect_candidates(size, pool_size=pool_size)
    hot, tail = partition_candidates(candidates, hot_size)
    validate_partition(candidates, hot, tail, hot_size=hot_size)
    hot_count = write_lexicon(output, hot, label="热头", total_size=size)
    tail_count = write_lexicon(
        resolved_tail,
        tail,
        label="长尾",
        total_size=size,
    )
    return hot_count, tail_count


def main() -> int:
    repo = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output",
        "--hot-output",
        dest="hot_output",
        type=Path,
        default=repo / "lexicon" / "zh" / "life_common_3char_20000.txt",
        help=(
            "hot-head output; --output is retained for compatibility with the "
            "previous single-file generator"
        ),
    )
    parser.add_argument(
        "--tail-output",
        type=Path,
        default=repo / "lexicon" / "zh" / "life_common_3char_tail_15000.txt",
    )
    parser.add_argument("--size", type=int, default=DEFAULT_SIZE)
    parser.add_argument("--hot-size", type=int, default=DEFAULT_HOT_SIZE)
    parser.add_argument(
        "--pool-size",
        type=int,
        help="wordfreq source pool size (mainly useful for deterministic tests)",
    )
    args = parser.parse_args()

    hot_count, tail_count = build(
        args.hot_output,
        args.size,
        hot_size=args.hot_size,
        tail_output=args.tail_output,
        pool_size=args.pool_size,
    )
    print(
        f"wrote hot={hot_count} to {args.hot_output}; "
        f"tail={tail_count} to {args.tail_output}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
