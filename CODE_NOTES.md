# CODE_NOTES — 代码标注

> 本文件逐块标注 VoxAlic 全部代码的**功能**与**联动关系**，作为 `CLAUDE.md`（架构总览）之外的细粒度索引。
> 约定：「→」表示调用/触发方向，「⇄」表示双向共享，`channel` 指 `std::sync::mpsc`，`event` 指 Tauri `emit`/`listen`。
> 维护提示：改动模块职责或新增通道/事件/命令时，同步更新本文件与 `CLAUDE.md`/`AGENTS.md`。

---

## 0. 全局联动总览

### 0.1 执行上下文（线程）
| 上下文 | 位置 | 职责 |
|--------|------|------|
| 主线程 | Tauri 事件循环 | webview、托盘、窗口事件、`run_on_main_thread` 派发 |
| Tokio 任务 · 抓取循环 | `lib.rs` setup | 每 1800s 拉 worldstate + 启动即跑一次；跑订阅检查 |
| Tokio 任务 · 秒级 tick | `lib.rs` setup | 每 1s 递减倒计时、推进计时器 elapsed、emit `tick-update`；调用 `check_cycle_advance_alerts` 检查周期提前预告 |
| std 线程 · OCR 计时 | `mission_timer::start_timer_thread` | 截屏+OCR 状态机，`mpsc` 命令驱动 |
| std 线程 · 日志转发 | `lib.rs` setup | `log_rx` → emit `timer-log` |
| std 线程 · toast 转发 | `lib.rs` setup | `alert_rx` → `show_toast`（仅任务计时） |
| std 线程 · 订阅转发 | `lib.rs` setup | `notify_rx` → 累积列表 + emit `sub-notify` + 置闪烁标志 |
| std 线程 · 托盘闪烁 | `lib.rs` setup | 每 500ms 在「正常/内芯红」帧间切托盘图标 |

跨边界协调只能经**共享状态**或**通道**——OCR std 线程与 Tokio 任务不能直接互调。

### 0.2 通道（mpsc）
| 通道 | 生产者 | 消费者 | 载荷 |
|------|--------|--------|------|
| `log_tx/rx` | OCR 线程 | 日志转发线程 | `String` 日志 |
| `alert_tx/rx` | OCR 线程 | toast 转发线程 | `AlertMsg{title,body}`（计时提醒） |
| `notify_tx/rx` | 抓取循环的订阅检查 | 订阅转发线程 | `SubNotify`（订阅命中） |
| `cmd_tx`（Sender） | 前端命令 `timer_command`/`single_capture` | OCR 线程 | `TimerCommand` |

### 0.3 事件（emit → listen）
| event | 发射 | 监听 | 用途 |
|-------|------|------|------|
| `worldstate-update` | `fetch_store_emit` | main.ts | 整包刷新 UI |
| `tick-update` | 秒级 tick | main.ts | 每秒倒计时/计时器刷新 |
| `timer-log` | 日志转发线程 / `timer_command` | main.ts | 计时日志栏 |
| `sub-notify` | 订阅转发线程 / 托盘 Enter | notify.ts（弹窗） | 推送订阅列表 |

### 0.4 命令（前端 `invoke` → 后端 `#[tauri::command]`）
worldstate：`refresh_now`。配置：`get_config`/`set_config`。计时：`timer_command`/`single_capture`/`capture_preview`/`test_recognize`/`test_alert`。窗口：`list_windows`/`select_window`。物品库：`update_item_names`/`item_names_count`。**订阅提醒**：`get_notifications`/`clear_notifications`。自启动：`get_autostart`/`set_autostart`。卸载：`uninstall_clean`。更新：`check_for_update`/`install_update`。

### 0.5 共享托管状态（`app.manage`，命令经 `tauri::State` 取用）
`SharedState`(worldstate) · `SharedConfig`(配置) · `MissionTimerShared`(计时状态) · `Sender<TimerCommand>`(cmd_tx) · `Arc<DigitTemplates>`(OCR 模板) · `NotifyList`(订阅列表) · `FlashFlag`(闪烁开关) · `HideGen`(弹窗生命周期 gen)。

⚠ **Tauri v2 陷阱：`app.manage()` 必须在 webview 加载之前调用。** `setup` 闭包与前端加载是并发的——如果 `app.manage()` 放在 `setup` 末尾（托盘、线程、异步任务创建之后），前端 `invoke` 可能在 state 就绪之前到达，返回 `"state not managed"` 错误。所有 `app.manage()` 调用必须紧跟在各自 state 创建之后，在窗口/webview 创建之前完成。

### 0.6 数据流（简图）
```
api.fetch_worldstate ─► parse_* ─► AppState(state.rs) ─┐
                                                       ├─ build_payload ─► AppStatePayload ─ emit worldstate/tick ─► main.ts handleUpdate ─► render*
秒级 tick ─ 递减/推进 ───────────────────────────────────┘
抓取循环 ─► check_fissure/cycle/arbitration_alerts ─ notify_tx ─► 订阅转发线程 ─► NotifyList + emit sub-notify + FlashFlag=true
                                                                                    │
托盘闪烁线程 ◄─ 读 FlashFlag ── 切图标(正常/内芯红)                                      ▼
托盘 Enter ─► emit sub-notify(当前快照) + show 弹窗  ;  Leave ─► hide  ;  左键 ─► 唤主窗+FlashFlag=false
OCR 线程(截屏→ocr→状态机) ⇄ MissionTimerShared ; checkpoint/HP ─ alert_tx/dispatch_alert ─► toast（focus 无效时降级）
```

---

## 1. 后端模块（`src-tauri/src/`）

### 1.1 `main.rs`
- `main()` → 调 `voxalic_lib::run()`。桌面入口；产物 exe 的薄壳。

### 1.2 `lib.rs` — 应用编排核心
**(a) 订阅通知数据 + 检查器**
- `pub struct SubNotify{kind,icon,title,detail,ts}` — 一条订阅命中，序列化给弹窗。
- `check_fissure_alerts(fissures,&[FissureAlert],&mut notified,&Sender<SubNotify>)` — 命中匹配裂缝（按 `node_key:expiry` 去重）→ 发 `SubNotify`；清理过期键。被抓取循环调用。前端 `fissureSubscribed()` 镜像其匹配逻辑做列表标注。
- `check_arbitration_alerts(...)` — 仲裁节点变更且匹配 → 发 `SubNotify`。
- `check_cycle_alerts(...)` — 周期**状态切入**订阅态时 → 发 `SubNotify`。
- `check_cycle_advance_alerts(...)` — 周期**提前预告**（per-second tick 中）：当前状态≠目标状态且 `remain_ms ≤ advance_minutes×60×1000` 时发 `SubNotify`。按 `地点|目标状态|提前量` 去重，状态切入目标后自动清除标记。当前仅处理夜灵平野。
- 三者（+ 提前预告）均**只发 `notify_tx`**（不再走 toast；toast 仅留给计时器）。

**(b) 托盘闪烁图标生成**
- `hole_mask(&Image) -> Vec<bool>` — 从图标四周 flood-fill 标外部透明区，剩下被 logo 包住的透明像素=镂空内芯。
- `make_center_pulse_frames(&Image) -> Vec<Image>` — 基于 `hole_mask` + BFS 把内芯向外晕 2px(权重1/0.65/0.35)，生成「亮/暗」两帧（内芯亮红 255,45,45）。无洞时兜底整 logo 泛红。被 setup 预生成；闪烁线程循环切帧。

**(c) 负载构建 + 抓取**
- `build_payload(&AppState,&MissionTimerState) -> AppStatePayload` — 每 tick 重算裂缝/周期/Baro/赏金/回廊的倒计时；过期周期本地自愈（`roll_forward_cycle`/`parse_vallis/duviri`）。仲裁受 `state.initialized` 控制（API 数据就绪后才计算）。被秒级 tick 与 `fetch_store_emit` 调用。
- `fetch_store_emit(&SharedState,&MissionTimerShared,&AppHandle)` — 拉 worldstate→`parse_*`→写 `AppState`→设 `initialized=true`→`build_payload`→emit `worldstate-update`。网络失败时仍设 `initialized=true` 放行本地数据。被 `refresh_now`、抓取循环、托盘「立即刷新」调用。

**(d) Tauri 命令**（均 `#[tauri::command]`）
- `refresh_now` → `fetch_store_emit`。
- `get_config`/`set_config` — 读写 `SharedConfig` 并持久化（`config::save_config`）。
- `timer_command`/`single_capture` — 经 `cmd_tx` 给 OCR 线程发 `TimerCommand`；`set_mode` 直接改计时状态并 emit `timer-log`。
- （窗口解析统一用 `window::resolve_hwnd(keyword)`；原 lib.rs/mission_timer.rs 各有一份重复定义，已合并到 window.rs。）
- `capture_preview`/`test_recognize` — 校准用：截当前帧 / 对指定 ROI 跑一次 OCR，不动计时状态。
- `get_autostart`/`set_autostart` — 读写注册表 `HKCU\...\Run` 的 `VoxAlic` 项。
- `uninstall_clean` — 删 app data + 自启动项 + 启动系统卸载程序后退出。
- `show_toast(&AppHandle,title,body)` — `tauri-plugin-notification` 弹 Windows toast。被 toast 转发线程、`test_alert` 用。
- `test_alert` — 按 `alert_method` 预览计时提醒（toast 或窗口前置）。
- `update_item_names`/`item_names_count` — 物品名中文表下载热替换 / 计数（→ `item_i18n`）。`update_item_names` 现仅作**发版前重新生成内置 `baro_zh.json` 的工具**保留（前端「检查更新」按钮已移除），不再有用户入口。
- `game_data_version() -> &'static str` — 返回内置物品库对应的游戏更新号常量 `GAME_DATA_VERSION`（发版时手填，如「更新 43《Jade 之影：众星》」）；前端 设置→物品库 只读显示。
- `get_notifications(NotifyList) -> Vec<SubNotify>` — 弹窗加载时取当前订阅列表。
- `clear_notifications(NotifyList,FlashFlag)` — 清空列表 + 停闪。
- `build_source_updater`/`check_for_update`/`install_update` — 双源（Gitee/GitHub）更新；endpoint 按 `source` 切。

**(e) `run()` → `.setup()` 接线（联动枢纽）**
顺序：加载 config（`load_config`）→ `item_i18n::init` → 加载 OCR 模板 → 建 `MissionTimerShared` + `cmd_tx`（`start_timer_thread`）→ 建 `notify_list`/`flashing`/`notify_tx` + 预生成 `base_icon`/`glow_frames` → 起【日志/ toast / 订阅 / 闪烁】四个 std 线程 → 起【抓取/秒级 tick】两个 Tokio 任务 → 建托盘(`with_id("main")`，菜单 显示/刷新/退出，`on_tray_icon_event`：Enter 显弹窗、Leave 隐、左键唤主窗+停闪) → 给 "notify" 窗口挂失焦自动隐 → 主窗 `CloseRequested` 按 `close_to_tray` 隐藏到托盘 → `app.manage(...)` 全部托管状态 → `invoke_handler` 注册全部命令。
- ⚠️ 图标须 `Image::new_owned` 转 owned 才能进 `'static` 闪烁线程（借用 `default_window_icon` 会 E0521）。
- ⚠️ 托盘 `set_icon`/`set_tooltip` 必须 `run_on_main_thread` 派发。

### 1.3 `models.rs` — 序列化数据结构
`Fissure`/`CycleInfo`/`BaroItem`/`BaroInfo`/`MissionTimerPayload`/`RewardItem`/`RewardRotation`/`BountyJob`/`BountyInfo`/`CircuitInfo`/`ArbitrationSlot`/`ArbitrationInfo`/`AppStatePayload`。前端 `main.ts` 顶部 interface 与之一一对应；改字段须两侧同步。

### 1.4 `config.rs` — 配置持久化
- `ROISettings`/`MissionTimerConfig`/`AppConfig` + 各 `default_*` serde 默认。
- `FissureAlert`/`CycleAlert`/`ArbitrationAlert` — 订阅规则（空串=任意）。`check_*_alerts` 与前端 `fissureSubscribed` 消费。
- `load_config`（+ `migrate_old_default_rois` 静默升级旧 ROI）/`save_config`/`config_path`。存 `{app_data_dir}/config.json`。

### 1.5 `state.rs` — 运行时世界状态
- `AppState{normal/hard/storm_fissures,cycles,baro,bounties,circuit,last_update,countdown_secs,initialized}`，`SharedState=Arc<tokio::RwLock<AppState>>`。被抓取写、`build_payload` 读。`initialized` 控制仲裁等本地计算数据在首次 API 拉取完成前不渲染。

### 1.6 `api.rs` — worldstate 抓取与解析
- 时间/格式：`now_ms`/`to_ms`/`get_ms`/`is_active`/`fmt_remain`/`fmt_remain_baro`/`fmt_remain_days`/`fmt_dhms`。
- 翻译/查表：`tier_label`/`tier_order`/`mission_type`/`node_lookup`(NodeInfo,~160 节点)/`name_from_path`。
- 裂缝：`parse_fissure`/`parse_fissures`（普通/钢铁/风暴三组）。
- 周期：`find_active_syndicate`/`build_hex_cycle`/`roll_forward_cycle`(自愈)/`unknown_cycle`/`parse_vallis_cycle`(本地纪元)/`parse_duviri_cycle`(本地纪元)/`parse_zariman_cycle`/`parse_cycles`/`parse_hex_cycle`/`zariman_is_corpus`。
- 赏金：`bounty_card`/`bounty_type_zh`/`active_rotation_of`/`reward_tier`/`bounty_title`/各 `*_rewards()`/`rewards_for`/`rarity_rank`/`reward_rotations`/`sort_pool`/`deimos_rotations`/`parse_bounty_job`/`static_bounty_job`/`synthesize_{zariman,hex,entrati_lab}_jobs`/`parse_bounties`。
- Baro：`parse_void_trader`（物品名经 `item_i18n::translate` 兜底 `name_from_path`）。
- 回廊：`circuit_names`/`circuit_zh`/`parse_circuit`（读 `EndlessXpSchedule[0].CategoryChoices`，回退 `EndlessXpChoices`；DE 在 Jade Shadows 更新中将 Choices 合并进 Schedule）。
- 仲裁：`ArbData`/`ArbNodeInfo`/`arb_data`(内嵌时刻表)/`arb_slot_at`/`parse_arbitration`（纪元索引）。
- 抓取：`fetch_worldstate()` → `api.warframe.com/.../worldState.php` JSON。被 `fetch_store_emit` 调。

### 1.7 `capture.rs` — Windows GDI 截屏
`ROIConfig`/`is_black_frame`/`capture_full`(PrintWindow+GetDIBits,BGR,DWM 校正)/`crop_roi`/`capture_roi`/`capture_roi_stripped`(去 16:9 黑边)/`capture_preview_data_url`(校准用 PNG dataURL)/`base64_encode`。被 OCR 线程与 `capture_preview`/`test_recognize` 用。

### 1.8 `ocr.rs` — 数字模板匹配
`DigitTemplate`/`DigitTemplates`(0–9 内嵌 PNG，1.0x/0.85x 双尺度)/`push_template`/`recognize_digits`(二值化→NCC→NMS→"M:SS")/`match_template`(NCC)/`nms`(IoU=inter/min)。被 OCR 线程与 `test_recognize` 用。

### 1.9 `mission_timer.rs` — 计时状态机 + OCR 线程
- `TimerCommand`(枚举)/`AlertMsg{title,body}`/`AlertParams`/`render_alert_text`(`{min}` 替换)/`dispatch_alert`(focus 优先，窗口无效时降级 toast)。
- `TimerState`(Idle/Running/Paused/Checkpoint)/`MissionTimerState`(+payload)/`impl`(状态推进、checkpoint 5min 桶、HP 检测、OCR 接受规则、重置基准)。
- `state_str`/`parse_time_to_secs`/`log`/`apply_timer_command`/`detect_life_support`(HSV 红像素密度)/`resolve_hwnd`/`start_timer_thread`(起线程，返回 `cmd_tx`)。
- 联动：经 `MissionTimerShared` ⇄ tick/`build_payload`；`log_tx`→`timer-log`；`alert_tx`→toast 转发线程。

### 1.10 `window.rs` — Win32 窗口
`WindowInfo`/`list_windows`(EnumWindows 按关键字)/`resolve_hwnd`(两层检测：①标题+进程名双重匹配防浏览器误判 → ②纯进程名兜底)/`exe_name_for_pid`(CreateToolhelp32Snapshot 查进程名)/`find_window_by_process`(按进程名枚举窗口)/`is_minimized`/`is_valid`/`bring_to_front`(前置，HP/checkpoint focus 提醒用，无效 HWND 降级 toast)/`strip_frame`(16:9 去黑边)。

### 1.11 `item_i18n.rs` — 物品名中文化
`cell`(OnceLock<RwLock<HashMap>>)/`parse_compact`/`init`(用户覆盖文件→内嵌 `baro_zh.json`)/`translate`(verbatim→去 `/StoreItems/`→失败返 None)/`count`/`RawEntry`/`LangName`/`update_from_remote`(下 WFCD i18n.json 抽 zh.name 热替换)。被 `parse_void_trader`/`parse_circuit`/`update_item_names` 用。

---

## 2. 前端（`src/`、根 HTML）

### 2.1 页面结构
- `index.html` — 主窗：4 个 tab（世界时间/虚空裂缝/任务计时/设置）+ 统一订阅面板（裂缝/周期/仲裁规则）。脚本 `/src/main.ts`。
- `notify.html` — 托盘悬停弹窗（独立 vite 入口）：暗色卡片列表 `#np-list` + 头部「清空」`#np-clear`。脚本 `/src/notify.ts`。样式内联。
- `vite.config.ts` — `build.rollupOptions.input = {main, notify}` 双入口。

### 2.2 `src/main.ts`（主窗逻辑）
- 顶部 interface 镜像 `models.rs`；`AppConfig` 镜像 `config.rs`。
- 全局态：`currentData`/`currentSubTab`/`currentConfig` + 各面板展开标志。
- 渲染：`renderCycles`/`renderBountyPanel`/`renderCircuitPanel`/`renderBaro`/`renderArbitration`/`renderTimer`/`renderFissures`/`updateFilters`。
- 裂缝标注：`fissureSubscribed(f)` 镜像后端 `check_fissure_alerts` 匹配 → `renderFissures` 给命中行加 `.subscribed`（金边+🔔）。
- `filterFissures`/`getFilteredFissures` — 与标注互不冲突（筛选只看，标注只标）。
- `handleUpdate(payload)` — `worldstate-update`/`tick-update` 总入口，刷新所有面板；订阅规则 UI 仅在数据结构性变化时重渲（`_lastAlertSig`，避免每秒关掉下拉）。
- `DOMContentLoaded` — 接线全部 UI：tab/子 tab 切换、`btn-refresh`→`refresh_now`、筛选、设置项→`set_config`、计时按钮→`timer_command`、`btn-test-alert`→`test_alert`、更新/卸载/自启动等。生产版锁「任务计时」tab。
- 订阅 UI：`setupAlerts`/`renderFissureAlerts`/`renderCycleAlerts`/`renderArbitrationAlerts`/`selOpts`/`available*` + `saveAlerts`（存配置后重渲裂缝列表以同步标注）。
- 校准：`getTimerMode`/`setupCalibration`（截图→画布→拖框→`test_recognize`→存 ROI）。
- 监听：`timer-log`/`worldstate-update`/`tick-update`。

### 2.3 `src/notify.ts`（弹窗逻辑）
镜像 `SubNotify` interface。`init()`：`get_notifications` 初始填充 + `listen('sub-notify')` 实时刷新 + 「清空」→`clear_notifications`。`render`/`esc`/`relTime`。

### 2.4 `src/styles.css`
深色主题 + `:root` 变量。本次相关：`#fissure-table tr.subscribed`（金边/描边）、`.sub-bell`、`.sub-unified-foot`/`.sub-unified-hint`（订阅面板底部按钮行）。

---

## 3. 专题：订阅托盘提醒 端到端流程（本次新增，跨文件）

1. 用户在 设置→订阅 加规则 → `set_config` 存 `AppConfig.{fissure,cycle,arbitration}_alerts`。
2. 抓取循环（每 30min/启动）`fetch_store_emit` 后，读 config 调 `check_*_alerts`。
3. 命中 → 经 `notify_tx` 发 `SubNotify`。
4. 订阅转发线程：插入 `NotifyList`（上限 50）→ emit `sub-notify`（全量快照）→ `FlashFlag=true`。
5. 闪烁线程读 `FlashFlag`，每 500ms 在 正常/内芯红 帧间切托盘图标（亮/暗交替）。
6. 用户**悬停**托盘 → `Enter`：emit 当前快照给 "notify" 窗（防早期事件竞态）+ 定位光标上方 + `show`（不抢焦点）；**移开** `Leave`→`hide`；**左键**→唤主窗 + `FlashFlag=false`（停闪）。
7. 弹窗 `notify.ts` 渲染列表；「清空」→`clear_notifications`（清列表+停闪）。
8. 同一命中在 虚空裂缝 tab：`fissureSubscribed` → 命中行金边+🔔（仅裂缝；周期/仲裁不标注）。
- 计时器提醒**不走此链路**，仍 `alert_tx`→`show_toast`（右下角 Windows 通知）。

设计沿革与可调项见记忆 `project-subscription-tray-notify`（闪烁形态经多轮迭代定为「内芯红·亮暗交替·500ms」；备选 FlashWindowEx/自动弹窗/整 logo 变色未采纳）。

---

## 4. 资源 / 构建
- `src-tauri/resources/`：`digit_templates/`(OCR)、`baro_zh.json`(物品名)、`*_bounty_rewards.json`、`circuit_names.json`、仲裁时刻表等。`_gen_*.py` 生成脚本依赖 WFCD all.json（已删，需重下）。
- 构建：开发版 `npx tauri build --no-bundle`（出 `target/release/voxalic.exe`，带 `custom-protocol`）；发行版 `npx tauri build`（NSIS）。详见 `CLAUDE.md`「Build」段与记忆 `feedback-dev-vs-release-binaries`。
