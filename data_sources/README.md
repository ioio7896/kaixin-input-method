# 开心输入法数据来源

本目录只保存构建内置词库所需的可追溯原始数据，不会被运行时词库加载器直接扫描。

## THUOCL

- 上游：<https://github.com/thunlp/THUOCL>
- 固定提交：`a30ce79d895d01ab5132a5c74c29703ff7efb4cc`
- 提交日期：2018-11-21
- 许可证：MIT，原文见 `thuocl/LICENSE`
- 下载归档 SHA-256：`8de3dc36e0f6519be4a382cd46b50ec093b2557b339f60e7e3bddc9e49814eed`
- 本项目处理：使用 `build_clean_lexicons` 去重、按原始频次排序，并用 `pinyin` crate 生成无声调拼音；生成文件不修改原始数据文件。

## rust-pinyin

- crate：`pinyin 0.10.0`
- 上游：<https://github.com/mozillazg/rust-pinyin>
- crates.io 校验和：`16f2611cd06a1ac239a0cea4521de9eb068a6ca110324ee00631aa68daa74fc0`
- 许可证：MIT，原文见 `pinyin/LICENSE`
- 本项目处理：用于给 THUOCL 词语生成拼音，并生成 `pinyin_supported_chars.txt`。多音词读音由 `kaixin/polyphone_corrections.tsv` 覆盖。

## wordfreq

- 包：`wordfreq 3.1.1`
- 上游：<https://github.com/rspeer/wordfreq>
- 许可证：Apache License 2.0
- Copyright 2022 Robyn Speer
- 本项目处理：按英文频率顺序选取常用词，过滤非 ASCII、单字母、重复及不适合主动候选的粗俗词，并补充少量电脑/开发常用词，生成 `lexicon/en/kaixin_common_english.txt`。
- 重建：`python -m pip install -r scripts/requirements-lexicon.txt`，然后运行 `python scripts/generate_common_english.py`。

## 自维护数据

- `kaixin/polyphone_corrections.tsv`：多音词校正表。
- `kaixin/common_phrases.tsv`：基础日常候选覆盖表；用于补足分类词表不覆盖的常用词和候选质量回归用例。
- 上述项目自维护表，以及行政区划和杭州本地词库，由项目作者使用生成式 AI 辅助添加、
  整理或生成；声明日期、覆盖范围和仍待补充的生成记录见
  `docs/licenses/AI_DATA_PROVENANCE.md`。

中文词库和字符表可通过 `scripts/regenerate_lexicons.ps1` 重现；英文词库通过 `scripts/generate_common_english.py` 单独重现。
