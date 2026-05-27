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
