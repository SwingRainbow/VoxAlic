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
| 遗物图标 | ✅ Pillow | ❌ 暂不做 |
| 打包大小 | ~30MB | 4-7MB 安装包 |
