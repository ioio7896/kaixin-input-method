# 统一边界与包体维护

- `shared/config_schema.json` 是配置键、默认值和类型的唯一来源；`shared/game_rules.json` 是游戏识别名单的唯一来源。修改后运行 `python scripts/generate_shared_contracts.py`，构建时自动检查生成文件是否同步。安装器、Rust 设置页与 C++ 读取器均使用生成结果；已有用户配置与扩展键保留。
- Cargo 工作区包含 `kaixin-common`（契约与共享规则）、`kaixin-core`（查询、学习与词库引擎）和 `pinyin-ime`（可执行程序及桌面界面）。引擎和后台服务使用 `--no-default-features` 构建；界面依赖通过 `desktop` 功能启用。
- TSF 的物理焦点代次与输入会话代次分别管理。结束会话会使旧查询、按键、提交和候选回调失效；恢复不能重新执行旧会话任务。
- 查询最终展示经过 `core/presentation.rs` 的同一入口，统一稳定词排序、冷词降权、用户精确排序和数量限制。
- 安装包的 `lexicon.pack` 保存两个独立无损 zlib 流，保留标准与热门权重差异。加载器按配置只解压所需流，并兼容旧二进制词库。打包时逐字节比对原数据。
- OCR 通过 `shared/package_trim.json` 的明确名单裁剪无关工具、样例与人脸分类器，保留全部识别模型、可视化字体、运行库及许可。
- 每次成功发布两种安装器后，保留最近三批历史安装器。`scripts/maintain_workspace.ps1` 默认只预览；`-Apply` 清理已知旧 CMake 输出；`-CleanRustDebug` 可额外删除可重建的 Rust 调试缓存。保留当前发布目录、词库源数据、用户数据与 OCR 构建依赖。

验证：共享契约检查、核心排序和压缩损坏测试、设置配置兼容测试、x64/x86 原生策略测试、包体清单以及 OCR 实际识别。安装器打包不执行宿主注册或安装。
