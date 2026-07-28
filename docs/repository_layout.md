# 公开仓库与发行资产布局

主仓库只保存开心输入法自研源码、构建脚本和经过再分发核验的小型数据。安装包、PDB、
Python 虚拟环境、OCR 模型和其他大文件不进入 Git 历史。

修改后的 ShareX 是 GPL-3.0 独立程序。建议把 `third_party/ShareX/` 迁移到独立公开
fork，并在主仓库中固定到具体提交；确定代码托管组织和仓库 URL 后，可以使用 Git
submodule，或让构建脚本按提交和 SHA-256 下载源码。迁移前，现有 `build.py` 会为每个
发布二进制生成独立的 `kaixin-sharex-corresponding-source.zip`。

```text
kaixin-ime/          主源码仓库
kaixin-sharex/       ShareX GPL-3.0 fork 和本地修改历史
Releases/            安装包、模型下载说明、SBOM、许可证报告和 SHA-256
```

不要在尚未确定实际 URL 时提交占位的 `repository` 字段。创建公开仓库后，应补充
`pinyin-ime/Cargo.toml` 的 `repository`，更新 README 徽章和安全报告入口，再将 ShareX
fork 的固定 URL/提交写入构建配置。
