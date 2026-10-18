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
REJECTED_PHRASES = POLICY.get("rejected_phrases", {})
# In these old templates 一下 was appended after the object. Keep explicitly
# authored expressions such as 请看一下文件 and 帮我看一下 unchanged.
MISPLACED_YIXIA = re.compile(
    r"^(?:请|麻烦)(?:帮我)?(?:发来|发送|提供|补充|查看|核对|确认|转发|同步|更新|调整|准备|提交|查询|完成|检查|订正).+一下$"
)
ABSTRACT_DESTINATION = re.compile(
    r"先去(?:时间|地点|内容|安排|计划|事情|问题|情况|消息|电话|短信|文件|资料|材料)$"
)


def phrase_rejection_reason(phrase: str) -> str | None:
    # Match reviewed words exactly. Short fragments must never become substring
    # filters that remove legitimate longer phrases, names or idioms.
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


def reviewed_long_rows() -> list[tuple[str, str, int]]:
    path = ROOT / POLICY["reviewed_long_phrases"]
    result = []
    seen = set()
    for number, line in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        phrase, reading, score = line.split("\t")
        key = (phrase, reading)
        if (not 5 <= len(phrase) <= 8 or len(reading.split()) != len(phrase)
                or not 1 <= int(score) <= 10_000 or key in seen
                or phrase_rejection_reason(phrase)):
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
        encoding="utf-8", newline="\n",
    )
    return len(rows)


def update_category_catalog() -> None:
    """Describe maintained output without rewriting the upstream provenance."""
    manifest_path = ROOT / "shared/category_lexicons.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    for item in manifest["categories"]:
        path = ROOT / "lexicon/zh-ext" / item["file"]
        data = path.read_bytes()
        item["rows"] = sum(bool(l.strip()) and not l.startswith("#") for l in data.decode("utf-8-sig").splitlines())
        item["sha256"] = hashlib.sha256(data).hexdigest()
        if item["tag"] == "category_idioms":
            item["group"] = "成语与固定表达"
        elif item["tag"] in {"category_common_three", "category_common_four", "category_common_long"}:
            item["group"] = "通用词语补充"
        elif item["tag"] in {"category_office", "category_project", "category_human_resources", "category_government"}:
            item["group"] = "办公与办事"
        elif item["tag"] == "category_pets":
            item["group"] = "日常表达"
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    generated = ["// Generated from shared/category_lexicons.json; maintained by scripts/lexicon_quality.py.",
                 "pub fn category_lexicon_info(tag: &str) -> Option<(&'static str, &'static str)> {",
                 "    match tag {"]
    generated.extend(f'        "{c["tag"]}" => Some(("{c["title"]}", "{c["group"]}")),' for c in manifest["categories"])
    generated.extend(['        _ => None,', '    }', '}'])
    (ROOT / "pinyin-ime/src/category_lexicons.generated.rs").write_text("\n".join(generated) + "\n", encoding="utf-8", newline="\n")


def check_quality(root: Path) -> list[str]:
    errors = []
    for path in sorted((root / "lexicon").rglob("*.txt")):
        if "en" in path.relative_to(root / "lexicon").parts:
            continue
        for number, line in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
            if not line.strip() or line.startswith("#"):
                continue
            fields = line.split("\t")
            phrase = fields[0]
            try:
                score = int(fields[1] if len(fields) == 2 else fields[2])
                if not 1 <= score <= 10_000:
                    raise ValueError("out of range")
            except (ValueError, IndexError):
                errors.append(f"{path.relative_to(root)}:{number}: expected a 1-10000 score")
            reason = phrase_rejection_reason(phrase)
            if reason:
                errors.append(f"{path.relative_to(root)}:{number}: {reason}: {phrase}")
            if len(fields) >= 3 and corrected_reading(phrase, fields[1]) != fields[1]:
                errors.append(f"{path.relative_to(root)}:{number}: rejected reading: {phrase} {fields[1]}")
    # Compare membership, readings and weights: accidentally running the legacy
    # quota generator must fail packaging, even if its rows look grammatical.
    path = root / "lexicon/zh/life_common_phrases.txt"
    actual = [line for line in path.read_text(encoding="utf-8-sig").splitlines()
              if line.strip() and not line.startswith("#")]
    expected = [f"{p}\t{r}\t{s}" for p, r, s in reviewed_long_rows()]
    if actual != expected:
        errors.append("life_common_phrases.txt differs from reviewed source; regenerate with lexicon_quality.py --apply")
    manifest = json.loads((root / "shared/category_lexicons.json").read_text(encoding="utf-8"))
    for item in manifest["categories"]:
        path = root / "lexicon/zh-ext" / item["file"]
        data = path.read_bytes()
        count = sum(bool(line.strip()) and not line.startswith("#") for line in data.decode("utf-8-sig").splitlines())
        if count != item["rows"] or hashlib.sha256(data).hexdigest() != item["sha256"]:
            errors.append(f"{item['file']}: maintained category manifest has stale count/hash")
    return errors


def apply_quality() -> dict:
    report = {"removed": [], "corrected": [], "files": {}}
    for path in sorted((ROOT / "lexicon").rglob("*.txt")):
        if "en" in path.relative_to(ROOT / "lexicon").parts or path.name == "life_common_phrases.txt":
            continue
        lines = path.read_text(encoding="utf-8-sig").splitlines()
        output = []
        seen = set()
        for line in lines:
            if not line.strip() or line.startswith("#"):
                output.append(line)
                continue
            fields = line.split("\t")
            if phrase_rejection_reason(fields[0]):
                report["removed"].append({"file": path.relative_to(ROOT).as_posix(), "row": line})
                continue
            if len(fields) >= 3:
                reading = corrected_reading(fields[0], fields[1])
                if reading != fields[1]:
                    report["corrected"].append({"file": path.relative_to(ROOT).as_posix(), "row": line, "reading": reading})
                    fields[1] = reading
            revised = "\t".join(fields)
            # Preserve independent pronunciation classes and scores here.
            if revised not in seen:
                output.append(revised)
                seen.add(revised)
        if output != lines:
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
        (dest / "last-cleanup.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(f"Quality cleanup: {len(report['removed'])} removed, {len(report['corrected'])} readings corrected, {report['reviewed_long_count']} reviewed long expressions")
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
