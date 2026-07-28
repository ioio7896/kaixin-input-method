# 繁简映射数据库来源记录

status: verified

开发文件：`pinyin-ime/data/s2t_chars.sqlite`

SHA-256：`f09b4aa90b1569a22c49d222e7930bf98406b81a1a12c581a6b67f9e3915cc3a`

项目作者于 2026-07-27 声明：该数据库是其使用生成式 AI 辅助添加并纳入项目的内容，
不是从当前仓库中某个已登记的第三方数据集直接转换而来。详细声明见
`docs/licenses/AI_DATA_PROVENANCE.md`。

项目作者进一步确认使用的服务是 OpenAI ChatGPT。历史原始提示、精确模型版本、具体
客户端和首次生成时间无法完整恢复，未知项没有被推测填写。2026-07-27 已按重建后的
明确任务说明完成冻结版本验收：SQLite `integrity_check` 通过，共 2714 行，
`simplified` 无重复，`sort_order` 为 0 至 2713 的连续唯一值，字段无空值。

完整的任务说明、OpenAI 服务条款依据、核验方法和固定文件清单见
`docs/licenses/AI_DATA_VERIFICATION.md`。执行
`python scripts/verify_ai_generated_data.py` 可复核数据库身份和结构。

这里的 `verified` 表示来源声明、发布依据、固定哈希和验收过程已经留档；不表示生成式
AI 输出当然准确或当然不存在任何第三方权利。该固定版本可以进入公开 Git 历史和发行
包；修改数据库后必须重新核验并更新本记录。
