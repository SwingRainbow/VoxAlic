# 进度日志

## 2026-05-27 会话 — 从零构建 Tauri 版

### 设计阶段
- [x] 确定 Tauri v2 + Rust + TS 技术栈
- [x] 编写设计文档 → `docs/superpowers/specs/2026-05-27-tauri-warframe-monitor-design.md`
- [x] 编写实施计划 → `docs/superpowers/plans/2026-05-27-tauri-warframe-monitor.md`

### 实施阶段
- [x] 安装 Rust 1.95.0 + MinGW 编译链
- [x] npm create tauri-app 脚手架
- [x] Task 0: 环境就绪，cargo check 通过
- [x] Task 1: models.rs (Fissure, CycleInfo, AppStatePayload)
- [x] Task 2: api.rs (839行, HTTP + 解析 + 200+ 节点)
- [x] Task 3: state.rs (Arc<RwLock<AppState>>)
- [x] Task 4: lib.rs 重写 (后台循环 + 托盘 + 刷新命令 + tick)
- [x] Task 5: 前端 HTML/CSS (暗色主题)
- [x] Task 6: 前端 main.ts (事件监听 + DOM 渲染 + 筛选)
- [x] Task 7: tauri.conf.json + capabilities
- [x] Task 8: npm run tauri build → 构建成功

### 产物
- `tauri-warframe-monitor.exe` (23MB)
- `Warframe Monitor_0.1.0_x64-setup.exe` (NSIS, ~4MB)
- `Warframe Monitor_0.1.0_x64_en-US.msi` (~7MB)

### Git 提交历史
```
7779b33 feat: frontend UI — tabs, cycle cards, fissure table with filtering
6851b85 feat: add state management, background loops, system tray
e2e0c05 feat: add API layer — fetch, parse fissures and cycles
751cd88 feat: add data models (Fissure, CycleInfo, AppStatePayload)
151657f Initial scaffold — Tauri v2 + Vanilla TS + Rust deps
```

### 错误记录

| 错误 | 尝试 | 解决 |
|------|------|------|
| cargo check: `link.exe` not found | 1 | rustup default stable-x86_64-pc-windows-gnu |
| cargo check: `dlltool.exe` not found | 2 | winget install WinLibs MinGW (261MB) |
| cargo check: `tray` module not found | 3 | Cargo.toml: tauri features 加 `tray-icon` |
| cargo check: `.handle()` on AppHandle | 4 | lib.rs: `app.handle().clone()` → `app.clone()` |

## 2026-05-28 会话 — Bug 修复

- [x] 修复世界时间卡片不流动 → `CycleInfo` 加 `expiry_ms`，`build_payload` 每 tick 重算
- [x] 修复筛选后 Tab 计数不更新 → 计数逻辑对齐表格筛选条件
- [x] 构建验证通过
- [x] 打 tag v0.1.1 正式版

### 产物
- `tauri-warframe-monitor.exe` (23MB)
- `Warframe Monitor_0.1.0_x64-setup.exe` (NSIS, ~4MB)
- `Warframe Monitor_0.1.0_x64_en-US.msi` (~7MB)

### Git 提交历史
```
<latest>  feat: add settings tab with configurable close behavior
657cbd7   fix: cycle time not ticking + filter count not updating (v0.1.1)
b20dc2f   fix: mission type mappings, steel path detection, relic icons
4642e81   fix: complete 500+ node lookup table from Python original
feb1f80   Docs: add planning files — task plan, findings, progress log
7779b33   feat: frontend UI — tabs, cycle cards, fissure table with filtering
6851b85   feat: add state management, background loops, system tray
e2e0c05   feat: add API layer — fetch, parse fissures and cycles
751cd88   feat: add data models (Fissure, CycleInfo, AppStatePayload)
151657f   Initial scaffold — Tauri v2 + Vanilla TS + Rust deps
```

## 2026-05-28 会话 — 任务计时功能

- [x] 新增 `capture.rs` — Win32 PrintWindow 屏幕捕获 + ROI 裁剪
- [x] 新增 `ocr.rs` — 手写归一化互相关模板匹配 + NMS
- [x] 新增 `mission_timer.rs` — 计时器状态机 + OCR 轮询线程 + 维生 HSV 检测
- [x] 扩展 `models.rs` — MissionTimerPayload
- [x] 扩展 `config.rs` — ROISettings, MissionTimerConfig
- [x] 扩展 `lib.rs` — 全部整合，timer_command，tick 合并
- [x] 前端：计时 tab + CSS 样式 + TS 渲染
- [x] 构建验证通过，tag v0.3.0

### 产物
- `tauri-warframe-monitor.exe` (28MB)
- `Warframe Monitor_0.3.0_x64-setup.exe` (NSIS)
- `Warframe Monitor_0.3.0_x64_en-US.msi`

### Git 提交历史
```
<latest>  feat: mission timer with OCR screen capture — v0.3.0
d10420c   feat: integrate mission timer into lib.rs — thread, tick, commands
c6c01d0   feat: add mission timer tab and CSS styles
feat: add mission timer frontend rendering and controls
b81f140   fix: preserve paused_elapsed when resuming timer
abf85c4   fix: use asymmetric OCR validation bound (-10/+30s)
ada91f9   feat: add mission timer state machine and OCR polling thread
81ae428   feat: add MissionTimerConfig with ROI settings to AppConfig
a25544d   feat: add MissionTimerPayload to data models
9c25821   feat: add template matching OCR module
c028bad   fix: address code review findings in capture.rs
b0ed7c0   feat: add Win32 PrintWindow screen capture module
9110ebd   feat: add windows/image deps and digit templates for OCR
4a08bdc   feat: add settings tab with configurable close behavior
657cbd7   fix: cycle time not ticking and filter count not updating
b20dc2f   fix: mission type mappings, steel path detection, relic icons
4642e81   fix: complete 500+ node lookup table from Python original
feb1f80   Docs: add planning files — task plan, findings, progress log
7779b33   feat: frontend UI — tabs, cycle cards, fissure table with filtering
6851b85   feat: add state management, background loops, system tray
e2e0c05   feat: add API layer — fetch, parse fissures and cycles
751cd88   feat: add data models (Fissure, CycleInfo, AppStatePayload)
151657f   Initial scaffold — Tauri v2 + Vanilla TS + Rust deps
```
