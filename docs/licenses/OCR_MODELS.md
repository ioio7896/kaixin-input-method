# OCR 模型来源记录

status: verified

当前开发机上的两个模型与 RapidOCR 3.9.0 默认模型清单中的 URL 和 SHA-256 一致：

| 文件 | 上游下载地址 | SHA-256 |
| --- | --- | --- |
| `PP-OCRv6_det_medium.onnx` | `https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.0/onnx/PP-OCRv6/det/PP-OCRv6_det_medium.onnx` | `92078b7355007ccfffcd4c8cd441a3afd4538904d06881b29a155e1e679907c2` |
| `PP-OCRv6_rec_medium.onnx` | `https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.0/onnx/PP-OCRv6/rec/PP-OCRv6_rec_medium.onnx` | `eef444829dbbe18d7fea59a3f6eb75647518d2b3a9568d27c92e42940204894b` |

URL 和预期哈希来自
`RapidOCR-3.9.0/python/rapidocr/default_models.yaml`。ModelScope 上固定到 `v3.9.0`
的模型卡声明 `license: Apache License 2.0`，模型仓库 API 同样返回
`License: Apache License 2.0`。核验日期为 2026-07-26。

正式发布时必须同时保留：

- 模型仓库：`https://www.modelscope.cn/models/RapidAI/RapidOCR`；
- 固定模型卡：`https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.0/README.md`；
- RapidOCR 和 Apache-2.0 许可证声明；
- 上表下载地址、文件名和 SHA-256；
- 本项目未重新训练或转换模型权重的说明。

ONNX 文件仍由 `.gitignore` 排除以控制 Git 仓库体积。构建/发布流程应按固定 URL 下载、
校验 SHA-256，并在发行包中保留上述署名和许可信息。
