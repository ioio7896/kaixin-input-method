# 开心输入法数据来源

本目录只保存构建内置词库所需的可追溯原始数据，不会被运行时词库加载器直接扫描。

## rust-pinyin

- crate：`pinyin 0.10.0`
- 上游：<https://github.com/mozillazg/rust-pinyin>
- crates.io 校验和：`16f2611cd06a1ac239a0cea4521de9eb068a6ca110324ee00631aa68daa74fc0`
- 许可证：MIT，原文见 `pinyin/LICENSE`
- 本项目处理：用于给项目自维护词语生成拼音，并生成 `pinyin_supported_chars.txt`。多音词读音由 `kaixin/polyphone_corrections.tsv` 覆盖。

## chinese-surnames

- 上游 npm 包：<https://github.com/stevemao/chinese-surnames>
- 版本：`1.0.0`（npm 归档：`chinese-surnames-1.0.0.tgz`）
- 许可证：MIT，原文见 `data_sources/chinese_surnames/LICENSE`
- 本项目处理：从上游 `index.json` 生成 `data_sources/lexicon_fragments/zh-ext/chinese_surnames.txt`，补充无声调普通话拼音及姓氏专用异读；合并后的运行时词库为 `lexicon/zh-ext/people_names.txt`，同一姓氏的多个读音分别保留。

## wordfreq

- 包：`wordfreq 3.1.1`
- 上游：<https://github.com/rspeer/wordfreq>
- 许可证：Apache License 2.0
- Copyright 2022 Robyn Speer
- 本项目处理：按英文频率顺序选取常用词，过滤非 ASCII、单字母、重复及不适合主动候选的粗俗词，并补充少量电脑/开发常用词，生成 `lexicon/en/kaixin_common_english.txt`。
- 重建：`python -m pip install -r scripts/requirements-lexicon.txt`，然后运行 `python scripts/generate_common_english.py`。

## 自维护数据

- `kaixin/polyphone_corrections.tsv`：用于给其他词库生成标准拼音的多音词校正表；可选第三列为 `exact` 或 `substring`，省略时按 `substring` 处理。
- `kaixin/pronunciation_aliases.tsv`：运行时可接受读音与多音字表；同一个词语或单字可以用不同拼音和独立权重出现多次，不会传播到其他词条。
- `kaixin/common_phrases.tsv`：基础日常候选覆盖表；用于补足分类词表不覆盖的常用词和候选质量回归用例。
- `lexicon_fragments/zh-ext/animal_common_5000.txt`：项目作者使用生成式 AI 辅助生成、整理并人工筛选的
  动物候选池；运行时 `animals.txt` 只取高频优先的前 500 条，权重统一为 6000。
- 上述项目自维护表，以及行政区划和杭州本地词库，由项目作者使用生成式 AI 辅助添加、
  整理或生成；声明日期、覆盖范围和仍待补充的生成记录见
  `docs/licenses/AI_DATA_PROVENANCE.md`。

`lexicon_fragments/zh-ext/` 保存分类词库的可追溯源片段；
`scripts/merge_zh_ext_lexicons.py` 按“词语 + 输入码”去重、同项保留最高权重，生成
`lexicon/zh-ext/` 下 7 个用户可关闭的分类词库。

项目自维护中文词库和字符表可通过 `scripts/regenerate_lexicons.ps1` 重现；英文词库通过
`scripts/generate_common_english.py` 单独重现。

8105 个常用单字词库由 `scripts/generate_single_char_lexicon.py` 基于固定版本
wordfreq 3.1.1 的中文词频顺序生成，并仅保留 `pinyin_supported_chars.txt`
中引擎可读的汉字。权重按频率名次严格降序映射到 10000～1。

现代短语词库由 `scripts/build_common_phrase_lexicons.py` 从 wordfreq 中文频率排序
和项目维护的聊天/办公常用表达生成；通用词库输出到 `lexicon/zh/`，聊天和办公源片段
写入 `lexicon_fragments/zh-ext/`，再合并为 `lexicon/zh-ext/daily_communication.txt`。

现代 AI、软件云、互联网产品和编程框架词库由
`scripts/build_modern_it_lexicons.py` 从 wordfreq 中文频率排序及项目维护的技术实体
生成，源片段写入 `lexicon_fragments/zh-ext/` 并合并为
`lexicon/zh-ext/technology.txt`；其中 ASCII/中英混合实体使用显式输入码保留大小写、
符号和产品名写法。
