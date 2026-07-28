# 内置语料来源与生成记录

status: verified

`pinyin-ime/data/corpus.txt` 不是独立下载的语料，而是 `build.py` 的 `ensure_corpus()`
从项目启用的非英文 `lexicon/` 词库中生成的缓存。生成过程读取词条和权重，按来源校准
权重、去重并写出结果；`corpus.txt.source.sha256` 保存参与生成的词库指纹。

- 当前生成文件 SHA-256：`a33e156daa193373de0279e446adf487cd32d1db91886ff76348fb918b27b204`
- 当前源指纹：`6f0d866f2bd7d12865fd1cde733c10ffa4eaafa382be514ef1bde419fb503f1b`
- 生成命令：`python build.py --rebuild-corpus --package-variants ime`
- 许可证：输出继承其各输入词库的许可证和署名要求，具体见 `THIRD_PARTY_NOTICES.md`

生成缓存由 `.gitignore` 排除；公开仓库应从有明确来源记录的词库重新生成。只要任何
启用词库仍处于授权阻断状态，正式发行也应视为阻断。
