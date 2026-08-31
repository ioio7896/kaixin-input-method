# 公开仓库与发行资产布局

主仓库只保存开心输入法自研源码、构建脚本和经过再分发核验的小型数据。安装包、PDB、
Python 虚拟环境、OCR 模型和其他大文件不进入 Git 历史。

```text
kaixin-ime/          主源码仓库
Releases/            安装包、模型下载说明、SBOM、许可证报告和 SHA-256
```

不要在尚未确定实际 URL 时提交占位的 `repository` 字段。创建公开仓库后，应补充
`pinyin-ime/Cargo.toml` 的 `repository`，并更新 README 徽章和安全报告入口。
