本输入法一切东西都留在本机win系统内。下载源代码之前可以通过你的AI进行一次源代码审计，放心再用。谢谢





# 开心输入法

![开心输入法图标](assets/kaixin-input-icon.png)

开心输入法是一款面向 Windows 的本地中文输入法。C++/Win32 前端通过 Windows
TSF（Text Services Framework）接入系统输入法，Rust 进程负责拼音解析、候选排序、
用户学习和辅助工具。

项目强调三个方向：

- **本地优先**：拼音计算、词库、手写和 OCR 均在本机运行。
- **行为可控**：学习、剪贴板采集、功能热键、应用排除和诊断级别均可配置。
- **可验证**：词库来源、构建流程、候选质量、IPC 和候选窗均有检查。

当前版本为 **2.0.0**，目标平台为 64 位 Windows；安装包同时包含 64 位和 32 位
TSF/辅助组件，以适配不同位数的宿主程序。

当前支持 Windows 10 和 Windows 11。Windows Server、Windows on ARM、Wine
以及早于 Windows 10 的版本不在正式支持范围内。

> [!IMPORTANT]
> 拼音核心和本地 OCR 不会上传输入内容。翻译功能通过另行安装的 WinTranslator
> 完成，是否联网及其数据处理方式由 WinTranslator 自身决定；本仓库不打包翻译模型
> 或翻译运行时。

## 功能概览

### 拼音输入

- 全拼、简拼和全拼/声母混拼。
- 整句、长词、短词和单字候选联合排序。
- 字符 Bigram、词频、用户频次、上下文和近期选择信号。
- 邻键、漏键、调序和近音等输入纠错。
- `z/zh`、`c/ch`、`s/sh`、`n/l`、`f/h`、`an/ang`、`en/eng`、
  `in/ing` 等可选模糊音。
- 内置双拼方案、简繁输出和自定义短语。
- 英文词候选、日期时间直输、符号和 Emoji。
- 用户词条学习、置顶、移除、导入、明文导出和清空。

### 候选窗口与兼容性

- 竖排或横排，每页 3～9 项。
- 经典、紧凑、卡片布局及多种密度。
- 字体、字号、字重、透明度、浅色、深色、高对比度和材质设置。
- 鼠标提交、滚轮翻页、增强定位和候选来源/读音调试信息。
- 全屏及游戏应用候选覆盖层。
- 按应用设置候选策略、覆盖层位置、缩放、显示器和提交方式。
- TSF、Unicode SendInput、剪贴板粘贴等提交兼容方案。

### 本地工具

- **剪贴板管理器**：搜索、置顶、复制、粘贴、快粘和清空文本历史。
- **手写查字**：独立画布、候选复制和直接粘贴。
- **截图**：ShareX 集成或原生 Windows Graphics Capture，支持区域和窗口截图。
- **OCR**：本地 RapidOCR/ONNXRuntime，支持预处理、二次框选、历史和结果整理。
- **翻译联动**：通过当前用户命名管道连接外部 WinTranslator。
- **托盘与设置**：统一管理输入、外观、工具、应用兼容和隐私选项。

## 架构

```text
Windows 应用 / TSF 宿主
        │
        ▼
tsf-tip/srf_tsf_tip.dll       C++：按键、composition、候选窗、应用兼容
        │  本地命名管道 + capability
        ▼
srf_ime_engine.exe            Rust：解析、候选、学习、词库和剪贴板查询
        │
        ├── srf_ime_settings.exe
        ├── srf_ime_tray.exe
        ├── srf_ime_clipboard.exe
        ├── srf_ime_handwrite.exe
        ├── srf_ime_ocr.exe
        └── srf_ime_translate_result.exe ──► 外部 WinTranslator
```

候选窗口通常由 TSF 进程内渲染；全屏或无 UI 宿主可按策略切换到独立的
`srf_ime_overlay.exe`。

## 安装

完整构建会在 `dist/` 生成两类安装包：

```text
kaixin-setup-ime-<version>-<timestamp>.exe   纯输入法
kaixin-setup-ocr-<version>-<timestamp>.exe   输入法 + 本地 OCR
```

运行所需安装包，完成后从 Windows 输入法切换面板选择“开心输入法”。如果输入法没有
立即出现在列表中，可注销并重新登录，或在 Windows 语言设置中检查
“中文（简体，中国）”下的键盘。

默认生成机器范围安装包。开发者也可使用 `python build.py --user-installer` 生成
当前用户安装包。

卸载默认保留用户数据。静默卸载可使用：

- `/RemoveTransientUserData=1`：删除缓存和日志等临时数据。
- `/RemoveUserData=1` 或 `/DeleteUserData=1`：删除配置和全部用户数据。

## 基本使用

| 操作 | 默认按键 |
| --- | --- |
| 输入拼音 | `a-z` |
| 选择候选 | `1-9` |
| 提交首选 | `Space` / `Enter` |
| 取消 composition | `Esc` |
| 删除输入 | `Backspace` |
| 翻页 | `-` / `=`、`,` / `.`、`PageUp` / `PageDown` |
| 轻按切换中英文 | `Shift` |

截图、剪贴板、设置、手写、OCR 和翻译等全局功能热键**默认关闭**，请在设置页按需
启用，避免与其他程序冲突。

## VV 直输助手

输入 `vv` 加命令可生成常用文本或打开工具：

| 命令 | 功能 | 示例 |
| --- | --- | --- |
| `vv rq` / `vv date` / `vv jr` | 当前或相对日期 | `vv rq mingtian`、`vv rq +3` |
| `vv sj` / `vv time` | 当前时间 | `vv sj` |
| `vv xq` / `vv week` / `vv zhou` | 星期 | `vv xq` |
| `vv num` / `vv upper` / `vv full` | 中文数字、大写或全角转换 | `vv num 12345` |
| `vv roman` / `vv hex` | 罗马数字或进制转换 | `vv hex 255` |
| `vv percent` / `vv bytes` | 百分比或容量格式 | `vv bytes 1048576` |
| `vv calc` / `vv convert` | 计算与单位换算 | `vv calc (2+3)*4` |
| `vv sym` / `vv fh` | 符号 | `vv sym punct` |
| `vv emoji` / `vv face` | Emoji | `vv emoji smile` |
| `vv unit` / `vv dw` | 常用单位 | `vv unit` |
| `vv dx` / `vv rmb` / `vv money` | 人民币大写 | `vv rmb 123.45` |
| `vv mail` / `vv email` | 邮箱片段 | `vv mail` |
| `vv url` / `vv site` | 网址片段 | `vv url` |
| `vv md` / `vv markdown` | Markdown 片段 | `vv md` |
| `vv cb` / `vv clip` / `vv paste` | 剪贴板候选 | `vv cb` |
| `vv hw` / `vv handwrite` / `vv sx` | 手写查字 | `vv hw` |
| `vvu` | 打开剪贴板管理器/快粘 | `vvu` |

`rq`、`sj` 等也可以不加 `vv` 直接输入。相对日期支持 `jintian`、`mingtian`、
`houtian`、`zuotian`、`+N`、`-N`、`xiazhouyi`、`monthstart`、
`monthend`、`nextmonthstart` 和 `nextmonthend` 等写法。

自定义短语示例：

```ini
;qq = name@example.com
;;r = 此致，敬礼
;sig = 此致\n敬礼
```

保存后可输入 `;qq`、`;;r`、`;sig` 或 `vv ;qq` 调出。

## 剪贴板

剪贴板后台采集默认关闭。启用后：

- 默认最多保留 60 条普通记录和 24 条置顶记录。
- 单条文本最多 20,000 个 UTF-16 单元。
- 默认不按时间自动过期。
- 默认不记录来源进程，也不在候选元数据中展示内容预览。

即使后台采集关闭，主动打开剪贴板管理器、刷新或使用 `vvu` 仍会读取一次当前系统
文本剪贴板。全局隐私模式会同时禁止后台采集和按需披露。

## OCR 与截图

OCR 安装包要求以下内容完整存在于安装目录：

```text
RapidOCR-3.9.0/
.venv-rapidocr/
tools/kaixin_ocr_engine.py
RapidOCR-3.9.0/python/rapidocr/models/PP-OCRv6_det_medium.onnx
RapidOCR-3.9.0/python/rapidocr/models/PP-OCRv6_rec_medium.onnx
```

程序会根据 `package_manifest.sha256` 校验模型哈希。可选的 small/INT8 检测模型存在时
可用于快速档，否则使用 PP-OCRv6 medium。

截图默认保存到 `%USERPROFILE%\Pictures\Kaixin Screenshots`，OCR 截图默认保存到
`%USERPROFILE%\Pictures\Kaixin OCR`；两者都可在设置中修改。截图图片本身是普通
PNG/JPEG 文件，不由 DPAPI 加密，请将保存目录视为敏感数据目录。

## 隐私与安全

### 安全默认值

- 剪贴板后台采集默认关闭。
- 所有工具全局热键默认关闭。
- 运行诊断默认仅记录 `error` 级别。
- 密码输入框和常见密码管理器进程自动视为敏感上下文。
- 可为进程配置“永不学习”“永不读取剪贴板”“永不显示候选”。
- `[privacy] enabled=1` 会强制 ASCII、隐藏候选、停止学习并禁止剪贴板访问。

建议的隐私配置：

```ini
[privacy]
enabled=1

[clipboard]
background_enabled=0

[diagnostics]
log_level=error
```

### 本地数据

| 数据 | 默认位置 | Windows 上的保护 |
| --- | --- | --- |
| 配置 | `%LOCALAPPDATA%\kaixin\kaixin.ini` | 当前用户目录 ACL；内容为明文 |
| IPC capability | `%LOCALAPPDATA%\kaixin\engine_capability.dat` | DPAPI + 文件 ACL |
| 用户词库 | `%LOCALAPPDATA%\kaixin\user_dict.sqlite` | 整库 DPAPI |
| 剪贴板历史 | `%LOCALAPPDATA%\kaixin\clipboard_store.sqlite` | 整库 DPAPI |
| OCR 历史 | `%LOCALAPPDATA%\kaixin\ocr_history.sqlite` | 整库 DPAPI |
| 截图库索引 | `%LOCALAPPDATA%\kaixin\screenshot_library.sqlite` | 整库 DPAPI |
| 运行事件 | `%LOCALAPPDATA%\kaixin\runtime_events.sqlite` | 消息及详情字段 DPAPI |
| 截图图片 | 用户设置目录 | 普通 PNG/JPEG，不加密 |
| 诊断日志 | `%LOCALAPPDATA%\kaixin\logs` | 可能包含明文诊断信息 |

当前开发格式会拒绝旧版明文用户词库、剪贴板、OCR 和截图库数据库，不再自动读取或
迁移。升级本开发版本前，如不需要旧数据，可关闭输入法相关进程后删除对应旧文件。

### IPC 与进程加固

- 引擎命名管道拒绝远程客户端。
- 管道 DACL 仅允许 SYSTEM、管理员和当前登录会话。
- Lookup、学习、剪贴板解析等命令统一校验 DPAPI capability。
- Rust GUI/工具进程限制 DLL 搜索路径，并启用严格句柄、扩展点禁用和镜像加载策略。
- MSVC 目标启用 `/GS`、`/sdl`、Control Flow Guard、DEP、ASLR、CET 和高熵 ASLR。

这些措施主要防止其他 Windows 账户、远程管道客户端、离线复制和普通未授权调用。
它们不能防止已经以同一 Windows 用户身份运行的恶意程序读取剪贴板、模拟输入或调用
该用户可解密的 DPAPI 数据；管理员权限和已入侵的同用户会话也不在此隔离边界内。

分享诊断包之前，应检查日志、窗口标题、进程路径和截图路径是否含有隐私信息。

## 主题

主题位于 `skins/<id>/theme.json`。当前内置：

`light`、`dark`、`cherry-pop`、`forest-ink`、`high-visibility`、
`ink-violet`、`mint-glass`、`moon-ink`、`neon-night`、`nordic-frost`、
`paper-latte`、`retro-terminal`、`rose-gold`、`sea-salt`、
`sunlit-amber`。

主题格式参见 [皮肤主题规范](docs/skin_theme_schema.md)。

## 从源码构建

构建脚本仅支持 Windows。基础依赖：

- Python 3
- Rust/Cargo（MSVC 工具链及 `i686-pc-windows-msvc` 目标）
- CMake 3.20 或更高版本
- Visual Studio C++/MSVC 构建工具
- PowerShell
- Inno Setup 6（生成安装包时）
- OCR 变体所需的本地 RapidOCR、Python 环境及模型

完整 Release 构建默认同时构建 `ime` 和 `ocr` 变体，并执行 Rust/C++ 正确性检查：

```powershell
python build.py
```

没有 OCR 运行时或只需要输入法时：

```powershell
python build.py --package-variants ime
```

常用参数：

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

`--quick` 仅复用已有 Rust/C++ 产物重新 stage/package，不适合首次或正式构建。
正式发布建议强制代码签名：

```powershell
$env:KX_SIGN_CERT_SHA1 = '<certificate thumbprint>'
python build.py --sign --sign-required
```

也可通过 `KX_SIGN_PFX` 和相关密码环境变量使用 PFX；执行
`python build.py --help` 查看当前参数。

主要产物：

```text
dist/kaixin-setup-ime-<version>-<timestamp>.exe
dist/kaixin-setup-ocr-<version>-<timestamp>.exe
dist/kaixin-package-ime/
dist/kaixin-package-ocr/
dist/kaixin-package-*.zip
```

## 验证

项目级验证：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-fast.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-full.ps1
```

Rust：

```powershell
cd pinyin-ime
cargo check --all-targets
cargo build --bin srf_ime_engine
target\debug\srf_ime_engine.exe --install-health-check --probe nihao
```

性能与运行时检查：

```powershell
cd pinyin-ime
cargo run --bin phrase_len_eval -- --limit 500
```

性能检查：

```powershell
python build.py --perf-smoke
```

需要交互桌面的截图/WGC 测试在无桌面环境中会跳过。

## 词库

| 目录 | 内容 |
| --- | --- |
| `lexicon/zh` | 中文主词库：热门基础候选和单字表，候选排序优先级较高 |
| `lexicon/zh-ext` | 中文扩展词库：分类、地域、纠音等补充词，频率会校准且排序优先级较低 |
| `lexicon/en` | 可重建的 20,000 词英文词库 |
| `data_sources` | 固定上游版本、许可证、哈希和生成说明 |

中文词库格式：

```text
词语<TAB>拼音<TAB>权重
```

英文词库格式：

```text
word<TAB>input-code<TAB>weight
```

来源和重建说明参见 [data_sources/README.md](data_sources/README.md)、
[lexicon/en/README.md](lexicon/en/README.md) 与
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 仓库结构

```text
assets/          应用图标
data_sources/    词库上游数据、许可和生成说明
docs/            配置、皮肤、质量评测和开发文档
icons/           Windows 多尺寸图标
lexicon/         运行时中文及英文词库
pinyin-ime/      Rust 引擎、GUI 工具和评测程序
scripts/         验证、词库生成和安装检查脚本
skins/           候选窗主题
tools/           本地 OCR 辅助脚本
tsf-tip/         C++ TSF 前端、候选覆盖层和安装器
build.py         一键构建、验证、stage、签名和打包
```

## 相关文档

- [配置项参考](docs/config_reference.md)
- [皮肤主题规范](docs/skin_theme_schema.md)
- [高 DPI / GDI 检查清单](docs/high_dpi_gdi_checklist.md)
- [第三方组件声明](THIRD_PARTY_NOTICES.md)
- [TSF 前端说明](tsf-tip/README.txt)
- [公开仓库与发行资产布局](docs/repository_layout.md)

## 安全问题

输入法会在本地处理用户输入、剪贴板和截图。请不要在公开 Issue 中粘贴输入内容、
诊断数据库、日志或截图。安全漏洞请按照 [SECURITY.md](SECURITY.md) 提供的方式私下
报告。

## 许可证

开心输入法自行开发的代码和文档采用
[Apache License 2.0](LICENSE) 授权。ShareX、RapidOCR、OCR 模型、词库和手写数据等
第三方内容不自动适用该许可证，详细边界见 [LICENSE_SCOPE.md](LICENSE_SCOPE.md) 和
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

正式发布的安装包、便携包和 ShareX 对应源码包应同时提供 SHA-256 校验值。项目的
已知限制包括：仅支持 Windows；部分截图测试需要交互式桌面；OCR 变体依赖体积较大的
第三方运行时及模型；未提供已核验的 `s2t_chars.sqlite` 时源码构建会停用简繁映射并
保持原文输出；管理员权限或已被入侵的同用户会话不在本项目的隐私隔离边界内。

欢迎提交问题和改进，开发及提交要求见 [CONTRIBUTING.md](CONTRIBUTING.md)。
