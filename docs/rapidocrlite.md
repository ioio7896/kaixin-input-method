# RapidOCRLite 构建说明

OCR 引擎使用 RapidOCRLite 5.0.0a15（上游标签 `v5.0.0a15`），PP-OCRv6 的 small/medium 检测模型、medium 识别模型及其字符字典。模型和字典由 `scripts/fetch_rapidocr_models.ps1` 下载并校验；不使用 Indra 模型。

Windows 原生库包含 `scripts/rapidocrlite-windows-utf8.patch`：将模型和字典的 UTF-8 路径通过 `std::filesystem::u8path` 交给 Windows 文件 API。构建时获取对应标签源码，应用补丁，使用 MSVC 和 CMake 编译 x64 Release 共享库，关闭 Vulkan（`PPOCR_ENABLE_VULKAN=OFF`），并设置 C++ 编译选项 `/EHsc /utf-8`；将得到的 `rapidocrlite.dll` 放入 `RapidOCRLite/python/rapidocr/`。需保留上游 Apache-2.0 许可和本补丁。

OCR 仅使用 CPU；模型缓存保留至 OCR 进程退出或用户关闭模型常驻选项。
