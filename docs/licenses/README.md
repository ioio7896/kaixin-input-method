# 第三方来源与许可证记录

本目录保存正式发布需要的可复核来源记录。`status: verified` 表示来源、许可证和哈希
已经由发布负责人核验；`status: blocked` 表示该文件只能保留在开发机上，不能进入公开
Git 历史或正式发布包。

发布前运行：

```powershell
python scripts/check_open_source_readiness.py --release
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/generate_license_reports.ps1
```

生成的依赖报告位于 `docs/licenses/generated/`，默认不提交；正式发行时应把报告复制到
发行资产和安装包的第三方声明目录中。

- [OCR 模型](OCR_MODELS.md)
- [繁简映射数据库](S2T_SOURCE.md)
- [内置语料](CORPUS_SOURCE.md)
- [项目自维护数据](PROJECT_DATA.md)
- [AI 辅助数据来源声明](AI_DATA_PROVENANCE.md)
- [AI 辅助项目数据核验记录](AI_DATA_VERIFICATION.md)
