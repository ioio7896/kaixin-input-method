# 开心输入法英文词库

`kaixin_common_english.txt` 是运行时启用“英语单词输入”后使用的 20,000 词常用英文词库，格式为：

```text
单词<TAB>输入码<TAB>权重
```

词库主体按 `wordfreq 3.1.1` 的英文词频顺序选取，并加入输入法在技术文档、设置和搜索场景常用的少量电脑/开发词。生成过程仅保留 2—24 个 ASCII 小写字母组成的单词，去重并过滤不适合主动候选的粗俗词。权重只表示本词库内的相对顺序，不是原始语料的出现次数。

生成命令：

```powershell
python -m pip install -r scripts/requirements-lexicon.txt
python scripts/generate_common_english.py
```

数据与生成工具来源：

- wordfreq 3.1.1：<https://github.com/rspeer/wordfreq>
- 许可证：Apache License 2.0
- Copyright 2022 Robyn Speer
- 许可证摘要见本目录 `LICENSE.wordfreq.md`

开心输入法对词条进行筛选、加入常用技术词、统一格式和生成权重。该转换不表示 wordfreq 作者认可或赞助开心输入法。
