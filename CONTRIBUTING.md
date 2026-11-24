# 为开心输入法做贡献

感谢你帮助改进开心输入法。提交代码即表示你有权提交相关内容，并同意按项目的
Apache-2.0 许可证授权贡献。复制或改编第三方代码、模型、字体、词库或其他数据时，
必须在 Pull Request 中提供来源、版本、许可证、修改说明和可复核哈希。

## 开发环境

项目当前只支持在 Windows 上完整构建。需要 Python 3.12+、Rust 1.99.0、
MSVC C++ 工具链、CMake 3.20+ 和 PowerShell。打包还需要 Inno Setup 6。OCR
使用原生 RapidOCRLite DLL 和模型，桌面辅助程序均随 Rust 工作区构建。

首次开发运行 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/setup_dev_tools.ps1`，
将固定版本的 Ruff、clang-format 和 PSScriptAnalyzer 安装到 `dist/dev-tools/`。
正式检查缺少工具或版本不符时必须失败，不能静默跳过。

## 代码约定

- 源码使用 UTF-8、LF；`.cmd` 使用 CRLF。包含中文的 PowerShell 脚本保留 UTF-8 BOM，兼容 Windows PowerShell 5.1。
- Rust 使用 rustfmt，C++ 使用根目录 `.clang-format`，Python 使用 `pyproject.toml` 中的 Ruff 配置，PowerShell 使用 `scripts/PSScriptAnalyzerSettings.psd1`。
- 生成文件只改源配置或生成器；运行生成器后用 `--check` 验证。第三方源码不参与项目格式化。
- `kaixin-common` 拥有契约与共享规则；`kaixin-core` 拥有引擎及其平台适配；`pinyin-ime` 拥有桌面程序。各 crate 使用自己的源码和构建入口，禁止反向引用应用源码。
- 按职责拆分模块，接口只公开调用方需要的内容。大文件行数用于审查提示，不设机械拆分指标。
- 系统资源使用 RAII 所有权封装；内核句柄复用 `OwnedWinHandle`，GDI、COM 等使用匹配其释放方法的封装。
- 新增 `unsafe` 调用须说明指针有效期、线程要求或所有权等安全前提；公开的 unsafe 函数须有 `# Safety` 文档。
- 关键读写、配置持久化和发布失败必须传播带操作上下文的错误。可选探测失败可降级，但须注明原因；析构清理不能 panic。
- 日志只记录操作、状态和错误类别，沿用隐私过滤规则；不得记录输入原文、剪贴板正文或认证信息。
- lint 例外限制在具体函数或类型，并写出理由；新增例外不能通过全局关闭检查实现。

## 发布

代码修改完成后运行 `build_latest_installers.cmd` 或 `python scripts/release.py`。
该入口一次生成上海时间的时间戳版本，强制源码/Rust/C++ 检查及冒烟检查，产出纯输入法和输入法＋OCR 两个安装器，
核对版本、数量和包体清单并写入 `dist/release-<版本>.json`。签名参数支持 `--sign` 和 `--sign-required`。
明确要求跳过测试时可传 `--skip-tests`，发布报告会记录 `verification: skipped_by_request`；编译及包体完整性校验仍执行。
单组件开发构建继续使用 `build.py`；正式交付必须走双安装器发布入口。

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
cargo check --manifest-path pinyin-ime/Cargo.toml --lib --locked
```

本地、CI 和打包统一调用 `scripts/verify.py`，检查矩阵由 `shared/verification.json` 维护：
`source` 检查格式和契约，`rust` 对无界面/桌面两种配置执行工作区全部目标的 Clippy 与测试，
`native` 构建 CMake 的 `kaixin_check_targets` 并运行 x64/x86 CTest。`verify-full.ps1` 另保留词库质量与性能评估。

修改 C++/TSF 代码时运行相应构建和人工冒烟检查；修改安装或第三方组件时应运行完整
验证和安装冒烟测试。需要交互桌面的测试若被跳过，请在 PR 中说明。

## Pull Request 要求

- 说明问题、解决方案、用户可见变化和验证结果。
- 新行为应说明验证步骤和人工验证结果。
- 不改变隐私默认值，除非 PR 明确说明风险和迁移方案。
- 不在日志或截图中包含真实输入、剪贴板、用户名或本地路径。
- 依赖变化须更新锁文件和第三方许可证报告。
- 第三方内容须更新 `THIRD_PARTY_NOTICES.md` 及 `docs/licenses/` 中的来源记录。

项目维护者可能要求拆分过大的 PR，或拒绝来源、授权和隐私边界无法核验的内容。
