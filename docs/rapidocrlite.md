# RapidOCRLite 构建说明

OCR 引擎使用 RapidOCRLite 5.0.0a15（上游标签 `v5.0.0a15`），PP-OCRv6 的 small/medium 检测模型、small/medium 识别模型及其字符字典。模型和字典由 `scripts/fetch_rapidocr_models.ps1` 下载并校验；不使用 Indra 模型。

Windows 原生库包含 `scripts/rapidocrlite-windows-utf8.patch`：将模型和字典的 UTF-8 路径通过 `std::filesystem::u8path` 交给 Windows 文件 API。构建时获取对应标签源码，应用补丁，使用 MSVC 和 CMake 编译 x64 Release 共享库，关闭 Vulkan（`PPOCR_ENABLE_VULKAN=OFF`），并设置 C++ 编译选项 `/EHsc /utf-8`；将得到的 `rapidocrlite.dll` 放入 `RapidOCRLite/`。需保留上游 Apache-2.0 许可和本补丁。

OCR 仅使用 CPU；CUDA、DirectML 和 Vulkan 的命令行选项仅保留兼容性。常驻进程按实际模型路径、检测尺寸和识别参数缓存，默认最多 2 套，切换语言不会加载同一模型的副本；淘汰时立即关闭原生句柄。关闭模型常驻选项或退出 OCR 进程会释放缓存。

## 模型分档与小字处理

| 档位 | 检测 | 识别 | 单次推理最长边 | 小字区域放大 | 工作预算 |
| --- | --- | --- | ---: | ---: | ---: |
| 快速 | small | small | 1280 | 不额外放大 | 30 秒 |
| 均衡 | small | medium | 1920 | 1.5 倍 | 60 秒 |
| 高精度 | medium | medium | 2560 | 2 倍 | 90 秒 |

普通 16:9 大截图和长图均分块，分块最长边不超过 1920（快速档 1280），相邻块重叠 128～192 像素。采用等分核心区加边缘重叠，避免刚超过尺寸限制就重复计算几乎整幅图。结果映射回原图坐标；接缝去重优先保留完整句子，能确定前后缀对应的截断行才拼接。同样的文字在不同位置不会去重。

小字分析在最长边不超过 2048 的灰度副本上提取文字形状线索，用原图坐标标出疑似小字区域。只有未覆盖或置信度不足的区域才补识别，已经识别的完整行覆盖其内部小区域。均衡档仍有明显未覆盖区域时，尝试 medium 检测。线索不是识别正确性的证明，不据此删除初次结果。

清晰截图保留颜色。自动对比度增强仅用于有文字线索、低对比度且识别不确定的区域，使用亮度通道 CLAHE；不自动二值化。正确的单字不触发重试。检测阈值仍为 0.3，框阈值 0.5，外扩比例 1.6，默认输出分数阈值 0.5；低置信度结果继续进入校对列表。

每张图最多 32 个主分块、累计 6400 万处理像素。补识别最多 8 次、额外 800 万像素，三种补识别方式共用预算。超出主流程预算明确报错并提示缩小选区；可选补识别超预算时保留已有结果，在 JSON 的 `skipped_passes` 中说明原因。工作进程在原生调用间检查时间，Rust 父进程另有硬超时并终止卡住的进程。

## 预热与调参

预热使用带文字的图像，显式执行检测和识别；同一缓存配置只预热一次。默认最多 4 个 CPU 线程、识别批量 8。`--intra-op-threads` / `KAIXIN_OCR_INTRA_OP_THREADS`、`--rec-batch-num` / `KAIXIN_OCR_REC_BATCH_NUM` 可用于对照测试；`--cache-engines 1` 可减少常驻配置数量。

`--fast-mode` / `KAIXIN_OCR_FAST_MODE=1` 仅在快速档启用 `Rec.fast_mode`。默认关闭，因为缩减行宽填充可能改变识别结果；不能把上游特定设备的提速视为本项目结果。

## 验证

```powershell
python -B -m unittest discover -s scripts -p test_ocr_pipeline.py
python -B scripts/ocr_benchmark.py --tune --repeats 2
```

生成评测图片时，需在执行评测的 Python 环境中安装 Pillow。评测覆盖普通、单字、暗色、低对比度、小字、4K、接缝、长图和空白图。`--baseline <旧辅助脚本>` 可比较旧版本，`--manifest <json>` 可使用人工标注图片。报告包含 CER、整行文本覆盖率、冷启动到预热完成、P50/P95 和 Windows 推理进程峰值工作集。整行覆盖率不等于检测召回率，合成样本不代表真实截图准确率。

生成安装器前，`cargo xtask release` 使用 OCR 暂存包内的 Rust 子进程、DLL 和模型运行三个档位，检查真实文字输出、检测和识别预热、语言缓存复用和两套缓存上限。结果写入 `dist/ocr-validation-kaixin-package-ocr.json`，失败会阻止打包。

参考：[上游模型选型](https://rapidai.github.io/RapidOCRDocs/main/en/model_list/)、[原生识别快速模式](https://github.com/RapidAI/RapidOCRLite#recognition-fast-mode)。
