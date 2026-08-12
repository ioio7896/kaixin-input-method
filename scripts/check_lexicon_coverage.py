#!/usr/bin/env python3
"""Validate lexicon coverage: phrase-length distribution, initial coverage,
and character-level coverage of the pinyin-supported character set."""
from __future__ import annotations

import argparse
import sys
from collections import Counter
from pathlib import Path


def resolve_repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def _chinese_char_count(term: str) -> int:
    return sum(not ch.isspace() and not (ch.isascii() and ch.isalpha()) for ch in term)


def _has_cjk(term: str) -> bool:
    return any(
        "一" <= ch <= "鿿"
        or "㐀" <= ch <= "䶿"
        or "豈" <= ch <= "﫿"
        or "\U00020000" <= ch <= "\U0002ebef"
        for ch in term
    )


def _is_mixed_entity(phrase: str) -> bool:
    return any(ch.isascii() and (ch.isalpha() or ch.isdigit()) for ch in phrase)


def _first_syllable_initial(code: str) -> str:
    """Extract the first pinyin initial from a code like 'zhong guo'."""
    token = code.strip().split()[0] if code.strip() else ""
    # Map common two-letter initials
    for initial in ("zh", "ch", "sh"):
        if token.startswith(initial):
            return initial
    if token:
        return token[0]
    return ""


def _iter_lexicon_lines(path: Path):
    with path.open("r", encoding="utf-8-sig", errors="replace") as f:
        for line in f:
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue
            yield line.rstrip("\n")


def check_coverage(
    lexicon_dir: Path,
    supported_chars_path: Path | None,
    *,
    min_per_length: dict[int, int] | None = None,
    min_per_initial: int = 100,
) -> tuple[int, list[str]]:
    """Run coverage checks. Returns (exit_code, warnings)."""
    if min_per_length is None:
        min_per_length = {
            1: 100,
            2: 5000,
            3: 2000,
            4: 500,
            5: 200,
            6: 100,
            7: 50,
            8: 30,
        }

    warnings: list[str] = []
    length_counts: Counter[int] = Counter()
    initial_counts: Counter[str] = Counter()
    all_chars: set[str] = set()

    for path in sorted(lexicon_dir.rglob("*.txt")):
        if "en" in {p.casefold() for p in path.relative_to(lexicon_dir).parts}:
            continue
        for line in _iter_lexicon_lines(path):
            parts = line.split("\t")
            term = parts[0].strip()
            if not term or not _has_cjk(term) or _is_mixed_entity(term):
                continue
            char_count = _chinese_char_count(term)
            length_counts[char_count] += 1

            # Track initials for pinyin coverage
            if len(parts) >= 2:
                code = parts[1].strip()
                initial = _first_syllable_initial(code)
                if initial:
                    initial_counts[initial] += 1

            for ch in term:
                if "一" <= ch <= "鿿":
                    all_chars.add(ch)

    # Check length coverage
    for length, minimum in min_per_length.items():
        actual = length_counts.get(length, 0)
        if actual < minimum:
            warnings.append(
                f"phrase length {length}: {actual} entries (minimum: {minimum})"
            )

    # Check initial consonant coverage
    expected_initials = set("bpmfdtnlgkhjqxrzcsyw")
    for initial in sorted(expected_initials):
        actual = initial_counts.get(initial, 0)
        if actual < min_per_initial:
            warnings.append(
                f"initial '{initial}': {actual} entries (minimum: {min_per_initial})"
            )

    # Check character coverage if supported_chars_path provided
    char_warnings: list[str] = []
    if supported_chars_path and supported_chars_path.is_file():
        supported = {
            line.strip()
            for line in supported_chars_path.read_text(encoding="utf-8-sig").splitlines()
            if line.strip()
        }
        missing_from_lexicon = supported - all_chars
        if missing_from_lexicon:
            missing_sample = sorted(missing_from_lexicon)[:20]
            char_warnings.append(
                f"{len(missing_from_lexicon)} supported chars have zero lexicon entries "
                f"(sample: {''.join(missing_sample)})"
            )

    exit_code = 1 if warnings or char_warnings else 0

    # Print results
    print(f"Phrase length distribution: {dict(sorted(length_counts.items()))}")
    print(f"Initial coverage: {dict(sorted(initial_counts.items()))}")
    print(f"Unique CJK chars in lexicon: {len(all_chars)}")
    if supported_chars_path:
        print(f"Supported chars with coverage: {len(all_chars)}")

    for w in warnings:
        print(f"WARNING: {w}")
    for w in char_warnings:
        print(f"WARNING: {w}")

    if exit_code == 0:
        print("Coverage check passed")
    else:
        print("Coverage check: some thresholds not met (see warnings above)")

    return exit_code, warnings + char_warnings


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Check lexicon phrase-length and initial-consonant coverage.",
    )
    parser.add_argument("--root", type=Path, default=None, help="repository root")
    parser.add_argument(
        "--check-chars",
        action="store_true",
        help="also verify every supported character has at least one lexicon entry",
    )
    return parser.parse_args()


def main() -> int:
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, OSError, TypeError, ValueError):
            pass

    args = parse_args()
    root = (args.root or resolve_repo_root()).resolve()
    lexicon_dir = root / "lexicon"
    chars_path = None
    if args.check_chars:
        chars_path = root / "pinyin-ime" / "data" / "pinyin_supported_chars.txt"

    if not lexicon_dir.is_dir():
        print(f"missing lexicon directory: {lexicon_dir}")
        return 2

    exit_code, _warnings = check_coverage(lexicon_dir, chars_path)
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
