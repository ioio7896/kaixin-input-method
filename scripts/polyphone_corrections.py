"""Load and apply the project's scoped polyphonic phrase corrections."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import re
from typing import Sequence

from pypinyin import Style, lazy_pinyin


PINYIN_SYLLABLE = re.compile(r"^[a-z]+$")


@dataclass(frozen=True)
class Correction:
    phrase: str
    reading: tuple[str, ...]
    scope: str


def load_corrections(path: Path) -> tuple[Correction, ...]:
    corrections = []
    seen = set()
    for line_no, raw in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        fields = [field.strip() for field in line.split("\t")]
        if len(fields) not in (2, 3):
            raise ValueError(f"{path}:{line_no}: expected phrase, pinyin[, scope]")
        phrase, code = fields[:2]
        reading = tuple(code.lower().replace("ü", "v").split())
        scope = fields[2] if len(fields) == 3 else "substring"
        if (not phrase or len(reading) != len(phrase)
                or any(not PINYIN_SYLLABLE.fullmatch(item) for item in reading)
                or scope not in ("exact", "substring") or phrase in seen):
            raise ValueError(f"{path}:{line_no}: invalid or duplicate correction")
        seen.add(phrase)
        corrections.append(Correction(phrase, reading, scope))
    return tuple(sorted(corrections, key=lambda item: -len(item.phrase)))


def corrected_reading(phrase: str, corrections: Sequence[Correction]) -> tuple[str, ...]:
    base = tuple(item.lower().replace("ü", "v") for item in
                 lazy_pinyin(phrase, style=Style.NORMAL, errors="default"))
    exact = next((item for item in corrections
                  if item.scope == "exact" and item.phrase == phrase), None)
    if exact is not None:
        return exact.reading
    reading = []
    offset = 0
    while offset < len(phrase):
        match = next((item for item in corrections
                      if item.scope == "substring" and phrase.startswith(item.phrase, offset)), None)
        if match is None:
            reading.append(base[offset])
            offset += 1
        else:
            reading.extend(match.reading)
            offset += len(match.phrase)
    return tuple(reading)
