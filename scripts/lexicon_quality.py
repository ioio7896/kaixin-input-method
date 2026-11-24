"""Reviewed phrase membership and narrowly scoped reading/grammar corrections.

No statistical score is treated as evidence that a generated sentence is natural.
The archived import snapshots remain provenance; maintained runtime output is
filtered by this policy even when an older snapshot is imported again.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
POLICY = json.loads((ROOT / "shared/lexicon_quality.json").read_text(encoding="utf-8"))
CORRECTIONS = {(r["phrase"], r["rejected"]): r["reading"] for r in POLICY["reading_corrections"]}
THREE_CHAR_SCORE_OVERRIDES = {
    (r["phrase"], r["reading"]): r["score"] for r in POLICY["three_char_score_overrides"]
}
SHORT_POLICY = json.loads((ROOT / "shared/short_phrase_quality.json").read_text(encoding="utf-8"))
SHORT_REJECTIONS = {
    phrase: reason
    for reason, phrases in SHORT_POLICY["rejected_groups"].items()
    for phrase in phrases
}
if any(not 3 <= len(phrase) <= 5 for phrase in SHORT_REJECTIONS):
    raise ValueError("Short-phrase review contains an entry outside three to five characters")
REJECTED_PHRASES = {**POLICY.get("rejected_phrases", {}), **SHORT_REJECTIONS}
SPELLING_CORRECTIONS = {rule["phrase"]: rule for rule in POLICY.get("spelling_corrections", [])}
RETAINED_READINGS = {
    (rule["phrase"], rule["reading"]) for rule in POLICY.get("retained_phrase_readings", [])
}
if set(SHORT_POLICY["preserved_phrases"]) & REJECTED_PHRASES.keys():
    raise ValueError("Short-phrase rejection conflicts with a preserved expression")
# In these old templates 一下 was appended after the object. Keep explicitly
# authored expressions such as 请看一下文件 and 帮我看一下 unchanged.
MISPLACED_YIXIA = re.compile(
    r"^(?:请|麻烦)(?:帮我)?(?:发来|发送|提供|补充|查看|核对|确认|转发|同步|更新|调整|准备|提交|查询|完成|检查|订正).+一下$"
)
ABSTRACT_DESTINATION = re.compile(
    r"先去(?:时间|地点|内容|安排|计划|事情|问题|情况|消息|电话|短信|文件|资料|材料)$"
)
FULLWIDTH_PUNCT_RE = re.compile(r"[（）【】《》，。！？、；：“”‘’]")
HANZI_ONLY_RE = re.compile(r"^[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff\U00020000-\U000323af]+$")
# Reviewed low-weight terms must not regain priority through a Core overlay.
CORE_BLOCKED_TERMS = {"操你妈", "傻逼"} | {phrase for phrase, _ in THREE_CHAR_SCORE_OVERRIDES}
CORE_CURATED_FILES = {
    "life_hot_3char_curated.txt",
    "life_hot_4char_curated.txt",
}


def phrase_rejection_reason(phrase: str, *, core: bool = False) -> str | None:
    # Match reviewed words exactly. Short fragments must never become substring
    # filters that remove legitimate longer phrases, names or idioms.
    if core and phrase in CORE_BLOCKED_TERMS:
        return "blocked vulgar/sensitive term in Core layer"
    if phrase in REJECTED_PHRASES:
        return "reviewed_lexical_fragment"
    if MISPLACED_YIXIA.search(phrase):
        return "request_suffix_word_order"
    if ABSTRACT_DESTINATION.search(phrase):
        return "abstract_destination"
    return None


def corrected_reading(phrase: str, reading: str) -> str:
    normalized = " ".join(reading.lower().split())
    return CORRECTIONS.get((phrase, normalized), normalized)


def canonical_entry(phrase: str, reading: str) -> tuple[str, str]:
    """Normalize only reviewed full spellings, never a matching substring."""
    rule = SPELLING_CORRECTIONS.get(phrase)
    if rule:
        phrase, reading = rule["replacement"], rule["reading"]
    return phrase, corrected_reading(phrase, reading)


def character_readings(root: Path) -> dict[str, set[str]]:
    """Pinned rust-pinyin snapshot plus the same single-character overrides as runtime."""

    def normalize(value: str) -> str:
        return value.lower().replace("ü", "v").replace("nue", "nve").replace("lue", "lve")

    result = {}
    path = root / "pinyin-ime/data/pinyin_character_readings.txt"
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        if line.strip() and not line.startswith("#"):
            ch, codes = line.split("\t")
            result[ch] = {normalize(code) for code in codes.split()}
    for name in ("pronunciation_aliases.tsv", "pronunciation_exclusions.tsv"):
        path = root / "data_sources/kaixin" / name
        for line in path.read_text(encoding="utf-8-sig").splitlines():
            if not line.strip() or line.startswith("#"):
                continue
            fields = line.split("\t")
            if len(fields[0]) != 1:
                continue
            readings = result.setdefault(fields[0], set())
            if len(fields) > 3 and fields[3] == "historical":
                readings.discard(normalize(fields[1]))
            else:
                readings.add(normalize(fields[1]))
    return result


def reading_matches_characters(phrase: str, reading: str, reference: dict[str, set[str]]) -> bool:
    # Mixed entities have authored input codes. Classical/place-name readings
    # are exact-word exceptions, never aliases spread to every use of a character.
    if (phrase, reading) in RETAINED_READINGS:
        return True
    if not all(
        0x3400 <= ord(ch) <= 0x9FFF or 0xF900 <= ord(ch) <= 0xFAFF or 0x20000 <= ord(ch) <= 0x323AF
        for ch in phrase
    ):
        return True
    syllables = (
        reading.lower().replace("ü", "v").replace("nue", "nve").replace("lue", "lve").split()
    )
    return len(syllables) == len(phrase) and all(
        syllable in reference.get(ch, set()) for ch, syllable in zip(phrase, syllables)
    )


def reviewed_long_rows() -> list[tuple[str, str, int]]:
    path = ROOT / POLICY["reviewed_long_phrases"]
    result = []
    seen = set()
    for number, line in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        phrase, reading, score = line.split("\t")
        phrase, reading = canonical_entry(phrase, reading)
        key = (phrase, reading)
        if (
            not 5 <= len(phrase) <= 8
            or len(reading.split()) != len(phrase)
            or not 1 <= int(score) <= 10_000
            or key in seen
            or phrase_rejection_reason(phrase)
        ):
            raise ValueError(f"Invalid reviewed expression: {path}:{number}")
        seen.add(key)
        result.append((phrase, corrected_reading(phrase, reading), int(score)))
    if not result:
        raise ValueError("Reviewed expression source must not be empty")
    return result


def clean_rows(rows):
    """Filter known malformed requests; merge only identical phrase/readings."""
    result = {}
    for phrase, reading, score in rows:
        phrase, reading = canonical_entry(phrase, reading)
        if phrase_rejection_reason(phrase):
            continue
        key = (phrase, corrected_reading(phrase, reading))
        result[key] = max(result.get(key, 0), score)
    return [(phrase, reading, score) for (phrase, reading), score in result.items()]


def write_reviewed_long_lexicon() -> int:
    rows = reviewed_long_rows()
    path = ROOT / "lexicon/zh/life_common_phrases.txt"
    path.write_text(
        f"# 开心输入法：{len(rows)} 条维护清单内的五至八字生活与办公表达\n"
        "# Source: data_sources/kaixin/reviewed_long_phrases.tsv\n"
        "# 逐条维护完整表达；不按数量补齐，不组合主语、动作、宾语。\n"
        + "".join(f"{p}\t{r}\t{s}\n" for p, r, s in rows),
        encoding="utf-8",
        newline="\n",
    )
    return len(rows)


def update_category_catalog() -> None:
    """Describe maintained output without rewriting the upstream provenance."""
    manifest_path = ROOT / "shared/category_lexicons.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    for item in manifest["categories"]:
        path = ROOT / "lexicon/zh-ext" / item["file"]
        data = path.read_bytes()
        item["rows"] = sum(
            bool(line.strip()) and not line.startswith("#")
            for line in data.decode("utf-8-sig").splitlines()
        )
        item["sha256"] = hashlib.sha256(data).hexdigest()
        scores = [
            int(line.split("\t")[2])
            for line in data.decode("utf-8-sig").splitlines()
            if line.strip() and not line.startswith("#")
        ]
        item["score_min"] = min(scores, default=0)
        item["score_max"] = max(scores, default=0)
        item["score_tiers"] = {
            f"{low}-{high}": sum(low <= score <= high for score in scores)
            for low, high in ((7500, 10000), (5000, 7499), (2500, 4999), (100, 2499), (1, 99))
        }
    manifest_path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n"
    )
    generated = [
        "// Generated from shared/category_lexicons.json; maintained by scripts/lexicon_quality.py.",
        "pub fn category_lexicon_info(tag: &str) -> Option<(&'static str, &'static str)> {",
        "    match tag {",
    ]
    generated.extend(
        f'        "{c["tag"]}" => Some(("{c["title"]}", "{c["group"]}")),'
        for c in manifest["categories"]
    )
    generated.extend(
        [
            "        _ => None,",
            "    }",
            "}",
            "",
            "pub fn category_lexicon_default_enabled(tag: &str) -> Option<bool> {",
            "    match tag {",
        ]
    )
    generated.extend(
        f'        "{c["tag"]}" => Some({str(c.get("default_enabled", True)).lower()}),'
        for c in manifest["categories"]
    )
    generated.extend(
        ["        _ => None,", "    }", "}", "", "const CATEGORY_LEXICON_TAGS: &[&str] = &["]
    )
    generated.extend(f'    "{c["tag"]}",' for c in manifest["categories"])
    generated.extend(
        [
            "];",
            "",
            "fn merged_category_defaults(tag: &str) -> &'static [(&'static str, bool)] {",
            "    match tag {",
        ]
    )
    for category in manifest["categories"]:
        defaults = [
            f'("{tag}", {str(enabled).lower()})'
            for tag, enabled in category.get("legacy_defaults", {}).items()
        ]
        if not defaults:
            continue
        arm = f'        "{category["tag"]}" => &['
        inline_arm = f"{arm}{', '.join(defaults)}],"
        if len(inline_arm) <= 100:
            generated.append(inline_arm)
        else:
            generated.append(arm)
            generated.extend(f"            {default}," for default in defaults)
            generated.append("        ],")
    generated.extend(["        _ => &[],", "    }", "}"])
    (ROOT / "crates/kaixin-core/src/category_lexicons.generated.rs").write_text(
        "\n".join(generated) + "\n", encoding="utf-8", newline="\n"
    )
    from lexicon_maintenance import write_inventory

    write_inventory(ROOT, manifest["categories"])


def check_quality(root: Path) -> list[str]:
    from lexicon_maintenance import THREE_CHAR_BASE, check_maintained_sources

    errors = []
    reference = character_readings(root)
    found_readings = set()
    zh_best_by_py: dict[str, tuple[int, str]] = {}
    ext_rows_by_file: dict[str, list[tuple[str, str, int]]] = {}
    for path in sorted((root / "lexicon").rglob("*.txt")):
        rel_path = path.relative_to(root)
        rel_parts = rel_path.parts[1:]
        if "en" in rel_parts:
            continue
        is_core_layer = rel_parts[0] == "core" or path.name in CORE_CURATED_FILES
        is_zh_or_ext = rel_parts[0] in ("zh", "zh-ext")
        is_three_char_base = rel_path.as_posix() == THREE_CHAR_BASE
        prev_2char_score = 10_001
        two_char_hot_count = 0
        for number, line in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
            if not line.strip() or line.startswith("#"):
                continue
            fields = line.split("\t")
            phrase = fields[0]
            if FULLWIDTH_PUNCT_RE.search(phrase):
                errors.append(
                    f"{path.relative_to(root)}:{number}: fullwidth punctuation in phrase: {phrase}"
                )
            if len(fields) >= 3:
                reading = fields[1]
                found_readings.add((phrase, reading))
                if not reading_matches_characters(phrase, reading, reference):
                    errors.append(
                        f"{path.relative_to(root)}:{number}: word/character reading mismatch: {phrase} {reading}"
                    )
                if is_zh_or_ext and any(syl in ("lue", "nue") for syl in reading.split()):
                    errors.append(
                        f"{path.relative_to(root)}:{number}: noncanonical lue/nue (use lve/nve): {phrase} {reading}"
                    )
                if (
                    not HANZI_ONLY_RE.match(phrase)
                    and len(reading.strip()) == 1
                    and reading.strip().isascii()
                ):
                    errors.append(
                        f"{path.relative_to(root)}:{number}: single-letter non-hanzi code: {phrase} -> {reading}"
                    )
            if phrase in SPELLING_CORRECTIONS:
                errors.append(
                    f"{path.relative_to(root)}:{number}: noncanonical spelling: {phrase} -> {SPELLING_CORRECTIONS[phrase]['replacement']}"
                )
            try:
                score = int(fields[1] if len(fields) == 2 else fields[2])
                if not 1 <= score <= 10_000:
                    raise ValueError("out of range")
            except (ValueError, IndexError):
                errors.append(f"{path.relative_to(root)}:{number}: expected a 1-10000 score")
                score = 0
            if is_three_char_base and len(fields) >= 3:
                expected_score = THREE_CHAR_SCORE_OVERRIDES.get((phrase, fields[1]))
                if expected_score is not None and score != expected_score:
                    errors.append(
                        f"{rel_path}:{number}: expected reviewed score {expected_score}: {phrase} {fields[1]}"
                    )
            if path.name == "life_common_2char_125000.txt" and len(fields) >= 3:
                if score > prev_2char_score:
                    errors.append(
                        f"{path.relative_to(root)}:{number}: score inversion ({score} > {prev_2char_score}): {phrase}"
                    )
                prev_2char_score = score
                if score >= 3500:
                    two_char_hot_count += 1
            if (
                rel_parts[0] == "zh"
                and len(fields) >= 3
                and path.name != "single_char_common_8105.txt"
            ):
                py = fields[1]
                prev = zh_best_by_py.get(py)
                if prev is None or score > prev[0]:
                    zh_best_by_py[py] = (score, phrase)
            elif rel_parts[0] == "zh-ext" and len(fields) >= 3:
                ext_rows_by_file.setdefault(path.name, []).append((phrase, fields[1], score))
            reason = phrase_rejection_reason(phrase, core=is_core_layer)
            if reason:
                errors.append(f"{path.relative_to(root)}:{number}: {reason}: {phrase}")
            if len(fields) >= 3 and corrected_reading(phrase, fields[1]) != fields[1]:
                errors.append(
                    f"{path.relative_to(root)}:{number}: rejected reading: {phrase} {fields[1]}"
                )
        if path.name == "life_common_2char_125000.txt" and two_char_hot_count > 30_000:
            errors.append(
                f"{path.relative_to(root)}: hot mixed-index slice (>=3500) too large: {two_char_hot_count} > 30000"
            )
    # Compare membership, readings and weights: accidentally running the legacy
    # quota generator must fail packaging, even if its rows look grammatical.
    path = root / "lexicon/zh/life_common_phrases.txt"
    actual = [
        line
        for line in path.read_text(encoding="utf-8-sig").splitlines()
        if line.strip() and not line.startswith("#")
    ]
    expected = [f"{p}\t{r}\t{s}" for p, r, s in reviewed_long_rows()]
    if actual != expected:
        errors.append(
            "life_common_phrases.txt differs from reviewed source; regenerate with lexicon_quality.py --apply"
        )
    manifest = json.loads((root / "shared/category_lexicons.json").read_text(encoding="utf-8"))
    for item in manifest["categories"]:
        path = root / "lexicon/zh-ext" / item["file"]
        data = path.read_bytes()
        count = sum(
            bool(line.strip()) and not line.startswith("#")
            for line in data.decode("utf-8-sig").splitlines()
        )
        if count != item["rows"] or hashlib.sha256(data).hexdigest() != item["sha256"]:
            errors.append(f"{item['file']}: maintained category manifest has stale count/hash")
        if item.get("default_enabled", False):
            for phrase, py, score in ext_rows_by_file.get(item["file"], []):
                zh_best = zh_best_by_py.get(py)
                if zh_best and zh_best[1] != phrase and score - zh_best[0] > 3500:
                    errors.append(
                        f"lexicon/zh-ext/{item['file']}: default-on ext entry {phrase}({score}) severely outranks zh {zh_best[1]}({zh_best[0]}) on '{py}'"
                    )
    for phrase, reading in sorted(RETAINED_READINGS - found_readings):
        errors.append(f"Protected word-specific reading was lost: {phrase} {reading}")
    errors.extend(check_maintained_sources(root))
    # Fixture membership is a source invariant, not an engine evaluation.
    path = root / "tests/popular_three_char_cases.tsv"
    for number, line in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
        if line.strip() and not line.startswith("#"):
            fields = line.split("\t")
            if len(fields) > 1 and phrase_rejection_reason(fields[1]):
                errors.append(
                    f"{path.relative_to(root)}:{number}: rejected phrase remains a positive case: {fields[1]}"
                )
    return errors


def apply_quality() -> dict:
    from lexicon_maintenance import THREE_CHAR_BASE, apply_maintained_sources

    report = {
        "removed": [],
        "corrected": [],
        "spelling_corrected": [],
        "files": {},
        "deduplicated": [],
        "maintenance": apply_maintained_sources(ROOT),
    }
    for path in sorted((ROOT / "lexicon").rglob("*.txt")):
        rel_path = path.relative_to(ROOT)
        rel_parts = rel_path.parts[1:]
        if "en" in rel_parts or path.name == "life_common_phrases.txt":
            continue
        is_core_layer = rel_parts[0] == "core" or path.name in CORE_CURATED_FILES
        is_three_char_base = rel_path.as_posix() == THREE_CHAR_BASE
        lines = path.read_text(encoding="utf-8-sig").splitlines()
        output = []
        seen = {}
        for line in lines:
            if not line.strip() or line.startswith("#"):
                output.append(line)
                continue
            fields = line.split("\t")
            if fields[0] in SPELLING_CORRECTIONS and len(fields) >= 3:
                phrase, reading = canonical_entry(fields[0], fields[1])
                report["spelling_corrected"].append(
                    {
                        "file": path.relative_to(ROOT).as_posix(),
                        "row": line,
                        "replacement": phrase,
                        "reading": reading,
                    }
                )
                fields[0], fields[1] = phrase, reading
            if phrase_rejection_reason(fields[0], core=is_core_layer):
                report["removed"].append({"file": path.relative_to(ROOT).as_posix(), "row": line})
                continue
            if len(fields) >= 3:
                reading = corrected_reading(fields[0], fields[1])
                if reading != fields[1]:
                    report["corrected"].append(
                        {"file": path.relative_to(ROOT).as_posix(), "row": line, "reading": reading}
                    )
                    fields[1] = reading
                if is_three_char_base:
                    score = THREE_CHAR_SCORE_OVERRIDES.get((fields[0], reading))
                    if score is not None:
                        fields[2] = str(score)
            revised = "\t".join(fields)
            # Merge only the same phrase, explicit reading AND reading class.
            # In particular, lue/lve and colloquial/primary rows remain distinct.
            key = (
                fields[0],
                fields[1] if len(fields) >= 3 else "",
                fields[3] if len(fields) >= 4 else "",
            )
            if key in seen:
                index = seen[key]
                previous = output[index].split("\t")
                weight_index = 1 if len(fields) == 2 else 2
                if int(fields[weight_index]) > int(previous[weight_index]):
                    output[index] = revised
                report["deduplicated"].append(
                    {"file": path.relative_to(ROOT).as_posix(), "row": revised}
                )
            else:
                seen[key] = len(output)
                output.append(revised)
        if output != lines:
            if output:
                output[0] = re.sub(r"(?<=：)\d+(?=\s*(?:条|个))", str(len(seen)), output[0])
            path.write_text("\n".join(output) + "\n", encoding="utf-8", newline="\n")
            report["files"][path.relative_to(ROOT).as_posix()] = len(seen)
    old = ROOT / "lexicon/zh/life_common_phrases.txt"
    approved = {(p, r) for p, r, s in reviewed_long_rows()}
    for line in old.read_text(encoding="utf-8-sig").splitlines():
        if line.strip() and not line.startswith("#"):
            p, r, s = line.split("\t")
            if (p, r) not in approved:
                report["removed"].append({"file": old.relative_to(ROOT).as_posix(), "row": line})
    report["reviewed_long_count"] = write_reviewed_long_lexicon()
    removed_by_length = {}
    for item in report["removed"]:
        length = str(len(item["row"].split("\t", 1)[0]))
        removed_by_length[length] = removed_by_length.get(length, 0) + 1
    report["removed_by_length"] = removed_by_length
    # The upstream import report stays frozen. This manifest describes the
    # maintained output, whose hash may now differ from the archived source.
    update_category_catalog()
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    if args.apply:
        report = apply_quality()
        dest = ROOT / "dist/lexicon-quality"
        dest.mkdir(parents=True, exist_ok=True)
        (dest / "last-cleanup.json").write_text(
            json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        print(
            f"Quality cleanup: {len(report['removed'])} removed, {len(report['corrected'])} readings corrected, {report['reviewed_long_count']} reviewed long expressions"
        )
    errors = check_quality(ROOT)
    for error in errors[:30]:
        print(error)
    if errors:
        print(f"Lexicon quality check failed: {len(errors)} issue(s)")
        return 1
    print("Lexicon quality check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
