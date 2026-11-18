# 发布与截图恢复验证

发布检查位于 `scripts/release_validation.py`，在生成任何安装器之前执行。引擎的 `--build-info` 和 TSF 的 `KxGetBuildInfo` 返回真实编译进二进制的版本、协议、位数及 Git 标识。协议源为 `shared/engine_protocol_version.txt`，CMake 与 Rust build.rs 共用；Git 为 unknown 时仍严格校验版本和协议。

`srf_release_probe.exe` 分别由 x64 和 Win32 编译，使用生产桥接代码验证握手、能力令牌、词库初始化和首批 `nihao` 候选中的“你好”。构建将暂存引擎、DLL 和词库复制到临时的 `LOCALAPPDATA/Programs/kaixin` 标准目录，逐文件比较 SHA-256，再加载这些一致的副本，保留生产路径信任检查。探针不注册 COM、不启动看门狗、不写 TSF 健康状态。失败或超时也会回收自建进程。报告位于 `dist/release-validation-*.json`。

托盘和 OCR 共用 `screenshot_capture`。每次 WGC/DXGI 尝试运行在独立的 `srf_ime_tray --capture-frame-helper` 进程中，先设置 Per-Monitor V2 DPI 上下文，确保显示器坐标均为物理像素。8 秒超时，结束后回收进程和临时图像。一次原生捕获最多两次尝试；设备失效切换后端，桌面黑帧触发另一后端的新进程复核，双方均为黑色时仍保留合法黑色画面；WGC 单窗口黑帧保持同后端复核，避免混入遮挡窗口。显示器变化或取消中止当前流程，选择器结束后不再次开启回退选择器。框选坐标裁剪也由托盘和 OCR 共用。

DXGI 会跳过仅更新鼠标、未更新桌面的帧，并在暂存纹理映射和读取完成后才释放桌面帧，遵循微软的 [ReleaseFrame 生命周期约定](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutputduplication-releaseframe)及[帧更新标记说明](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/ns-dxgi1_2-dxgi_outdupl_frame_info)。

内部诊断入口 `srf_ime_tray --capture-recovery-check <交接目录>` 可执行整个公共恢复流程，并在 `recovery.json` 返回最终后端和恢复原因；它只写入指定交接目录，不打开框选界面或处理剪贴板。

自动检查覆盖：unknown Git 下版本/协议/架构不一致、缺失组件、合法黑帧、设备失效回退、取消停止回退、负坐标显示器裁剪，以及故障码到修复说明的映射。它们不替代真实显卡和显示器的回归。

截图阶段日志写在 `%LOCALAPPDATA%\kaixin\logs\tray.log`，默认可用。筛选 `event=screenshot_stage`，按同一个 `pid` 和 `capture_id` 查看；`stage_id` 将阶段的 `status=start` 与结束状态配对，`elapsed_ms` 是本阶段耗时，`since_start_ms` 是相对本次截图开始的耗时。只见开始、没有结束时，先检查该阶段。日志不写文件路径、窗口标题、图片或 OCR 文本。

- `grab_wgc` / `grab_dxgi` 包含辅助进程和交接耗时；`overlay_prepare`、`overlay_first_paint` 与 `overlay_select` 分别记录初始化、首绘和等待用户框选的时间。
- `preview_first_paint`、`save_dialog`、`save_prepare`、`save_encode_write`、`save_publish`、`clipboard_encode`、`clipboard_write`、`silent_copy`、`ocr_handoff` 区分结果显示、用户选择文件路径、主文件交付和后续操作。另存对话框等待用户确认的时间与写入时间分开。

手工回归需分别覆盖 Windows 10/11、x64/x86 宿主及以下流程，并保存系统/显卡驱动版本、后端、捕获耗时和日志：

1. 正常截取窗口、跨屏区域、暗色终端和全黑画面，验证保存和剪贴板内容。
2. 睡眠唤醒后立即截图，重复至少五轮；确认后续截图不依赖重启托盘。
3. 截图过程中拔插显示器、切换分辨率/DPI，确认中止提示；重新截图应使用新拓扑。
4. 在取帧期间、蒙层静止时和拖框时再次按截图快捷键；另测 Esc。确认蒙层消失、鼠标恢复、无残留捕获辅助进程，下一次截图正常。
5. 将自动保存或静默副本目录设到受控慢盘或测试共享，确认结果窗可移动、关闭、复制，托盘仍能开始下一次截图。取消另存、保存失败或出现重名后，确认截图仍保留、可用其他路径重试。
6. 模拟引擎文件缺失、拒绝访问、超时及旧版本 DLL，确认提示与实际原因对应，并执行提示的修复步骤。

性能报告不测真实候选窗绘制；TSF 日志中的 request-to-window 也仅截至窗口更新调用。评估用户可见首字延迟时，还需宿主程序回归或独立显示时序测量。
