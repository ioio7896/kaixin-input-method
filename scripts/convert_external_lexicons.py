#!/usr/bin/env python3
from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC_DIR = ROOT / "外挂词库"
OUT_DIR = ROOT / "lexicon" / "ext"
DEFAULT_FREQ = 5000

LEXICONS = [
    ("765个世界主要城市.txt", "world_major_cities.txt", "765个世界主要城市外挂词库"),
    ("ICD-10疾病编码1.txt", "icd10_diseases.txt", "ICD-10疾病编码外挂词库"),
    ("成语俗语.txt", "chengyu_suyu.txt", "成语俗语外挂词库"),
    ("古诗词名句【官方推荐】.txt", "poetry_quotes.txt", "古诗词名句外挂词库"),
    ("国家和地区词库.txt", "countries_regions.txt", "国家和地区外挂词库"),
    ("杭州地名街道名.txt", "hangzhou_places_streets.txt", "杭州地名街道名外挂词库"),
    ("杭州风景名胜.txt", "hangzhou_scenic.txt", "杭州风景名胜外挂词库"),
    ("杭州公交站名.txt", "hangzhou_bus.txt", "杭州公交站名外挂词库"),
    ("杭州市城市信息精选.txt", "hangzhou_city_info.txt", "杭州市城市信息精选外挂词库"),
    ("杭州市地铁站名.txt", "hangzhou_metro.txt", "杭州市地铁站名外挂词库"),
    ("杭州市公交站名.txt", "hangzhou_city_bus.txt", "杭州市公交站名外挂词库"),
    ("杭州所有的小区.txt", "hangzhou_residential_communities.txt", "杭州所有小区外挂词库"),
    ("日常大词库.txt", "daily_large.txt", "日常大词库外挂词库"),
    ("生活、工作常用词.txt", "common_life_work.txt", "生活工作常用词外挂词库"),
    ("生活常用成语大全.txt", "common_chengyu.txt", "生活常用成语外挂词库"),
    ("宋词精选【官方推荐】.txt", "songci_quotes.txt", "宋词精选外挂词库"),
    ("县级行政区.txt", "county_admin.txt", "县级行政区外挂词库"),
    ("医生常用词汇.txt", "medical_common.txt", "医生常用词汇外挂词库"),
    ("浙江地名.txt", "zhejiang_places.txt", "浙江地名外挂词库"),
    ("浙江县市区名.txt", "zhejiang_county_city_district.txt", "浙江县市区名外挂词库"),
    ("政府机关.txt", "government_org.txt", "政府机关外挂词库"),
    ("中国风景名胜.txt", "china_scenic.txt", "中国风景名胜外挂词库"),
]

PINYIN_RE = re.compile(r"^[A-Za-züÜvV][A-Za-züÜvV']*$")


def normalize_pinyin(raw: str) -> str:
    return raw.strip().lower().replace("ü", "v").replace("'", " ")


def parse_line(line: str) -> tuple[str, str] | None:
    stripped = line.strip().lstrip("\ufeff")
    if not stripped or stripped.startswith("#"):
        return None
    parts = stripped.rsplit(None, 1)
    if len(parts) != 2 or not PINYIN_RE.fullmatch(parts[1]):
        raise ValueError(line.rstrip("\n"))
    word = parts[0].strip()
    pinyin = normalize_pinyin(parts[1])
    if not word or not pinyin:
        raise ValueError(line.rstrip("\n"))
    return word, pinyin


def convert_one(src_name: str, out_name: str, title: str) -> int:
    src = SRC_DIR / src_name
    out = OUT_DIR / out_name
    if not src.is_file():
        raise FileNotFoundError(src)

    seen: set[tuple[str, str]] = set()
    entries: list[str] = []
    for line_no, line in enumerate(src.read_text(encoding="utf-8-sig").splitlines(), 1):
        try:
            parsed = parse_line(line)
        except ValueError as exc:
            raise ValueError(f"{src_name}:{line_no}: cannot parse: {exc}") from exc
        if parsed is None:
            continue
        if parsed in seen:
            continue
        seen.add(parsed)
        word, pinyin = parsed
        entries.append(f"{word}\t{pinyin}\t{DEFAULT_FREQ}")

    header = [
        "# encoding: utf-8",
        f"# {title}",
        "# Generated from 外挂词库/*.txt by scripts/convert_external_lexicons.py.",
        "# 格式：词条<TAB>拼音<TAB>词频；需要调频时修改第三列数字。",
        "",
    ]
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(header + entries) + "\n", encoding="utf-8", newline="\n")
    return len(entries)


def main() -> int:
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, OSError):
        pass

    total = 0
    for src_name, out_name, title in LEXICONS:
        count = convert_one(src_name, out_name, title)
        total += count
        print(f"{out_name}\t{count}")
    print(f"total\t{total}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
