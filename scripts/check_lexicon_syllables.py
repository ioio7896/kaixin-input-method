#!/usr/bin/env python3
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


PINYIN_TOKEN_RE = re.compile(r"[A-Za-z]+")
SKIP_LEXICON_GROUPS = {"en"}


def resolve_repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def load_syllables(path: Path) -> set[str]:
    return {
        line.strip().lower()
        for line in path.read_text(encoding="utf-8-sig").splitlines()
        if line.strip()
    }


def iter_chinese_lexicon_files(lexicon_dir: Path):
    for path in sorted(lexicon_dir.rglob("*.txt"), key=lambda p: p.as_posix().lower()):
        rel_parts = set(path.relative_to(lexicon_dir).parts)
        if rel_parts & SKIP_LEXICON_GROUPS:
            continue
        yield path


def iter_lexicon_lines(path: Path):
    with path.open("r", encoding="utf-8-sig", errors="replace") as f:
        for line_no, line in enumerate(f, 1):
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue
            yield line_no, line.rstrip("\n")


def _chinese_char_count(phrase: str) -> int:
    """Count Chinese characters in a phrase (excluding whitespace and ASCII)."""
    count = 0
    for ch in phrase:
        if ch.isspace():
            continue
        if "一" <= ch <= "鿿" or "㐀" <= ch <= "䶿":
            count += 1
        # Also count rare/historical CJK
        elif (
            "豈" <= ch <= "﫿"
            or "\U00020000" <= ch <= "\U0002a6df"
            or "\U0002a700" <= ch <= "\U0002ebef"
        ):
            count += 1
    return count


def _is_mixed_entity(phrase: str) -> bool:
    """True when phrase contains ASCII letters/digits (e.g. ChatGPT, B站)."""
    return any(ch.isascii() and (ch.isalpha() or ch.isdigit()) for ch in phrase)


def scan_unknown_syllables(lexicon_dir: Path, syllables: set[str]) -> dict[str, list[tuple[Path, int, str, str]]]:
    unknown: dict[str, list[tuple[Path, int, str, str]]] = {}
    for path in iter_chinese_lexicon_files(lexicon_dir):
        for line_no, line in iter_lexicon_lines(path):
            fields = [field.strip() for field in line.split("\t") if field.strip()]
            if len(fields) < 2:
                continue
            phrase, code = fields[0], fields[1]
            if _is_mixed_entity(phrase):
                continue
            if not any(ch.isascii() and ch.isalpha() for ch in code):
                continue
            for match in PINYIN_TOKEN_RE.finditer(code.lower()):
                token = match.group(0)
                if token and token not in syllables:
                    unknown.setdefault(token, []).append((path, line_no, phrase, code))
    return unknown


def scan_syllable_count_mismatches(
    lexicon_dir: Path,
) -> list[tuple[Path, int, str, str, int, int]]:
    """Find entries where pinyin syllable count != Chinese character count.

    Returns list of (path, line_no, phrase, code, char_count, syllable_count).
    """
    mismatches: list[tuple[Path, int, str, str, int, int]] = []
    for path in iter_chinese_lexicon_files(lexicon_dir):
        for line_no, line in iter_lexicon_lines(path):
            fields = [field.strip() for field in line.split("\t") if field.strip()]
            if len(fields) < 2:
                continue
            phrase, code = fields[0], fields[1]
            if _is_mixed_entity(phrase):
                continue
            if not any(ch.isascii() and ch.isalpha() for ch in code):
                continue
            char_count = _chinese_char_count(phrase)
            if char_count == 0:
                continue
            syllable_tokens = PINYIN_TOKEN_RE.findall(code.lower())
            syllable_count = len(syllable_tokens)
            # Allow erhua: 儿 may add a syllable (e.g., 花儿 -> hua er)
            # Allow 多音字 entries where one char maps to two readings
            if syllable_count != char_count:
                mismatches.append(
                    (path, line_no, phrase, code, char_count, syllable_count)
                )
    return mismatches


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Check that native Chinese lexicon pinyin codes use known syllables.",
    )
    parser.add_argument("--root", type=Path, default=None, help="repository root")
    parser.add_argument("--max-samples", type=int, default=12, help="samples to print")
    parser.add_argument(
        "--check-syllable-count",
        action="store_true",
        help="also verify pinyin syllable count matches Chinese character count",
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
    syllables_path = root / "pinyin-ime" / "data" / "syllables.txt"
    lexicon_dir = root / "lexicon"
    if not syllables_path.is_file():
        print(f"missing syllable table: {syllables_path}")
        return 2
    if not lexicon_dir.is_dir():
        print(f"missing lexicon directory: {lexicon_dir}")
        return 2

    exit_code = 0

    unknown = scan_unknown_syllables(lexicon_dir, load_syllables(syllables_path))
    if unknown:
        total_refs = sum(len(items) for items in unknown.values())
        print(
            f"Lexicon syllable check failed: {len(unknown)} unknown syllable(s), "
            f"{total_refs} reference(s)"
        )
        for token, hits in sorted(unknown.items(), key=lambda kv: (-len(kv[1]), kv[0]))[
            : max(args.max_samples, 0)
        ]:
            path, line_no, phrase, code = hits[0]
            rel = path.relative_to(root).as_posix()
            print(f"  - {token}: {len(hits)} refs; first {rel}:{line_no} {phrase}\t{code}")
        exit_code = 1
    else:
        checked = sum(1 for _ in iter_chinese_lexicon_files(lexicon_dir))
        print(f"Lexicon syllable check passed: {checked} Chinese text file(s) scanned")

    if args.check_syllable_count:
        mismatches = scan_syllable_count_mismatches(lexicon_dir)
        if mismatches:
            print(
                f"Lexicon syllable count check: {len(mismatches)} mismatch(es) found"
            )
            for path, line_no, phrase, code, cc, sc in mismatches[
                : max(args.max_samples, 0)
            ]:
                rel = path.relative_to(root).as_posix()
                print(
                    f"  - {rel}:{line_no} '{phrase}' code='{code}' "
                    f"chars={cc} syllables={sc}"
                )
            # Syllable count mismatches are warnings, not errors (they may be
            # intentional for erhua or multi-reading entries)
            print("  (syllable count mismatches are advisory; review manually)")
        else:
            print("Lexicon syllable count check passed")

    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
