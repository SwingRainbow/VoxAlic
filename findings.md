# 发现记录 — Tauri Warframe Monitor

## 环境问题

### Rust 编译链
- winget 安装 Rust 后默认 MSVC 工具链，缺少 `link.exe`
- 切到 GNU 工具链：`rustup default stable-x86_64-pc-windows-gnu`
- GNU 工具链缺 `dlltool.exe` → 安装 WinLibs MinGW (BrechtSanders.WinLibs.POSIX.UCRT, 261MB)
- 最终成功编译

### Tauri v2 API 差异
- `tray` 模块需要 Cargo feature: `tauri = { features = ["tray-icon"] }`
- `on_menu_event` 回调参数是 `&AppHandle`，不是 `&App`，不能调用 `.handle()`
- `withGlobalTauri: false` 时前端必须用 npm import: `import { listen } from '@tauri-apps/api/event'`

## 与原版的取舍

| 功能 | 原版 (Python) | Tauri 版 |
|------|-------------|----------|
| 世界时间 | ✅ | ✅ |
| 虚空裂缝 | ✅ | ✅ |
| 任务计时 (OCR) | ✅ | ❌ 暂不做 |
| 遗物图标 | ✅ Pillow | ✅ 静态 PNG |

## Bug 修复记录 (2026-05-28)

### 世界时间卡片不流动
- **原因**: `build_payload()` 中只重新计算了裂缝的 `remain_ms`，世界时间卡片直接 clone 自 state，`remain_ms` 只在 30 分钟 API 刷新时更新
- **修复**: `CycleInfo` 新增 `expiry_ms` 字段（阶段结束时间戳），所有周期解析函数（夜灵平野、魔胎之境、奥布山谷、双衍王境、扎里曼）填入 `expiry_ms = now + remain`，`build_payload()` 每次 tick 重新计算 `remain_ms = expiry_ms - now`
- **文件**: `models.rs:27`, `api.rs` (5个函数), `lib.rs:29-32`

### 筛选后 Tab 计数不更新
- **原因**: `renderFissures()` 中 tab 计数用原始列表过滤 `remain_ms > 0`，未应用层/任务类型筛选
- **修复**: 计数逻辑改为使用与表格相同的筛选条件（tier + mission_type + remain_ms）
- **文件**: `main.ts:101-112`

### 之前修复 (2026-05-27)
- 钢铁之路 0 条: `m["isHard"]` → `m["Hard"]`
- 任务类型英文: `mission_type()` 映射重新对齐 Python `MISSION_TYPE`
- 节点名不匹配: 完整 500+ 条目 `node_lookup()` 来自 Python `data/nodes.py`

## 设置功能 (2026-05-28)

### 配置文件持久化
- 使用 `app.path().app_data_dir()` 获取 AppData 目录
- JSON 格式: `{"close_to_tray": true}`
- 首次运行自动创建默认配置
- 配置用 `Arc<RwLock<AppConfig>>` 在 Tauri 状态中管理
- 关闭行为: `CloseRequested` 事件中读取配置决定 prevent_close + hide 还是放行退出

## 任务计时功能 (2026-05-28)

### 屏幕捕获
- Win32 `PrintWindow` + `PW_RENDERFULLCONTENT`，Rust `windows` crate 调用
- Windows crate 不暴露 PrintWindow，需手动 FFI 声明 `extern "system" fn PrintWindow`
- GDI 资源用 RAII 管理：`SelectObject → DeleteObject → DeleteDC → ReleaseDC`
- 通过窗口标题 "Warframe" 查找 HWND，`DwmGetWindowAttribute` 获取 DPI 感知边界
- 代码审查后修复：DWM 不可用回退 `GetWindowRect`、ROI 越界校验、unsafe 块最小化

### OCR 模板匹配
- 手写归一化互相关（NCC），不引入 opencv crate，约 180 行
- 10 张数字模板 PNG 通过 `include_bytes!` 编译时嵌入
- BGR→灰度→二值化（阈值 160）预处理
- NMS 合并重叠检测（IoU > 0.3），按 x 坐标排序拼接数字串
- 返回格式 "M:SS" 或 "MM:SS"，秒数 < 60 校验
- 已知校验：非对称 -10/+30 秒跳变窗口

### 计时器状态机
- 4 状态：Idle → Running → Paused → Checkpoint
- 同步内计时器：Instant + paused_elapsed 累加，Pause/Resume 正确保持累计时间
- OCR 3 次连续拒绝过滤，5 分钟截点检测（OCR 值 30s 不变）
- 维生系统：HSV 红色像素检测，高红% → danger

### 架构
- OCR 轮询跑在独立 std::thread（2s 间隔），不阻塞 tokio
- 前端命令通过 mpsc channel 发送 TimerCommand
- 现成 1s tokio tick 每 tick 调用 update_elapsed 并推送 event

## Batch A 改进 — 任务计时完善 (2026-05-28)

### 窗口管理
- 新增 `window.rs` — Win32 `EnumWindows` 枚举桌面窗口，按标题 + PID 过滤
- 前端下拉框选择目标窗口，调用 `list_windows` → `select_window(hwnd)` 命令
- `capture_roi()` 接受 HWND 参数，所有截图命令使用选定窗口句柄
- 窗口状态检查：无效 HWND / 最小化 / 不可见 时返回错误状态，前端灰度显示
- `strip_frame()` 去掉窗口标题栏和边框，纯客户区截图用于 OCR

### 日志面板
- 新增 `log.rs` — 前端日志通道，支持 `info` / `warn` / `error` / `success` 四级标签
- 后端通过 mpsc channel 推送日志条目到 Tauri event，前端滚动日志面板
- 记录：OCR 识别结果、截图状态、窗口变更、维生检测、截点触发
- 日志最多保留 500 条，自动滚动至底部

### 双 ROI 双时间
- AppConfig 扩展 `hp_roi` 和 `teammate_roi` 两组 HP 区域
- 配置增加 `ocr_interval_ms`（截图间隔）、`recognition_rate_samples`（识别率采样数）
- 前端显示双时间：自己 HP 时间 + 队友 HP 时间

### 截点倒计时
- 检测到 OCR 值接近 5:00 / 10:00 / 15:00 / 20:00 整分钟截点时，30s 锁定期
- 锁定期间显示倒计时文案 "[截点名称] 倒计时 XXs"
- 5 分钟截点到期后自动恢复为 Running 状态

### 识别率
- 连续 N 次 OCR 检测中，成功解析的比率计算为 `recognition_rate`
- payload 中推送识别率百分比，前端进度条/文本显示
- 连续失败时前端状态变灰提示

### 弹窗开关
- AppConfig 新增 `checkpoint_popup` 布尔开关
- 前端设置面板支持开关控制截点到期时是否弹窗提醒
- 弹窗使用 Tauri dialog API (info/confirm)

