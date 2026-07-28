# 项目自维护数据来源记录

status: verified

项目作者于 2026-07-27 声明：下列数据由其使用生成式 AI 辅助添加、整理或生成，并由
项目作者选择纳入项目。详细范围和限制见 `docs/licenses/AI_DATA_PROVENANCE.md`。

下列内容在第三方声明中标记为“项目自维护 / AI 辅助”：

- `lexicon/base/*admin*.txt`
- `lexicon/base/hangzhou_*.txt`
- `lexicon/ext/hangzhou_*.txt`
- `data_sources/kaixin/*.tsv`

项目作者进一步确认使用的服务是 OpenAI ChatGPT。历史原始提示、精确模型版本、具体
客户端、首次生成时间和当时是否联网无法完整恢复，未知项没有被推测填写。2026-07-27
已建立重建后的明确任务说明，并对当前冻结版本执行 SHA-256、UTF-8/TSV 结构、数据
行数、空字段、重复组合、拼音音节和代表性杭州词条检查。

逐文件哈希、行数、有意保留的异读、OpenAI 服务条款依据、官方事实核对入口、核验方法
和时效边界见 `docs/licenses/AI_DATA_VERIFICATION.md`。任何内容变更都会使
`python scripts/verify_ai_generated_data.py` 失败，必须重新核验并更新记录。

行政区划和地名属于事实，并不意味着包含其选取、组织或表达的数据集必然没有权利限制。
本次 `verified` 的依据是作者来源声明、当前版本固定哈希、明确的验收规范及人工纳入
决定；它不表示现实名称永久有效，也不免除后续发布者的复核责任。
