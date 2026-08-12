# 源程序鉴别材料方案

## 申请口径

源程序鉴别材料建议使用“开心输入法软件 V2.0”自研核心代码，不把第三方 OCR 源码、外部模型、Python 虚拟环境、构建产物、安装包、词库数据或测试数据作为自研源程序提交。

## 建议纳入的自研源码范围

| 路径 | 内容 |
| --- | --- |
| `pinyin-ime/src/` | Rust 拼音引擎、候选排序、用户词库、设置页、托盘、剪贴板、手写、OCR/翻译窗口集成 |
| `pinyin-ime/build.rs` | Rust 构建期数据处理 |
| `tsf-tip/src/` | C++/Win32/TSF 输入法前端、候选窗口、注册逻辑 |
| `tsf-tip/include/` | TSF 前端头文件和接口定义 |
| `tools/*.py` | 自研辅助工具包装脚本 |
| `scripts/*.ps1`、`scripts/*.py` | 自研构建、检查和词库转换脚本，可按需要作为附加源码 |
| `tsf-tip/installer/*.iss` | 安装器脚本，可按需要作为附加源码 |

## 建议排除的内容

| 路径 | 排除原因 |
| --- | --- |
| `RapidOCR-3.9.0/` | 第三方 OCR 项目源码 |
| `.venv-rapidocr/` | 本地 OCR Python 运行时和依赖环境 |
| OCR 模型目录 | OCR 模型数据，不属于自研源程序 |
| `lexicon/` | 内置词库和外部词库数据，不作为程序源码提交 |
| `rocm-python-wheels-Windows*` | 临时 Python wheel 缓存 |
| `build/`、`dist/`、`pinyin-ime/target/` | 构建产物和安装包 |
| `.git/`、`.idea/`、`.vscode/` | 版本库和编辑器状态 |

## 页面要求和生成方式

登记材料通常按源程序前、后各连续 30 页提交；不足 60 页则提交全部。排版时建议每页不少于 50 行，并在页眉中统一写明“开心输入法软件 V2.0 源程序鉴别材料”。

建议先生成一份连续的自研源程序汇总文本，再从该文本中取前 1500 行和后 1500 行排版。这样可以保证材料连续、可复现，也能避免选到第三方源码。

示例生成思路：

```powershell
$outDir = "docs\soft-copyright\generated"
New-Item -ItemType Directory -Force $outDir | Out-Null
$files = @(
  "pinyin-ime\src\lib.rs",
  "pinyin-ime\src\core.rs",
  "pinyin-ime\src\engine.rs",
  "pinyin-ime\src\dict.rs",
  "pinyin-ime\src\segment.rs",
  "pinyin-ime\src\core\lookup.rs",
  "pinyin-ime\src\core\ranking.rs",
  "pinyin-ime\src\core\learning.rs",
  "pinyin-ime\src\user_dict.rs",
  "pinyin-ime\src\config_schema.rs",
  "pinyin-ime\src\bin\srf_ime_settings.rs",
  "pinyin-ime\src\bin\srf_ime_tray.rs",
  "pinyin-ime\src\bin\srf_ime_clipboard.rs",
  "pinyin-ime\src\bin\srf_ime_ocr.rs",
  "tsf-tip\include\srf_tip.h",
  "tsf-tip\src\srf_tip.cpp",
  "tsf-tip\src\srf_tip_parts\srf_tip_key_input.ipp",
  "tsf-tip\src\srf_tip_parts\srf_tip_candidates.ipp",
  "tsf-tip\src\srf_tip_parts\srf_tip_commit.ipp",
  "tsf-tip\src\candidate_window.cpp",
  "tsf-tip\src\candidate_ui.cpp",
  "tsf-tip\src\register.cpp"
)
foreach ($file in $files) {
  "===== FILE: $file =====" | Add-Content "$outDir\source-program-v2.0.txt" -Encoding UTF8
  Get-Content $file | Add-Content "$outDir\source-program-v2.0.txt" -Encoding UTF8
}
```

生成后再把 `source-program-v2.0.txt` 转成 Word/PDF，设置等宽字体、行号或固定每页 50 行，并导出前后各 30 页。

## 版本一致性检查

提交前请确认以下位置版本一致：

- `VERSION`
- `build.py`
- `pinyin-ime/Cargo.toml`
- `pinyin-ime/Cargo.lock`
- `README.md`
- 软著申请表
- 源程序材料页眉
- 用户手册封面和页眉
