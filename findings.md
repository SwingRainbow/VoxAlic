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
