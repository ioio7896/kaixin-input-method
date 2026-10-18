#!/usr/bin/env python3
"""Generate the 8,105-entry common single-Chinese-character lexicon.

Keep the highest-frequency 500 standalone characters stable. Rank the rest by
their appearances in the checked-in two-character lexicon, using the
three-character lexicon and original standalone order to break ties. The output uses
the native two-column format so all supported readings remain available.
"""

from __future__ import annotations

import argparse
from collections import defaultdict
from importlib.metadata import version
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "lexicon" / "zh" / "single_char_common_8105.txt"
SUPPORTED_CHARS = ROOT / "pinyin-ime" / "data" / "pinyin_supported_chars.txt"
SOURCE_ORDER = ROOT / "data_sources" / "kaixin" / "single_char_wordfreq_order.txt"
CONTEXT_LEXICON = ROOT / "lexicon" / "zh" / "life_common_2char_125000.txt"
TIEBREAK_LEXICON = ROOT / "lexicon" / "zh" / "life_common_3char.txt"
WORDFREQ_VERSION = "3.1.1"
CHARACTER_COUNT = 8_105
SOURCE_WORD_LIMIT = 500_000
STANDALONE_HEAD_COUNT = 500


def is_han_character(value: str) -> bool:
    if len(value) != 1:
        return False
    codepoint = ord(value)
    return (
        0x3400 <= codepoint <= 0x4DBF
        or 0x4E00 <= codepoint <= 0x9FFF
        or 0xF900 <= codepoint <= 0xFAFF
        or 0x20000 <= codepoint <= 0x2EBEF
        or 0x30000 <= codepoint <= 0x323AF
    )


def load_supported_characters(path: Path) -> set[str]:
    return {
        line.strip()
        for line in path.read_text(encoding="utf-8-sig").splitlines()
        if len(line.strip()) == 1
    }


def wordfreq_characters(supported: set[str]) -> list[str]:
    from wordfreq import top_n_list

    selected: list[str] = []
    seen: set[str] = set()
    for value in top_n_list("zh", SOURCE_WORD_LIMIT):
        if value in seen or value not in supported or not is_han_character(value):
            continue
        seen.add(value)
        selected.append(value)
        if len(selected) == CHARACTER_COUNT:
            return selected
    raise RuntimeError(
        f"wordfreq yielded only {len(selected)} supported Han characters; "
        f"need {CHARACTER_COUNT}"
    )


def load_source_order(supported: set[str]) -> list[str]:
    characters = [
        line.strip()
        for line in SOURCE_ORDER.read_text(encoding="utf-8-sig").splitlines()
        if line.strip() and not line.startswith("#")
    ]
    if (
        len(characters) != CHARACTER_COUNT
        or len(set(characters)) != CHARACTER_COUNT
        or any(not is_han_character(ch) or ch not in supported for ch in characters)
    ):
        raise ValueError(f"invalid standalone-character source: {SOURCE_ORDER}")
    return characters


def contextual_scores(path: Path, word_length: int) -> dict[str, int]:
    scores: dict[str, int] = defaultdict(int)
    rank = 0
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        if not line or line.startswith("#"):
            continue
        word = line.split("\t", 1)[0]
        if len(word) != word_length:
            raise ValueError(f"expected {word_length}-character word in {path}: {line!r}")
        rank += 1
        # Rank decay limits the influence of the long tail and proper names.
        contribution = 1_000_000 // (rank + 100)
        for ch in set(word):
            scores[ch] += contribution
    if rank == 0:
        raise ValueError(f"empty contextual lexicon: {path}")
    return scores


def ranked_characters(supported: set[str]) -> list[str]:
    standalone = load_source_order(supported)
    source_rank = {ch: index for index, ch in enumerate(standalone)}
    context = contextual_scores(CONTEXT_LEXICON, 2)
    tiebreak = contextual_scores(TIEBREAK_LEXICON, 3)
    return standalone[:STANDALONE_HEAD_COUNT] + sorted(
        standalone[STANDALONE_HEAD_COUNT:],
        key=lambda ch: (-context.get(ch, 0), -tiebreak.get(ch, 0), source_rank[ch]),
    )


def rank_weight(index: int) -> int:
    """Map frequency rank deterministically onto the native 1..10000 scale."""

    if not 0 <= index < CHARACTER_COUNT:
        raise ValueError(f"rank index outside generated lexicon: {index}")
    return 10_000 - (index * 9_999 // (CHARACTER_COUNT - 1))


def render(characters: list[str]) -> str:
    lines = [
        "# 开心输入法 8105 个常用单字词库",
        "# 排序：前 500 字沿用 wordfreq 3.1.1 单字顺序；其余按双字词中的出现名次加权。",
        "# 同分依次参考三字词中的出现名次和原单字顺序；仅保留拼音引擎支持的汉字。",
        "# 权重：按排序名次映射到 10000..1，严格降序；不代表实际频率数值。",
        "# 格式：汉字<TAB>权重；拼音由引擎自动生成，以保留多音字全部读音。",
    ]
    lines.extend(f"{character}\t{rank_weight(index)}" for index, character in enumerate(characters))
    return "\n".join(lines) + "\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument(
        "--refresh-source",
        action="store_true",
        help="refresh the pinned standalone order from wordfreq 3.1.1",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    supported = load_supported_characters(SUPPORTED_CHARS)
    if args.refresh_source:
        installed = version("wordfreq")
        if installed != WORDFREQ_VERSION:
            raise RuntimeError(
                f"wordfreq {WORDFREQ_VERSION} is required to refresh the source; "
                f"found {installed}"
            )
        source = wordfreq_characters(supported)
        SOURCE_ORDER.write_text(
            "# wordfreq 3.1.1 Chinese single-character order before contextual reranking.\n"
            "# One pinyin-supported Han character per line; generated by "
            "scripts/generate_single_char_lexicon.py --refresh-source.\n"
            + "\n".join(source)
            + "\n",
            encoding="utf-8",
            newline="\n",
        )
    characters = ranked_characters(supported)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(render(characters), encoding="utf-8", newline="\n")
    print(
        f"generated {len(characters)} single-character entries: {args.output} "
        f"({characters[0]}..{characters[-1]})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
