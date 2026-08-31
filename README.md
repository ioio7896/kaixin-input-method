# 开心输入法

<p align="center">
  <img src="assets/kaixin-input-icon.png" alt="开心输入法图标" width="128">
</p>

<p align="center">
  面向 Windows 10/11 的本地优先中文拼音输入法
</p>

<p align="center">
  <a href="https://github.com/ioio7896/kaixin-input-method/releases">
    <img alt="GitHub Release" src="https://img.shields.io/github/v/release/ioio7896/kaixin-input-method?include_prereleases">
  </a>
  <a href="LICENSE">
    <img alt="License" src="https://img.shields.io/badge/license-Apache--2.0-blue">
  </a>
  <img alt="Windows" src="https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078D6">
  <img alt="Architecture" src="https://img.shields.io/badge/architecture-x64%20%2B%20x86-lightgrey">
</p>

开心输入法是一款面向 Windows 的本地中文输入法。

C++/Win32 前端通过 Windows TSF（Text Services Framework）接入系统输入法，Rust 进程负责拼音解析、候选排序、用户学习、剪贴板管理、手写和本地 OCR 等功能。

项目重点关注：

- **本地优先**：拼音计算、词库、用户学习、手写和 OCR 均在本机运行。
- **行为可控**：学习、剪贴板采集、功能热键、应用排除和诊断级别均可配置。
- **隐私安全**：敏感输入保护、本地数据库加密、IPC 鉴权和进程加固。
- **可验证**：提供词库来源、构建流程、候选质量和安装检查。

当前版本见 [`VERSION`](VERSION)。

> [!IMPORTANT]
> 开心输入法的拼音核心和本地 OCR 不会上传输入内容。
>
> 翻译功能通过用户另行安装的 WinTranslator 提供。该功能是否联网以及如何处理数据，由 WinTranslator 自身决定。本仓库不包含翻译模型或翻译运行时。

## 下载与安装

请前往 [Releases](https://github.com/ioio7896/kaixin-input-method/releases) 下载最新安装包。

| 安装包 | 内容 | 适合用户 |
|---|---|---|
| `kaixin-setup-ime-*.exe` | 拼音输入法及本地工具，不含 OCR 运行时 | 只需要输入法的用户 |
| `kaixin-setup-ocr-*.exe` | 输入法、本地 OCR 运行时和 OCR 模型 | 需要截图识别的用户 |

如果 Releases 页面暂时没有安装包，请按照[从源码构建](#从源码构建)一节自行构建。

安装完成后：

1. 打开 Windows 输入法切换面板。
2. 选择“开心输入法”。
3. 如果没有立即显示，请注销并重新登录。
4. 仍未显示时，请检查 Windows 语言设置中是否已安装“中文（简体，中国）”。

默认安装包按机器范围安装，可能需要管理员权限。也可以从源码生成仅安装到当前用户的安装包。

> [!WARNING]
> 安装从互联网下载的未签名程序时，Windows SmartScreen 可能显示警告。请确认文件来自本项目 Releases 页面，并核对发布页面提供的 SHA-256。

## 界面预览

建议在 `docs/images/` 中放入实际截图后启用以下内容：

```markdown
| 拼音候选窗口 | 设置界面 |
|---|---|
| ![拼音候选窗口](docs/images/candidate-window.png) | ![设置界面](docs/images/settings.png) |

| 剪贴板管理器 | 本地 OCR |
|---|---|
| ![剪贴板管理器](docs/images/clipboard.png) | ![本地 OCR](docs/images/ocr.png) |
```

## 功能概览

### 拼音输入

- 全拼、简拼以及全拼与声母混拼。
- 整句、长词、短词和单字候选联合排序。
- 字符 Bigram、词频、用户频次、上下文和近期选择信号。
- 邻键、漏键、调序和近音等输入纠错。
- 可选模糊音：
  - `z/zh`
  - `c/ch`
  - `s/sh`
  - `n/l`
  - `f/h`
  - `an/ang`
  - `en/eng`
  - `in/ing`
- 内置双拼方案。
- 简体和繁体输出。
- 英文词候选。
- 日期、时间、数字、符号和 Emoji 直输。
- 自定义短语。
- 用户词条学习、置顶、移除、导入、导出和清空。

### 候选窗口

- 横排或竖排布局。
- 每页显示 3～9 个候选。
- 经典、紧凑和卡片布局。
- 字体、字号、字重和透明度设置。
- 浅色、深色、高对比度和多种内置主题。
- 鼠标选择、滚轮翻页。
- 候选来源和读音调试信息。
- 全屏及游戏应用候选覆盖层。
- 按应用设置候选策略、缩放、显示器和提交方式。

### 本地工具

- **剪贴板管理器**：搜索、置顶、复制、粘贴、快粘和清空文本历史。
- **手写查字**：通过独立画布查找汉字，支持复制或直接粘贴。
- **截图**：使用 Windows Graphics Capture，支持智能框选、自由区域和当前窗口截图。
- **OCR**：基于本地 RapidOCR 和 ONNXRuntime，支持预处理、二次框选、历史记录和结果整理。
- **翻译联动**：通过当前用户命名管道连接外部 WinTranslator。
- **托盘与设置**：集中管理输入、外观、工具、应用兼容和隐私选项。

## 快速使用

| 操作 | 默认按键 |
|---|---|
| 输入拼音 | `A-Z` |
| 选择候选 | `1-9` |
| 提交首选 | `Space` / `Enter` |
| 取消当前输入 | `Esc` |
| 删除输入 | `Backspace` |
| 向前或向后翻页 | `-` / `=`、`,` / `.`、`PageUp` / `PageDown` |
| 轻按切换中英文 | `Shift` |

截图、剪贴板、设置、手写、OCR 和翻译等全局热键默认关闭，请在设置界面按需启用，避免与其他程序冲突。

## VV 直输助手

输入 `vv` 加命令可以生成常用文本或打开本地工具。

| 命令 | 功能 | 示例 |
|---|---|---|
| `vv rq` / `vv date` | 日期 | `vv rq mingtian` |
| `vv sj` / `vv time` | 时间 | `vv sj` |
| `vv xq` / `vv week` | 星期 | `vv xq` |
| `vv num` / `vv upper` | 数字转换 | `vv num 12345` |
| `vv roman` / `vv hex` | 罗马数字或进制转换 | `vv hex 255` |
| `vv percent` / `vv bytes` | 百分比或容量格式 | `vv bytes 1048576` |
| `vv calc` / `vv convert` | 计算和单位换算 | `vv calc (2+3)*4` |
| `vv sym` / `vv fh` | 符号 | `vv sym punct` |
| `vv emoji` / `vv face` | Emoji | `vv emoji smile` |
| `vv dx` / `vv rmb` | 人民币大写 | `vv rmb 123.45` |
| `vv cb` / `vv clip` | 剪贴板候选 | `vv cb` |
| `vv hw` / `vv handwrite` | 手写查字 | `vv hw` |
| `vvu` | 打开剪贴板管理器 | `vvu` |

`rq`、`sj` 等常用命令也可以不加 `vv` 直接输入。

相对日期支持：

- `jintian`
- `mingtian`
- `houtian`
- `zuotian`
- `+N`
- `-N`
- `xiazhouyi`
- `monthstart`
- `monthend`
- `nextmonthstart`
- `nextmonthend`

更多命令和配置说明见[配置项参考](docs/config_reference.md)。

## 自定义短语

示例：

```text
;mail = name@example.com
;;r = 此致，敬礼
;sig = 此致\n敬礼
```

保存后，可以输入 `;mail`、`;;r`、`;sig` 或 `vv ;mail` 调出对应内容。

请勿把包含真实密码、令牌或敏感个人信息的短语文件提交到公开仓库。

## 隐私与网络行为

### 网络行为

| 功能 | 默认联网 | 是否上传输入内容 |
|---|---:|---:|
| 拼音输入与候选计算 | 否 | 否 |
| 用户词条学习 | 否 | 否 |
| 剪贴板管理 | 否 | 否 |
| 手写查字 | 否 | 否 |
| 本地 OCR | 否 | 否 |
| OCR 模型下载脚本 | 是 | 不上传输入内容 |
| 外部 WinTranslator | 取决于外部程序 | 由外部程序决定 |
| GitHub 源码及 Release 下载 | 是 | 不属于输入法运行过程 |

项目不包含遥测、广告 SDK 或云端用户学习服务。

### 安全默认值

- 剪贴板后台采集默认关闭。
- 所有工具的全局热键默认关闭。
- 运行诊断默认只记录 `error` 级别。
- 密码输入框和常见密码管理器进程自动视为敏感上下文。
- 可以按进程配置：
  - 永不学习
  - 永不读取剪贴板
  - 永不显示候选
- 全局隐私模式会：
  - 强制 ASCII 输入
  - 隐藏候选
  - 停止用户学习
  - 禁止剪贴板访问

建议的高隐私配置：

```ini
[privacy]
enabled=1

[clipboard]
background_enabled=0

[diagnostics]
log_level=error
```

### 本地数据

| 数据 | 默认位置 | 保护方式 |
|---|---|---|
| 配置 | `%LOCALAPPDATA%\kaixin\kaixin.ini` | 当前用户目录 ACL，内容为明文 |
| IPC capability | `%LOCALAPPDATA%\kaixin\engine_capability.dat` | DPAPI 和文件 ACL |
| 用户词库 | `%LOCALAPPDATA%\kaixin\user_dict.sqlite` | 整库 DPAPI |
| 剪贴板历史 | `%LOCALAPPDATA%\kaixin\clipboard_store.sqlite` | 整库 DPAPI |
| OCR 历史 | `%LOCALAPPDATA%\kaixin\ocr_history.sqlite` | 整库 DPAPI |
| 截图库索引 | `%LOCALAPPDATA%\kaixin\screenshot_library.sqlite` | 整库 DPAPI |
| 运行事件 | `%LOCALAPPDATA%\kaixin\runtime_events.sqlite` | 消息及详情字段 DPAPI |
| 截图图片 | 用户设置目录 | 普通 PNG/JPEG，不加密 |
| 诊断日志 | `%LOCALAPPDATA%\kaixin\logs` | 可能包含明文诊断信息 |

截图文件本身不会被 DPAPI 加密，请将截图保存目录视为敏感数据目录。

分享诊断信息前，请检查日志、窗口标题、进程路径和截图路径是否包含个人信息。

更完整的安全边界和配置说明见：

- [安全策略](SECURITY.md)
- [配置项参考](docs/config_reference.md)

## 剪贴板说明

剪贴板后台采集默认关闭。

启用后：

- 默认最多保留 60 条普通记录。
- 默认最多保留 24 条置顶记录。
- 单条文本最多保留 20,000 个 UTF-16 单元。
- 默认不按时间自动过期。
- 默认不记录来源进程。
- 默认不在候选元数据中展示内容预览。

即使后台采集关闭，主动打开剪贴板管理器、执行刷新或使用 `vvu` 时，程序仍会读取一次当前系统文本剪贴板。

全局隐私模式会同时禁止后台采集和按需读取。

## OCR 与截图

OCR 版安装包包含运行所需的 RapidOCR、Python 运行时依赖和 OCR 模型。

从源码准备模型：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/fetch_rapidocr_models.ps1
```

脚本会下载并校验所需模型。

默认保存目录：

| 内容 | 默认目录 |
|---|---|
| 普通截图 | `%USERPROFILE%\Pictures\Kaixin Screenshots` |
| OCR 截图 | `%USERPROFILE%\Pictures\Kaixin OCR` |

两个目录均可在设置中修改。

OCR 模型来源、哈希和许可信息见 [OCR 模型说明](docs/licenses/OCR_MODELS.md)。

## 系统要求

正式支持：

- Windows 10
- Windows 11
- x64 系统

安装包包含 64 位和 32 位 TSF/辅助组件，用于兼容不同位数的 Windows 应用程序。

暂不正式支持：

- Windows Server
- Windows on ARM
- Wine
- 早于 Windows 10 的系统

## 架构

```text
Windows 应用 / TSF 宿主
        │
        ▼
tsf-tip/srf_tsf_tip.dll
C++：按键处理、composition、候选窗口和应用兼容
        │
        │ 本地命名管道 + capability
        ▼
srf_ime_engine.exe
Rust：拼音解析、候选排序、用户学习、词库和剪贴板查询
        │
        ├── srf_ime_settings.exe
        ├── srf_ime_tray.exe
        ├── srf_ime_clipboard.exe
        ├── srf_ime_handwrite.exe
        ├── srf_ime_ocr.exe
        └── srf_ime_translate_result.exe
                    │
                    └──► 外部 WinTranslator
```

候选窗口通常由 TSF 进程内渲染。对于全屏或无 UI 宿主，可以按策略切换到独立候选覆盖层。

## 从源码构建

构建脚本仅支持 Windows。

### 基础依赖

- Python 3
- Rust/Cargo
- MSVC Rust 工具链
- `i686-pc-windows-msvc` Rust 目标
- CMake 3.20 或更高版本
- Visual Studio C++/MSVC 构建工具
- PowerShell
- Inno Setup 6（仅生成安装包时需要）

OCR 版本还需要：

- RapidOCR
- Python OCR 运行环境
- OCR 模型

### 完整构建

```powershell
python build.py
```

默认同时构建纯输入法版和 OCR 版，并执行 Rust/C++ 正确性检查。

### 仅构建纯输入法版

```powershell
python build.py --package-variants ime
```

### 常用参数

```powershell
python build.py --debug
python build.py --clean
python build.py --no-inno
python build.py --user-installer
python build.py --package-variants ime
python build.py --package-variants ocr
python build.py --portable-zip
python build.py --perf-smoke
python build.py --dry-run
```

`--quick` 只复用已有 Rust/C++ 产物重新执行 staging 和打包，不适合首次构建或正式发布。

执行以下命令查看全部参数：

```powershell
python build.py --help
```

### 代码签名

正式发布建议强制代码签名：

```powershell
$env:KX_SIGN_CERT_SHA1 = "<certificate thumbprint>"
python build.py --sign --sign-required
```

也可以通过 `KX_SIGN_PFX` 及对应密码环境变量使用 PFX 证书。

请勿将 PFX 文件、证书密码、私钥或环境配置提交到 Git。

### 构建产物

```text
dist/kaixin-setup-ime-<version>-<timestamp>.exe
dist/kaixin-setup-ocr-<version>-<timestamp>.exe
dist/kaixin-package-ime/
dist/kaixin-package-ocr/
dist/kaixin-package-*.zip
```

## 验证

快速验证：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-fast.ps1
```

完整验证：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-full.ps1
```

Rust 检查：

```powershell
cd pinyin-ime
cargo check --all-targets
cargo build --bin srf_ime_engine
target\debug\srf_ime_engine.exe --install-health-check --probe nihao
```

候选质量检查：

```powershell
cd pinyin-ime
cargo run --bin phrase_len_eval -- --limit 500
```

性能检查：

```powershell
python build.py --perf-smoke
```

依赖交互桌面的截图和 Windows Graphics Capture 测试，在无桌面环境中会自动跳过。

## 词库

| 目录 | 内容 |
|---|---|
| `lexicon/zh` | 中文主词库和单字表 |
| `lexicon/zh-ext` | 分类、地域和纠音等扩展词库 |
| `lexicon/en` | 可重建的英文词库 |
| `data_sources` | 上游数据、许可证、哈希和生成说明 |

中文词库格式：

```text
词语<TAB>拼音<TAB>权重
```

英文词库格式：

```text
word<TAB>input-code<TAB>weight
```

来源和重建说明：

- [词库数据来源](data_sources/README.md)
- [英文词库说明](lexicon/en/README.md)
- [第三方组件声明](THIRD_PARTY_NOTICES.md)

## 主题

主题位于：

```text
skins/<id>/theme.json
```

内置主题包括：

- `light`
- `dark`
- `cherry-pop`
- `forest-ink`
- `high-visibility`
- `ink-violet`
- `mint-glass`
- `moon-ink`
- `neon-night`
- `nordic-frost`
- `paper-latte`
- `retro-terminal`
- `rose-gold`
- `sea-salt`
- `sunlit-amber`

主题格式见[皮肤主题规范](docs/skin_theme_spec.md)。

## 仓库结构

```text
assets/          应用图标和静态资源
data_sources/    词库上游数据、许可和生成说明
docs/            配置、皮肤、质量评测和开发文档
icons/           Windows 多尺寸图标
lexicon/         运行时中文及英文词库
pinyin-ime/      Rust 引擎、GUI 工具和评测程序
scripts/         验证、词库生成和安装检查脚本
skins/           候选窗口主题
tools/           本地 OCR 辅助脚本
tsf-tip/         C++ TSF 前端、候选覆盖层和安装器
build.py         构建、验证、签名和打包入口
```

构建产物、安装包、虚拟环境、本机编辑器设置和本地隐私数据不会纳入源码仓库。

## 文档

- [配置项参考](docs/config_reference.md)
- [皮肤主题规范](docs/skin_theme_spec.md)
- [高 DPI/GDI 检查清单](docs/high_dpi_gdi_checklist.md)
- [TSF 前端说明](tsf-tip/README.md)
- [仓库与发布资产布局](docs/repository_layout.md)
- [第三方组件声明](THIRD_PARTY_NOTICES.md)
- [贡献指南](CONTRIBUTING.md)
- [安全策略](SECURITY.md)

## 已知限制

- 仅正式支持 Windows 10 和 Windows 11。
- 部分截图测试需要交互式桌面。
- OCR 版依赖体积较大的第三方运行时和模型。
- 截图图片是普通 PNG/JPEG 文件，不会自动加密。
- 管理员权限或已经入侵的同用户会话不在项目的隐私隔离边界内。
- 外部 WinTranslator 的联网和数据处理行为不由本项目控制。
- 未提供已核验的 `s2t_chars.sqlite` 时，源码构建会停用简繁映射并保持原文输出。

## 安全问题

输入法会在本地处理用户输入、剪贴板和截图。

请不要在公开 Issue 中提交：

- 真实输入内容
- 用户词库
- 剪贴板数据库
- OCR 历史数据库
- 诊断数据库或日志
- 包含隐私信息的截图
- API 密钥、令牌、证书或私钥

安全漏洞请按照 [SECURITY.md](SECURITY.md) 中的方式私下报告。

## 贡献

欢迎提交问题、修复和改进。

提交代码前请阅读：

- [贡献指南](CONTRIBUTING.md)
- [行为准则](CODE_OF_CONDUCT.md)
- [安全策略](SECURITY.md)

提交内容不得包含本机路径、真实用户数据、私钥、证书、访问令牌或其他个人信息。

## 许可证

开心输入法自行开发的代码和文档采用 [Apache License 2.0](LICENSE) 授权。

RapidOCR、OCR 模型、词库、手写数据以及其他第三方内容不自动适用 Apache License 2.0。详细许可边界见：

- [LICENSE_SCOPE.md](LICENSE_SCOPE.md)
- [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)
- [NOTICE](NOTICE)

正式发布的安装包和便携包应同时提供 SHA-256 校验值以及对应的第三方许可说明。

---

如果这个项目对你有帮助，欢迎提交 Issue、改进代码或为仓库点一个 Star。
