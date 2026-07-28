# 为开心输入法做贡献

感谢你帮助改进开心输入法。提交代码即表示你有权提交相关内容，并同意按项目的
Apache-2.0 许可证授权贡献。复制或改编第三方代码、模型、字体、词库或其他数据时，
必须在 Pull Request 中提供来源、版本、许可证、修改说明和可复核哈希。

## 开发环境

项目当前只支持在 Windows 上完整构建。需要 Python 3、Rust 1.88 或更高版本、
MSVC C++ 工具链、CMake 3.20+ 和 PowerShell。打包还需要 Inno Setup 6；OCR 和
ShareX 构建有各自的附加依赖。

建议从独立分支开始，保持一次提交只解决一个主题。不要提交 `dist/`、虚拟环境、
诊断包、日志、PDB、签名证书、用户数据库或其他本机生成内容。

## 验证

提交前至少运行：

```powershell
python scripts/check_open_source_readiness.py
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-fast.ps1
```

修改 Rust 引擎时运行：

```powershell
cargo test --manifest-path pinyin-ime/Cargo.toml --lib --locked
```

修改 C++/TSF 代码时运行相应 CMake/CTest 测试；修改安装或第三方组件时应运行完整
验证和安装冒烟测试。需要交互桌面的测试若被跳过，请在 PR 中说明。

## Pull Request 要求

- 说明问题、解决方案、用户可见变化和验证结果。
- 新行为应包含测试；不能测试时说明原因和人工验证步骤。
- 不改变隐私默认值，除非 PR 明确说明风险和迁移方案。
- 不在测试、日志或截图中包含真实输入、剪贴板、用户名或本地路径。
- 依赖变化须更新锁文件和第三方许可证报告。
- 第三方内容须更新 `THIRD_PARTY_NOTICES.md` 及 `docs/licenses/` 中的来源记录。
- 修改 ShareX fork 时，更新其 `SOURCE_INFO.md`，并验证对应源码包和发布二进制一致。

项目维护者可能要求拆分过大的 PR，或拒绝来源、授权和隐私边界无法核验的内容。
