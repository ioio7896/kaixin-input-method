# 开心输入法第三方组件声明

本文件说明开心输入法软件中集成或随包分发的第三方组件、模型和外部数据。开心输入法的自研范围包括拼音输入引擎、候选排序与学习逻辑、Windows TSF 前端、候选窗口、设置页、托盘工具、剪贴板管理、截图/OCR/翻译交互集成、安装部署和本地隐私数据管理。

OCR 引擎、OCR 模型、Python 运行时依赖、系统 API、外部词库和第三方手写数据不属于开心输入法自研源程序成果。本文件不改变任何第三方许可条款；使用、复制、修改和再分发相关组件时，应遵守其原始许可证。翻译由用户另行安装的 WinTranslator 提供，不随开心输入法分发。

## 第三方作者权利保护方式

开心输入法使用第三方开源作品时，应通过以下方式保护原作者声明的权利：

1. 保留原作者、项目名称、版权声明、许可证名称和许可证链接。
2. 随安装包或便携包分发本文件，并保留上游项目自带的 `LICENSE`、README、NOTICE 或模型卡说明。
3. 对修改、转换或重新打包的内容作出说明，例如“随本地运行时打包”“未修改上游源码”等。
4. 不把第三方项目、模型或数据描述为开心输入法自研成果。
5. 不使用第三方项目名称、作者名称、机构名称或商标暗示其认可、赞助或背书开心输入法。
6. 对 CC-BY-4.0 模型给出署名、许可证链接、原始模型链接和修改/转换说明。
7. 保留免责声明，不向用户承诺第三方组件提供超出原许可证的担保。

## 主要第三方作品的开源声明

| 第三方作品 | 许可证 | 你可以做什么 | 需要保留/声明什么 |
| --- | --- | --- | --- |
| ShareX 21.0.0 | GPL-3.0 | 可使用、研究、修改和再分发；本项目使用其截图与图像编辑功能 | 分发修改版二进制时同时提供完整对应源码、GPL-3.0 许可和修改说明；不得把 ShareX 描述为本项目自研 |
| RapidOCR | Apache License 2.0 | 可使用、复制、修改和再分发工程代码；可随商业软件分发 | 保留 Apache-2.0 许可证、版权和 NOTICE/README；说明 OCR 模型版权另属模型方；如修改 RapidOCR 源码，标明修改 |
| THUOCL | MIT License | 可使用、修改和再分发词表 | 保留 THUOCL 的 MIT 许可证及上游来源；本项目固定版本和归档哈希见 `data_sources/README.md` |
| chinese-surnames 1.0.0 | MIT License | 可使用、修改和再分发姓氏列表 | `lexicon/zh/chinese_surnames.txt` 为其 `index.json` 的拼音化派生结果；保留 `data_sources/chinese_surnames/LICENSE` 及上游来源 |
| rust-pinyin 0.10.0 | MIT License | 可使用、修改和再分发代码 | 保留 MIT 许可证和版权声明；本项目用于生成无声调拼音及支持字符表 |
| wordfreq 3.1.1 | Apache License 2.0 | 可使用、修改和再分发代码与随包数据 | 保留版权、许可证和来源；本项目仅派生并分发筛选后的 20,000 词英文排序 |

## OCR 功能

OCR 功能集成第三方开源项目 RapidOCR 及相关本地 OCR 模型。开心输入法自研部分包括截图交互、窗口界面、参数配置、OCR 结果处理、历史记录、剪贴板联动、输入法集成、安装部署和隐私数据管理；不主张 RapidOCR 工程源码、OCR 模型及 Python 运行时依赖的著作权。

| 组件 | 用途 | 许可/来源说明 |
| --- | --- | --- |
| RapidOCR (`RapidOCR-3.9.0/`) | 本地 OCR 引擎 | Apache License 2.0；随包保留 `RapidOCR-3.9.0/LICENSE`、README 和相关说明 |
| PP-OCRv6 ONNX 模型 | 屏幕文字识别 | RapidAI/RapidOCR v3.9.0 模型仓库声明 Apache-2.0；固定 URL、模型卡和哈希见 `docs/licenses/OCR_MODELS.md`；不作为开心输入法自研成果主张 |
| onnxruntime 1.27.0 | ONNX 模型推理 | MIT License |
| opencv-python 4.13.0.92 | 图像处理 | Apache License 2.0 |
| numpy 2.5.0 | 数值计算 | 第三方 Python 依赖，发布前保留对应许可证记录 |
| Pillow 12.2.0 | 图像读写 | 第三方 Python 依赖，发布前保留对应许可证记录 |
| pyclipper 1.4.0 | 几何裁剪 | MIT License |
| PyYAML 6.0.3 | 配置解析 | MIT License |
| Shapely 2.1.2 | 几何处理 | BSD 3-Clause License |

Rust GUI 依赖 `epaint_default_fonts` 随程序嵌入 Ubuntu Font Family、Noto Emoji、
Hack 和 emoji icon font。依赖许可证报告覆盖 MIT/Apache-2.0、SIL OFL-1.1 和
Ubuntu Font Licence 1.0；后者原文保存在
`docs/licenses/distribution/Ubuntu-Font-License-1.0.txt` 并随正式包分发。

## 截图功能

默认原生截图路径使用 Windows Graphics Capture；当 WGC 不可用或捕获失败时，可回退到 DXGI Desktop Duplication。ShareX 21.0.0 作为用户可选的截图与编辑器，提供二次框选、裁剪和标注。随包版本基于上游提交 `73967140f4fd64ca4b93203ae8ad5ac05ade9aaf`，增加了仅供开心输入法调用的区域截图和指定窗口截图命令，并在集成模式下关闭 ShareX 自身的托盘图标、全局热键、上传和自动保存任务。最终图像仅在本机处理，再由开心输入法按用户设置复制、保存或交给 OCR。

| 组件 | 用途 | 许可/来源说明 |
| --- | --- | --- |
| xcap 0.9.6 | Windows Graphics Capture 的 Rust 封装和 GPU 纹理读取 | Apache License 2.0；上游 <https://github.com/nashaofu/xcap> |
| dxgi-capture-rs 1.2.2 | DXGI Desktop Duplication 截图回退 | MIT License；Copyright (c) 2025 RobbyV2；上游 <https://github.com/RobbyV2/dxgi-capture-rs> |
| Windows Graphics Capture / DXGI | Windows 系统截图 API | 操作系统平台 API，不属于开心输入法自研源程序成果 |

ShareX 使用 GPL-3.0 许可证。完整对应源码、许可证、上游来源、提交版本、下载归档哈希及本地修改说明由发布目录中的 `kaixin-sharex-corresponding-source.zip` 单独提供；安装目录的 `ShareX/SOURCE_INFO.md` 也会说明源码获取方式。运行文件位于 `ShareX/` 目录。ShareX 名称和商标归其权利人所有，集成不表示 ShareX Team 对开心输入法的认可或背书。

## 翻译联动

开心输入法仅通过当前用户命名管道把 JSON 请求发送给独立安装的 WinTranslator，不再分发翻译运行时或模型。WinTranslator 的依赖和模型许可由其自身发行包负责。

## 其他外部数据

| 组件或路径 | 用途 | 许可/来源说明 |
| --- | --- | --- |
| `lexicon/en/` | 20,000 词常用英文词库 | 高频顺序来自 wordfreq 3.1.1（Apache-2.0），并由开心输入法筛选及补充常用技术词；目录内保留 README 与许可证摘要 |
| `lexicon/zh/life_common_4char.txt`、`lexicon/zh/life_common_phrases_5to8.txt`、`lexicon/zh/chat_common_phrases.txt`、`lexicon/zh/office_common_phrases.txt` | 现代生活、聊天和办公短语 | 词频排序来自 wordfreq 3.1.1（Apache-2.0）；项目脚本补充并整理现代固定搭配，生成方法见 `scripts/build_common_phrase_lexicons.py` |
| `lexicon/zh/single_char_common_8105.txt` | 8105 个常用汉字单字词库 | 按 wordfreq 3.1.1（Apache-2.0）中文词频排序，并与项目拼音支持表取交集；生成方法见 `scripts/generate_single_char_lexicon.py` |
| `lexicon/zh/ai_and_machine_learning.txt`、`software_and_cloud.txt`、`internet_products.txt`、`programming_frameworks.txt` | 现代 AI、软件云、互联网产品和编程框架实体 | 词频排序来自 wordfreq 3.1.1（Apache-2.0）；项目脚本补充现代技术实体和中英混合输入码，生成方法见 `scripts/build_modern_it_lexicons.py` |
| `data_sources/thuocl/`、`lexicon/zh/thuocl_*.txt` | 中文分类词库及其生成结果 | THUOCL，MIT License；固定上游提交、归档哈希和许可证原文均保存在 `data_sources/` |
| `lexicon/zh/chinese_surnames.txt` | 中国单姓、复姓及姓氏异读 | `chinese-surnames` 1.0.0，MIT License；上游来源、版本和许可证原文见 `data_sources/README.md` 与 `data_sources/chinese_surnames/LICENSE` |
| `data_sources/pinyin/`、`pinyin-ime/data/pinyin_supported_chars.txt` | 拼音生成与支持字符表 | rust-pinyin 0.10.0，MIT License；字符表由构建工具重新生成 |
| `data_sources/kaixin/*.tsv`、`lexicon/zh/kaixin_polyphone.txt` | 基础候选、多音词读音修正及其生成结果 | 开心输入法项目作者使用生成式 AI 辅助添加并自行维护；见 `docs/licenses/AI_DATA_PROVENANCE.md` |
| `lexicon/zh/kaixin_common.txt` | 基础中文词库 | 从许可明确的 THUOCL 数据及项目自维护基础候选经过去重、排序后构建 |
| `lexicon/zh/*admin*.txt`、`lexicon/zh/hangzhou_*.txt` | 行政区划及杭州本地词库 | 项目作者使用生成式 AI 辅助添加并自行维护；发布前补齐模型、提示输入、服务条款及事实核验记录 |
| `pinyin-ime/src/handwrite_lookup/` | 手写查字匹配和数据 | 目录内已有 LGPL/APL 许可说明，发布时应保留 |
| Windows TSF / Win32 API | 系统输入法接入 | 平台 API，不属于开心输入法自研源程序主张范围 |

## 发布前核验清单

- 保留 RapidOCR 的 `LICENSE`、README 和第三方模型说明；模型核验状态见
  `docs/licenses/OCR_MODELS.md`。
- OCR 模型的固定 URL、哈希及模型仓库 Apache-2.0 声明已经记录；正式发布时仍需保留
  模型卡、许可证和未修改说明。
- 生成 Rust 依赖许可证报告，例如通过 `cargo-about` 或 `cargo-deny`。
- 生成 Python 依赖许可证报告，覆盖 `.venv-rapidocr` 和 `.python-runtime` 中随包分发的依赖。
- 保留 `data_sources/thuocl/LICENSE`、`data_sources/pinyin/LICENSE` 和 `data_sources/README.md`。
- 确认 `pinyin-ime/data/s2t_chars.sqlite` 与 `docs/licenses/S2T_SOURCE.md` 中已核验的
  固定哈希一致。
- 确认行政区划、杭州词库和其他自维护数据与
  `docs/licenses/PROJECT_DATA.md`、`docs/licenses/AI_DATA_VERIFICATION.md` 中已核验的
  固定哈希一致。

## 参考链接

- RapidOCR: https://github.com/RapidAI/RapidOCR
- ShareX: https://github.com/ShareX/ShareX
- THUOCL: https://github.com/thunlp/THUOCL
- rust-pinyin: https://github.com/mozillazg/rust-pinyin
