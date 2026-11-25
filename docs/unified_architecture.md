# 统一边界与包体维护

- `shared/config_schema.json` 是配置键、默认值和类型的唯一来源；`shared/game_rules.json` 是游戏识别名单的唯一来源。修改后运行 `python scripts/generate_shared_contracts.py`，开发检查使用同一脚本的 `--check` 模式。安装器、Rust 设置页与 C++ 读取器均使用生成结果；已有用户配置与扩展键保留。
- Cargo 工作区包含 `kaixin-common`（契约与共享规则）、`kaixin-core`（查询、学习与词库引擎）、`pinyin-ime`（可执行程序及桌面界面）和 `kaixin-release`（构建发布工具）。依赖版本及共用特性在根 `Cargo.toml` 中定义。引擎和后台服务使用 `--no-default-features` 构建；界面依赖通过 `desktop` 功能启用。
- Rust 源码分别归属各 crate 的 `src/`；核心模型生成由 `kaixin-core/build.rs` 负责，应用图标和版本资源由 `pinyin-ime/build.rs` 负责。`pinyin-ime/src/facade.rs` 保留桌面程序的兼容导出。
- `shared/engine_contract.json` 统一候选容量、输入模式和学习标志；`generate_shared_contracts.py` 同时生成 Rust/C++ 定义。
- `cargo xtask` 调用 `kaixin-release` 调度开发构建、暂存、性能检查和双安装器发布。Windows 暂存、签名及安装器生成由同一套 PowerShell 实现处理。CMake 的 `srf_tip_shared` 由 TSF DLL、Overlay 和原生测试共用。
- 引擎通过原子指针发布词库和学习状态快照，查询连接独立持有解码缓存及只读冷词库连接。词典学习和管理进入同一个写入队列，持久化成功后发布新快照。
- 用户词库和剪贴板历史由引擎进程独占。每次变更追加认证加密记录，达到阈值后生成检查点。DPAPI 保护日志密钥，旧 SQLite 存储不参与启动读取。
- TSF 的物理焦点代次与输入会话代次分别管理。结束会话会使旧查询、按键、提交和候选回调失效；恢复不能重新执行旧会话任务。
- 查询最终展示经过 `core/presentation.rs` 的同一入口，统一稳定词排序、冷词降权、用户精确排序和数量限制。
- 安装包的 `lexicon.pack` 保存两个独立无损 zlib 流，保留标准与热门权重差异。加载器按配置只解压所需流，并兼容旧二进制词库。打包时逐字节比对原数据。
- OCR 通过 `shared/package_trim.json` 的明确名单裁剪无关工具、样例与人脸分类器，保留全部识别模型、可视化字体、运行库及许可。
- 每次成功发布两种安装器后，保留最近三批历史安装器。`scripts/maintain_workspace.ps1` 默认只预览；`-Apply` 清理已知旧 CMake 输出；`-CleanRustDebug` 可额外删除可重建的 Rust 调试缓存。保留当前发布目录、词库源数据、用户数据与 OCR 构建依赖。

验证：共享契约检查、核心排序和压缩损坏测试、设置配置兼容测试、x64/x86 原生策略测试、包体清单以及 OCR 实际识别。安装器打包不执行宿主注册或安装。
