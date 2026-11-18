from __future__ import annotations
import bisect
import hashlib
import json
import codecs
import io
import re
from pathlib import Path
from .config import (
    RETIRED_OPTIONAL_LEXICON_TAGS,
    RETIRED_LEXICON_FILES,
    COMMON_ENGLISH_LEXICON_RELATIVE_PATH,
    COMMON_ENGLISH_LICENSE_RELATIVE_PATH,
    EXPECTED_COMMON_ENGLISH_WORDS,
    SINGLE_CHAR_COMMON_RELATIVE_PATH,
    EXPECTED_SINGLE_CHAR_COMMON_ROWS,
    REQUIRED_COMMON_ENGLISH_WORDS,
)
from .manifests import _manifest_file_entry, sha256_file


LEXICON_LAYER_ORDER = {"core": 0, "base": 1, "ext": 2, "large": 3, "en": 4}

_HEX_COLOR_RE = re.compile(r"^#?[0-9a-fA-F]{6}$")


def find_lexicon_dir(repo: Path) -> Path:
    fixed = repo / "lexicon"
    if fixed.is_dir():
        return fixed.resolve()
    raise FileNotFoundError("lexicon directory not found: lexicon/")


def _looks_utf16(data: bytes, le: bool) -> bool:
    if len(data) < 4 or len(data) % 2:
        return False
    nul_idx = 1 if le else 0
    nul = sum(1 for i in range(0, len(data), 2) if data[i + nul_idx] == 0)
    return nul > 0 and nul * 3 >= len(data) // 2


def _has_cjk(s: str) -> bool:
    return any(
        0x3400 <= ord(c) <= 0x4DBF
        or 0x4E00 <= ord(c) <= 0x9FFF
        or 0xF900 <= ord(c) <= 0xFAFF
        or 0x20000 <= ord(c) <= 0x2EBEF
        for c in s
    )


def _weight_repeats(w: int) -> int:
    # Lexicon frequencies are normalized to the 1..10000 score space.
    # Keep five coarse corpus-repeat buckets without relying on the old
    # million-scale source weights.
    if w >= 7_500:
        return 5
    if w >= 5_000:
        return 4
    if w >= 2_500:
        return 3
    if w >= 100:
        return 2
    return 1


def _adjust_weight_for_source(term: str, raw_weight: int, source_file_name: str) -> int:
    """Sanitize source weights before frequency calibration."""
    _ = (term, source_file_name)
    return max(1, raw_weight)


def _parse_lexicon_weight(parts: list[str]) -> int:
    for field in reversed(parts[1:]):
        try:
            return int(field.strip())
        except ValueError:
            continue
    return 1


def _is_native_lexicon_text(path: Path) -> bool:
    return path.suffix.lower() == ".txt" and not path.name.lower().startswith("readme")


def _lexicon_layer(path: Path) -> str | None:
    parts = {part.casefold() for part in path.parts}
    if "zh-ext" in parts:
        return "ext"
    for legacy_layer in ("core", "base", "ext", "large", "en"):
        if legacy_layer in parts:
            return legacy_layer
    if "zh" not in parts:
        return None
    name = path.name.casefold()
    if name in {
        "kaixin_explicit.txt",
        "kaixin_polyphone.txt",
        "kaixin_pronunciation_aliases.txt",
        "life_hot_3char_curated.txt",
        "life_hot_4char_curated.txt",
    }:
        return "core"
    if "_tail_" in name or name.startswith("large_"):
        return "large"
    return "base"


def _lexicon_sort_key(path: Path, _root: Path) -> tuple[int, str]:
    layer = LEXICON_LAYER_ORDER.get(_lexicon_layer(path), 99)
    return layer, path.as_posix().lower()


def _is_default_enabled_lexicon_source(path: Path) -> bool:
    """Keep build-time LM inputs aligned with the runtime default lexicon set."""
    name = path.name.lower()
    if any(
        path.as_posix().lower().endswith("/" + relative.lower())
        for relative in RETIRED_LEXICON_FILES
    ):
        return False
    if _lexicon_layer(path) == "ext":
        tag = path.stem.lower()
        if tag.startswith("thuocl_"):
            tag = tag[len("thuocl_") :]
        elif "__thuocl_" in tag:
            tag = tag.split("__thuocl_", 1)[1]
        if tag in RETIRED_OPTIONAL_LEXICON_TAGS:
            return False
    return not name.endswith("_纯名单.txt") and not name.startswith(("professional_", "hangzhou_"))


def _lexicon_term_char_count(term: str) -> int:
    return sum(not ch.isspace() for ch in term)


def _is_frequency_reference_lexicon_source(path: Path) -> bool:
    return _lexicon_layer(path) == "base"


def _is_absolute_priority_lexicon_source(path: Path) -> bool:
    return _lexicon_layer(path) == "core" and path.name.lower() in {
        "kaixin_polyphone.txt",
        "kaixin_pronunciation_aliases.txt",
    }


def _is_unscaled_lexicon_source(path: Path) -> bool:
    """Keep intentionally low-priority sources on their authored scale.

    Names and local places contain flat low-priority weights. Percentile
    calibration would turn those values into an arbitrary point in the base
    corpus distribution, making recall-only entries look common.
    """
    return _is_absolute_priority_lexicon_source(path) or path.name.lower() in {
        "people_names.txt",
        "hangzhou_local.txt",
        "daily_communication.txt",
    }


def _source_frequency_values(path: Path) -> dict[int, list[int]]:
    """Read usable CJK entry weights grouped by phrase length from one source."""
    values: dict[int, list[int]] = {}
    for line in _iter_lexicon_lines(path):
        line = line.strip().lstrip("\ufeff")
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        term = parts[0].strip()
        if not term or not _has_cjk(term):
            continue
        values.setdefault(_lexicon_term_char_count(term), []).append(
            _adjust_weight_for_source(term, _parse_lexicon_weight(parts), path.name)
        )
    return values


def _build_frequency_calibrators(
    source_files: list[Path],
) -> dict[Path, tuple[dict[int, list[int]], dict[int, list[int]]]]:
    """Map auxiliary source percentiles onto the base lexicon's frequency scale."""
    reference: dict[int, list[int]] = {}
    source_values: dict[Path, dict[int, list[int]]] = {}
    for path in source_files:
        values = _source_frequency_values(path)
        if _is_frequency_reference_lexicon_source(path):
            for length, weights in values.items():
                reference.setdefault(length, []).extend(weights)
        elif not _is_unscaled_lexicon_source(path):
            source_values[path] = values
    if any(path.name.lower() == "life_common_3char.txt" for path in source_files):
        fixed_reference = json.loads(
            (
                Path(__file__).resolve().parents[2] / "shared/lexicon_frequency_reference.json"
            ).read_text(encoding="utf-8")
        )
        reference.update({int(length): weights for length, weights in fixed_reference.items()})
    for weights in reference.values():
        weights.sort()
    for values in source_values.values():
        for weights in values.values():
            weights.sort()
    return {path: (values, reference) for path, values in source_values.items() if values}


def _calibrated_source_weight(
    term: str,
    raw_weight: int,
    calibrator: tuple[dict[int, list[int]], dict[int, list[int]]] | None,
) -> int:
    """Map auxiliary source percentiles onto the base lexicon's frequency scale.

    This is the Python mirror of ``SourceFrequencyCalibrator::normalize`` in
    ``crates/kaixin-core/src/thuocl.rs``.  The two implementations MUST stay in sync:
    any change to the percentile-mapping logic here must be reflected in the
    Rust side, and vice versa.  See the Rust doc comment for rationale.
    """
    if calibrator is None:
        return max(1, raw_weight)
    source, reference = calibrator
    length = _lexicon_term_char_count(term)
    source_weights = source.get(length)
    reference_weights = reference.get(length)
    if not source_weights or not reference_weights:
        return max(1, raw_weight)
    lower = bisect.bisect_left(source_weights, raw_weight)
    upper = bisect.bisect_right(source_weights, raw_weight)
    rank = lower + max(0, upper - lower - 1) // 2
    rank = min(rank, len(source_weights) - 1)
    if len(source_weights) <= 1 or len(reference_weights) <= 1:
        return max(1, reference_weights[0])
    index = rank * (len(reference_weights) - 1) // (len(source_weights) - 1)
    index = min(index, len(reference_weights) - 1)
    return max(1, reference_weights[index])


def _lexicon_source_files(repo: Path) -> list[Path]:
    lex_dir = find_lexicon_dir(repo)
    return sorted(
        [
            f
            for f in lex_dir.rglob("*")
            if f.is_file() and _is_default_enabled_lexicon_source(f) and _is_native_lexicon_text(f)
        ],
        key=lambda p: _lexicon_sort_key(p, lex_dir),
    )


def _corpus_lexicon_source_files(repo: Path) -> list[Path]:
    """Return only sources that can contribute CJK terms to corpus.txt."""
    lex_dir = find_lexicon_dir(repo)
    return [
        path
        for path in _lexicon_source_files(repo)
        if "en" not in {part.casefold() for part in path.relative_to(lex_dir).parts}
    ]


def _source_files_fingerprint(lex_dir: Path, source_files: list[Path]) -> tuple[float, str]:
    h = hashlib.sha256()
    max_m = 0.0
    for path in source_files:
        try:
            st = path.stat()
            max_m = max(max_m, st.st_mtime)
            rel = path.resolve().relative_to(lex_dir.resolve()).as_posix()
            h.update(rel.encode("utf-8", errors="surrogateescape"))
            h.update(b"\0")
            h.update(sha256_file(path).encode("ascii"))
            h.update(b"\n")
        except (OSError, ValueError):
            continue
    return max_m, h.hexdigest()


def _lexicon_sources_fingerprint(repo: Path) -> tuple[float, str]:
    try:
        lex_dir = find_lexicon_dir(repo)
        return _source_files_fingerprint(lex_dir, _lexicon_source_files(repo))
    except FileNotFoundError:
        return 0.0, hashlib.sha256().hexdigest()


def _corpus_sources_fingerprint(repo: Path) -> tuple[float, str]:
    try:
        lex_dir = find_lexicon_dir(repo)
        return _source_files_fingerprint(lex_dir, _corpus_lexicon_source_files(repo))
    except FileNotFoundError:
        return 0.0, hashlib.sha256().hexdigest()


def _corpus_fingerprint_path(corpus: Path) -> Path:
    return corpus.with_name(corpus.name + ".source.sha256")


def model_source_fingerprints(repo: Path) -> dict:
    """Return source hashes that determine the embedded Rust input model."""
    data_dir = repo / "pinyin-ime" / "data"
    inputs = [
        data_dir / "pinyin_supported_chars.txt",
        data_dir / "syllables.txt",
        data_dir / "corpus.txt",
        _corpus_fingerprint_path(data_dir / "corpus.txt"),
        repo / "crates" / "kaixin-core" / "build.rs",
        repo / "crates" / "kaixin-core" / "src" / "dict.rs",
        repo / "data_sources" / "kaixin" / "pronunciation_aliases.tsv",
        repo / "data_sources" / "kaixin" / "pronunciation_exclusions.tsv",
    ]
    files = []
    h = hashlib.sha256()
    for path in inputs:
        if not path.is_file():
            continue
        entry = _manifest_file_entry(path, repo)
        files.append(entry)
        h.update(entry["path"].encode("utf-8"))
        h.update(b"\0")
        h.update(entry["sha256"].encode("ascii"))
        h.update(b"\n")
    return {"sha256": h.hexdigest(), "files": files}


def _iter_lexicon_lines(path: Path):
    """Build helper: _iter_lexicon_lines."""
    with path.open("rb") as raw:
        head = raw.read(262144)
        raw.seek(0)
        if head.startswith(codecs.BOM_UTF8):
            raw.seek(len(codecs.BOM_UTF8))
            text = io.TextIOWrapper(raw, encoding="utf-8", errors="replace", newline=None)
        elif head.startswith(codecs.BOM_UTF16_LE):
            raw.seek(len(codecs.BOM_UTF16_LE))
            text = io.TextIOWrapper(raw, encoding="utf-16-le", errors="replace", newline=None)
        elif head.startswith(codecs.BOM_UTF16_BE):
            raw.seek(len(codecs.BOM_UTF16_BE))
            text = io.TextIOWrapper(raw, encoding="utf-16-be", errors="replace", newline=None)
        else:
            if _looks_utf16(head, le=True):
                text = io.TextIOWrapper(raw, encoding="utf-16-le", errors="replace", newline=None)
            else:
                chosen = "utf-8"
                for enc in ("utf-8", "gb18030"):
                    try:
                        head.decode(enc)
                        chosen = enc
                        break
                    except UnicodeDecodeError:
                        continue
                raw.seek(0)
                text = io.TextIOWrapper(raw, encoding=chosen, errors="replace", newline=None)
        yield from text


def validate_single_char_common_lexicon(lexicon_dir: Path) -> int:
    """Validate the curated common-character head shipped with every package."""
    path = lexicon_dir / SINGLE_CHAR_COMMON_RELATIVE_PATH
    if not path.is_file() or path.stat().st_size <= 0:
        raise FileNotFoundError(f"missing common single-character lexicon: {path}")

    seen: set[str] = set()
    previous_weight: int | None = None
    for line_no, raw in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        fields = [field.strip() for field in line.split("\t")]
        if len(fields) != 2 or len(fields[0]) != 1:
            raise RuntimeError(f"invalid common single-character row {path}:{line_no}")
        char, weight_text = fields
        try:
            weight = int(weight_text)
        except ValueError as exc:
            raise RuntimeError(
                f"invalid common single-character weight {path}:{line_no}: {weight_text!r}"
            ) from exc
        if char in seen:
            raise RuntimeError(f"duplicate common character {path}:{line_no}: {char}")
        if previous_weight is not None and weight >= previous_weight:
            raise RuntimeError(
                f"common single-character weights must be strictly decreasing {path}:{line_no}"
            )
        seen.add(char)
        previous_weight = weight

    if len(seen) != EXPECTED_SINGLE_CHAR_COMMON_ROWS:
        raise RuntimeError(
            f"common single-character lexicon must contain exactly "
            f"{EXPECTED_SINGLE_CHAR_COMMON_ROWS} rows, found {len(seen)}: {path}"
        )
    return len(seen)


def validate_common_english_lexicon(lexicon_dir: Path) -> int:
    """Validate the checked-in 20,000-word native English lexicon."""
    path = lexicon_dir / COMMON_ENGLISH_LEXICON_RELATIVE_PATH
    license_path = lexicon_dir / COMMON_ENGLISH_LICENSE_RELATIVE_PATH
    readme_path = lexicon_dir / "en" / "README.md"
    for required_path in (path, license_path, readme_path):
        if not required_path.is_file() or required_path.stat().st_size <= 0:
            raise FileNotFoundError(f"missing English lexicon file: {required_path}")

    try:
        text = path.read_text(encoding="utf-8-sig")
    except UnicodeDecodeError as exc:
        raise RuntimeError(f"English lexicon must be UTF-8: {path}") from exc

    words: set[str] = set()
    previous_weight: int | None = None
    for line_no, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        fields = [field.strip() for field in line.split("\t")]
        if len(fields) != 3:
            raise RuntimeError(
                f"invalid English lexicon row {path}:{line_no}: expected exactly 3 tab-separated fields"
            )
        word, code, weight_text = fields
        if not (2 <= len(word) <= 24 and word.isascii() and word.isalpha() and word.islower()):
            raise RuntimeError(f"invalid English lexicon word {path}:{line_no}: {word!r}")
        if code != word:
            raise RuntimeError(f"English input code must equal the word {path}:{line_no}: {code!r}")
        try:
            weight = int(weight_text)
        except ValueError as exc:
            raise RuntimeError(
                f"invalid English lexicon weight {path}:{line_no}: {weight_text!r}"
            ) from exc
        if weight <= 0:
            raise RuntimeError(f"English lexicon weight must be positive {path}:{line_no}")
        if previous_weight is not None and weight > previous_weight:
            raise RuntimeError(f"English lexicon weights must be non-increasing {path}:{line_no}")
        if word in words:
            raise RuntimeError(f"duplicate English lexicon word {path}:{line_no}: {word}")
        words.add(word)
        previous_weight = weight

    if len(words) != EXPECTED_COMMON_ENGLISH_WORDS:
        raise RuntimeError(
            f"English lexicon must contain exactly {EXPECTED_COMMON_ENGLISH_WORDS} words, "
            f"found {len(words)}: {path}"
        )
    missing = sorted(REQUIRED_COMMON_ENGLISH_WORDS - words)
    if missing:
        raise RuntimeError(
            f"English lexicon is missing required coverage words: {', '.join(missing)}"
        )
    return len(words)


def _is_color_like(value: object) -> bool:
    return isinstance(value, str) and _HEX_COLOR_RE.match(value.strip()) is not None


def _parse_theme_color(value: object) -> tuple[int, int, int] | None:
    if not _is_color_like(value):
        return None
    text = str(value).strip().lstrip("#")
    return (int(text[0:2], 16), int(text[2:4], 16), int(text[4:6], 16))


def _relative_luminance(rgb: tuple[int, int, int]) -> float:
    def channel(v: int) -> float:
        s = v / 255.0
        return s / 12.92 if s <= 0.04045 else ((s + 0.055) / 1.055) ** 2.4

    r, g, b = rgb
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)


def _contrast_ratio(a: tuple[int, int, int], b: tuple[int, int, int]) -> float:
    la = _relative_luminance(a)
    lb = _relative_luminance(b)
    hi, lo = (la, lb) if la >= lb else (lb, la)
    return (hi + 0.05) / (lo + 0.05)


def _validate_theme_json(path: Path) -> list[str]:
    """Build helper: _validate_theme_json."""
    theme_name = f"{path.parent.name}/{path.name}"
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        raise RuntimeError(f"Could not read theme file: {path} ({e})") from e
    except OSError as e:
        raise RuntimeError(f"Could not read theme file: {path} ({e})") from e

    if not isinstance(data, dict):
        raise RuntimeError(f"Theme root must be a JSON object: {path}")

    warnings: list[str] = []
    required = ("name", "window_bg", "header_bg", "text", "selected_bg", "selected_text")
    missing = [k for k in required if k not in data]
    if missing:
        warnings.append(f"{theme_name}: missing required keys: {', '.join(missing)}")

    color_keys = [
        "window_bg",
        "window_bg_to",
        "header_bg",
        "header_bg_to",
        "border",
        "divider",
        "item_bg",
        "item_border",
        "hover_bg",
        "hover_border",
        "selected_bg",
        "selected_border",
        "pressed_bg",
        "pressed_border",
        "text",
        "muted_text",
        "selected_text",
        "selected_muted_text",
        "badge_bg",
        "badge_border",
        "badge_text",
        "chip_bg",
        "chip_border",
        "chip_text",
        "chip_active_bg",
        "chip_active_border",
        "chip_active_text",
        "selected_outline",
    ]
    for key in color_keys:
        if key in data and data[key] is not None and not _is_color_like(data[key]):
            warnings.append(f"{theme_name}: field {key} is not a valid color (#RRGGBB expected)")

    def color_for(key: str, fallback: str | None = None) -> tuple[int, int, int] | None:
        return _parse_theme_color(data.get(key)) or (
            _parse_theme_color(data.get(fallback)) if fallback else None
        )

    contrast_checks = [
        ("text", "item_bg", "window_bg", 4.2),
        ("muted_text", "item_bg", "window_bg", 3.2),
        ("selected_text", "selected_bg", None, 4.2),
        ("selected_muted_text", "selected_bg", None, 3.2),
        ("badge_text", "badge_bg", None, 3.6),
        ("chip_text", "chip_bg", None, 3.6),
        ("chip_active_text", "chip_active_bg", None, 4.2),
    ]
    for fg_key, bg_key, bg_fallback, min_ratio in contrast_checks:
        fg = color_for(fg_key)
        bg = color_for(bg_key, bg_fallback)
        if not fg or not bg:
            continue
        ratio = _contrast_ratio(fg, bg)
        if ratio < min_ratio:
            warnings.append(
                f"{theme_name}: {fg_key} on {bg_key} contrast is {ratio:.2f}:1 "
                f"(recommended >= {min_ratio:.1f}:1)"
            )

    for key in ("shadow_enabled", "animations_enabled"):
        if key in data and not isinstance(data[key], bool):
            warnings.append(f"{theme_name}: {key} should be boolean")
    numeric_ranges = {
        "selected_accent_width": (0, 8),
        "selected_ring_opacity": (0.0, 1.0),
        "border_opacity": (0.0, 1.0),
        "divider_opacity": (0.0, 1.0),
        "shadow_opacity": (0.0, 1.0),
        "shadow_size": (0, 24),
        "font_weight": (300, 700),
        "selected_font_weight": (400, 800),
        "label_font_weight": (400, 800),
        "chip_font_weight": (350, 700),
        "show_animation_ms": (0, 240),
        "selection_animation_ms": (0, 240),
        "hover_animation_ms": (0, 240),
        "press_animation_ms": (0, 240),
        "page_animation_ms": (0, 240),
    }
    for key, (lo, hi) in numeric_ranges.items():
        if key not in data:
            continue
        value = data[key]
        if (
            not isinstance(value, (int, float))
            or isinstance(value, bool)
            or value < lo
            or value > hi
        ):
            warnings.append(f"{theme_name}: {key} should be numeric in range {lo}..{hi}")
    if "selected_indicator" in data and data["selected_indicator"] not in (
        "left_bar",
        "bottom_bar",
        "outline",
        "none",
    ):
        warnings.append(
            f"{theme_name}: selected_indicator should be left_bar, bottom_bar, outline, or none"
        )

    return warnings
