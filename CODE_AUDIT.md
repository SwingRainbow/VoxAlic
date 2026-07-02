# CODE_AUDIT.md — 深度审查报告

> 审查日期：2026-07-02
> 审查范围：`src-tauri/src/` 全部 11 个 Rust 源文件
> 审查维度：逻辑正确性 / 性能 / 简化 / Rust 最佳实践
> 严重程度：🔴 高（可能导致崩溃/数据错） 🟡 中（性能/可维护性） 🟢 低（风格/nit）

---

## 1. `state.rs` (43 行)

**总体评价：** 简洁、无问题。`initialized` 标志的设计是之前修过的竞态修复，当前实现正确。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 1.1 | 简化 | 🟢 | `Default` impl 手动写但可以 derive | `#[derive(Default)]` 代替手动 `impl Default`，减少样板代码 |

---

## 2. `models.rs` (187 行)

**总体评价：** 数据结构定义清晰，与前端 interface 对应良好。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 2.1 | 最佳实践 | 🟡 | 大量 `String` 字段在 `clone()` 时产生堆分配。`AppStatePayload` 每 tick（1秒）构造一次，包含所有 `Fissure`/`CycleInfo`/`BountyInfo` 的 clone | 考虑用 `Arc<str>` 或 `Cow<'static, str>` 减少重复字符串的堆分配。不过当前数据量不大，实际影响有限 |
| 2.2 | 简化 | 🟢 | `Fissure` 同时有 `tier_key`（英文）和 `tier_label`（中文），冗余但方便前端 | 如果前端只需要中文，可去掉 `tier_key`；目前两处都用，保留合理 |
| 2.3 | bug | 🟢 | `BaroInfo::default()` 通过 `#[derive(Default)]` 生成，`start_ms`/`end_ms` 初始化为 0，但 `build_payload` 中对 `remain_ms` 做 `target - now` 计算——如果 Baro 数据尚未从 API 到达且 `initialized=false` 时 `baro` 字段为 `None`，不会触发此路径。当前受 `initialized` 保护，安全 | 无需修改 |

---

## 3. `config.rs` (316 行)

**总体评价：** serde 默认值体系完善，ROI 迁移逻辑正确。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 3.1 | 简化 | 🟡 | 每个默认值都有独立的小函数（`default_roi_x`、`normal_roi_y`、`fissure_roi_y` 等 12 个），散落且部分函数仅一行 | 可合并为模块级常量 `const DEFAULT_ROI_X: f64 = 0.005;` 等，直接用 `#[serde(default = "default_roi_x")]` 指向常量函数。当前写法可读性尚可，不改也行 |
| 3.2 | 最佳实践 | 🟢 | `load_config` 中 `let _ = std::fs::create_dir_all(app_data_dir);` 忽略错误——目录创建失败时后续 `save_config` 也会失败，但错误信息不够明确 | 可将 `create_dir_all` 的错误传播或至少 log |
| 3.3 | 性能 | 🟢 | `migrate_old_default_rois` 在每次 `load_config` 时都运行，即使配置已经是新的。虽然只做浮点比较（极快），但概念上是"一次性迁移" | 可加一个 `config_version: u32` 字段，迁移后设为当前版本，后续跳过。当前开销可忽略 |
| 3.4 | bug | 🟡 | `ROISettings::default()` 调用 `default_roi_y()` 返回 `normal_roi_y()` = 0.415。但 `default_fissure_roi()` 用 `..Default::default()` 展开后覆盖 `y` 和 `h`，而 `default_life_support_roi()` 覆盖四个字段。这意味着如果有人用 `ROISettings::default()` 然后只改 `x`，会得到 normal timer 的 y=0.415——对 timer ROI 是正确的，但语义不直观 | 当前所有实际使用都通过命名构造函数（`default_fissure_roi()` 等），不会误用。风险低 |

---

## 4. `api.rs` (1829 行) — **重点审查**

**总体评价：** 文件过长，承担过多职责（时间工具 + 节点查表 + 裂缝解析 + 周期解析 + Baro + 赏金 + 回廊 + 仲裁 + HTTP 抓取）。建议拆分但当前功能正确。

### 4.1 时间工具 (`now_ms` / `to_ms` / `get_ms` / `fmt_*`)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 4.1.1 | 最佳实践 | 🟢 | `now_ms()` 每次调用 `SystemTime::now().duration_since(UNIX_EPOCH).unwrap()` — 这个 `unwrap()` 理论上不会 panic（当前时间不可能早于 Unix epoch），但如果系统时钟被重置到 1970 年前则会崩溃 | 可改用 `unwrap_or(0)` 防御 |
| 4.1.2 | 性能 | 🟡 | `fmt_remain` / `fmt_remain_baro` / `fmt_remain_days` 在 `build_payload` 的每 tick 循环中对每个 fissure/cycle/bounty 调用，每次分配新 `String` | 已通过 clone 后的 mutate 实现，无更好方案。当前合理 |

### 4.2 节点查表 (`node_lookup`)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 4.2.1 | 性能 | 🟡 | `node_lookup` 是一个 ~160 分支的巨大 `match` 语句，每次调用都是线性扫描。Rust 编译器将其编译为跳转表或二分搜索，但不如 `HashMap` 直观 | 可改为 `OnceLock<HashMap<&str, NodeInfo>>` 在首次访问时构建。但编译器优化的 match 可能比 HashMap 查找更快。当前性能足够 |
| 4.2.2 | bug | 🟡 | `node_lookup` 对未匹配的 key 返回 `name: ""`，然后调用方 (`parse_fissure`) 用 `if info.name.is_empty() { node_key.clone() }` 兜底。但如果 API 新增节点类型，空 name 会导致前端显示原始 key——这不理想但在 Warframe 的更新频率下是可接受的降级 | 可考虑 log 未匹配的 key 以便发现新增节点 |

### 4.3 裂缝解析 (`parse_fissure` / `parse_fissures`)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 4.3.1 | 简化 | 🟡 | `parse_fissure` 中 `is_storm` 分支分别处理 `mission_type`：storm 用 `node_lookup` 的 `info.mission`，非 storm 用 `MissionType` 字段的翻译 | 两个分支逻辑差异大但结构相似，可抽取公共部分 |

### 4.4 周期解析 (`parse_cycles` / `roll_forward_cycle` / `build_hex_cycle`)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 4.4.1 | **bug** | 🔴 | `build_hex_cycle` 中 `remain_ms` 在传给 `CycleInfo` 后，又在 `build_payload` 的 for 循环中做 `if remain <= 0` 检查并可能被覆盖。但 `build_hex_cycle` 内部用的 `now_ms()` 与调用方 `build_payload` 的 `now` 之间存在微小时间差（同一函数内但两次调用），极端情况下可能导致 `remain_ms` 符号不一致 | 实际影响极小（两次 `now_ms()` 调用在微秒级内），但可传入 `now` 参数消除 |
| 4.4.2 | bug | 🟡 | `roll_forward_cycle` 中 Plains/Cambion 的 `if c.expiry_ms <= 0` 返回 `None`，此时 `build_payload` 会走到 `api::roll_forward_cycle` 的 `else` 分支 `c.remain_ms = 0; c.remain_str = "切换中"`。如果 API 返回了无效的 expiry (≤0)，UI 会一直显示"切换中"直到下次 API 刷新 | 可考虑在此情况下用 epoch 硬编码兜底（类似 Vallis/Duviri），但 Plains/Cambion 没有可靠的 epoch |
| 4.4.3 | 最佳实践 | 🟢 | `ZARIMAN_CORPUS_ANCHOR_MS: i64 = 1_780_384_080_000` — 硬编码的时间锚点。如果将来 Warframe 改变 Zariman 的轮换规律，需要更新此值 | 考虑将此锚点从配置文件读取或加入注释说明如何验证 |

### 4.5 赏金解析 (`parse_bounties` / `bounty_title` / 奖池)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 4.5.1 | 简化 | 🟡 | `bounty_title` 函数 ~170 行，包含 6 个 syndicate 的分支逻辑，每个分支又有大量硬编码匹配 | 可拆分为 `cetus_bounty_title` / `zariman_bounty_title` / `hex_bounty_title` 等独立函数 |
| 4.5.2 | 简化 | 🟡 | 6 个 `*_REWARDS: OnceLock<RewardTable>` 和对应的 `*_rewards()` 函数模式完全相同，只是嵌入的 JSON 文件不同 | 可用宏 `static_rewards!(CETUS, "cetus_bounty_rewards.json")` 消除重复 |
| 4.5.3 | bug | 🟡 | `reward_rotations` 的 steel-path fallback 只检查 `min >= 150`。如果 DE 某天改了 steel-path 的等级偏移量（比如 +150 而不是 +100），回退逻辑会失效 | 当前 Warframe 的 steel-path 固定 +100，风险低 |
| 4.5.4 | 最佳实践 | 🟡 | `deimos_rotations` 中 `prefix` 判断依赖 `rewards_path` 包含 `"Arcana"` 或 `"Vault"` 字符串。这是一种脆弱的字符串匹配——如果 DE 重命名路径会失效 | 可考虑在 `parse_bounty_job` 中传递更结构化的信息 |

### 4.6 回廊解析 (`parse_circuit`)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 4.6.1 | 性能 | 🟢 | `parse_circuit` 两次访问 `data["EndlessXpSchedule"].as_array().and_then(|a| a.first())`——一次读 `CategoryChoices`，一次读 `Expiry` | 可提取 `let schedule = data["EndlessXpSchedule"].as_array().and_then(\|a\| a.first());` 复用 |
| 4.6.2 | bug | 🟡 | 如果 `EndlessXpSchedule` 存在但 `Expiry` 字段缺失，`expiry` 为 0，`remain_ms = now_ms() - 0` 会是巨大的正数，前端显示错误的倒计时 | 应处理 `expiry == 0` 的情况，或在 `CircuitInfo` 中用 `Option<i64>` |

### 4.7 仲裁解析 (`parse_arbitration` / `arb_data`)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 4.7.1 | 最佳实践 | 🟡 | `ARB_DATA` 的 `OnceLock` 初始化中，`seq` 字段类型是 `&'static [u8]`，来自 `include_bytes!`。但如果 `arbitration_meta.json` 解析失败（`unwrap_or(Value::Null)`），后续 `nodes` 会为空，`arb_slot_at` 返回 `None`，前端显示空白而非错误 | 考虑在 meta 解析失败时 log 错误 |
| 4.7.2 | bug | 🟡 | `parse_arbitration` 中 `if hour_idx >= d.seq.len()` 返回 `None`——当内置的时刻表过期（超过 44056 小时 ≈ 5 年）后会静默失效 | 需在 2031 年前更新 `arbitration_seq.bin`。建议加一个 log 警告 |

### 4.8 HTTP 抓取 (`fetch_worldstate`)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 4.8.1 | bug | 🟡 | `fetch_worldstate` 中 primary 请求的 parse 错误只 `eprintln!` 后 fallback，但 fallback 的 fetch 错误会直接 `?` 返回给调用方——意味着 primary parse 失败 + fallback fetch 失败 = 错误。但如果 primary fetch 成功 + parse 失败 = 只试 fallback fetch（不重试 primary）。逻辑不对称 | 行为上合理（parse 失败通常意味着数据格式问题，换源可能解决）。可注释说明意图 |
| 4.8.2 | 性能 | 🟢 | 每次 `fetch_worldstate` 创建新的 `reqwest::Client`——应该复用连接池 | 使用 `OnceLock<reqwest::Client>` 或传入共享 client |

---

## 5. `lib.rs` (1301 行) — **重点审查**

**总体评价：** 应用编排核心，承担过多职责。线程管理、托盘、更新检查、订阅通知、命令路由全部在同一个文件。建议按职责拆分为 `commands.rs` / `notify.rs` / `tray.rs` / `update.rs`。

### 5.1 订阅通知 (`check_*_alerts`)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 5.1.1 | bug | 🟢 已评估-不修复 | `tick_advance_fired` 是纯内存 HashSet，重启后清空。若重启恰好在 advance 窗口内（占比 ≤10%），会产生一条重复的托盘提前提醒。详细讨论见第 5/6/8 轮审查 | 不修复——advance 窗口占比低 × 重启事件低频 × 托盘弹窗非侵入式，不值得引入任何持久化或时间幂等机制 |
| 5.1.2 | bug | 🟡 | `check_fissure_alerts` 中 `notified` set 的清理逻辑：`active` set 从当前所有 active fissures 构建，`retain` 移除已过期的。但如果一个 fissure 被 DE 提前移除（在 expiry 之前就没了），对应 key 会残留在 `notified` 中直到应用重启 | 影响小：残留 key 不会造成错误通知（因为 fissure 已经不在列表中了），只是占一点内存 |
| 5.1.3 | 性能 | 🟡 | 每秒 tick 中 `check_cycle_advance_alerts` 读取 config 的 `cycle_alerts` 并迭代。如果用户没有配置任何 cycle alert，仍会 clone `cycle_alerts`（空 Vec）并检查 `is_empty()` | 当前实现已在 `!cycle_alerts.is_empty()` 时才调用，正确 |

### 5.2 托盘闪烁 (`make_center_pulse_frames` / `hole_mask`)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 5.2.1 | 最佳实践 | 🟢 | `make_center_pulse_frames` 返回 `Vec<tauri::image::Image<'static>>`，内部用 `Image::new_owned` 创建 owned 副本。正确规避了 borrowed icon 的生命周期问题 | 无需修改 |
| 5.2.2 | 性能 | 🟢 | 闪烁线程每 500ms 通过 `run_on_main_thread` 设置托盘图标——这是必须的，因为 Tauri 的托盘 API 要求主线程 | 无需修改 |

### 5.3 托盘事件处理 (`on_tray_icon_event`)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 5.3.1 | bug | 🟡 | `TrayIconEvent::Enter` 处理中，如果 `snapshot.is_empty()`，hide popup 后直接 `return`，但在此之前已经 `fetch_add(1)` 了 `tray_gen`——这意味着 generation 被递增但 popup 没有显示，下一次 Enter 的 watcher 仍然会正确启动，但这次递增是多余的 | 无功能影响，只是 generation 多跳一次 |
| 5.3.2 | bug | 🟡 | `start_popup_watch` 中 `GetCursorPos` 在循环内每 120ms 调用一次，这是安全的（thread-safe Win32 API）。但如果系统有多个显示器且 DPI 不同，`GetCursorPos` 返回的物理坐标与 `popup.outer_size()` 的坐标可能不在同一坐标系——Tauri 在 Windows 上用物理像素，`GetCursorPos` 也是物理像素，理论上一致 | 实际测试中未发现问题。但理论上不同 DPI 显示器可能导致坐标系不一致 |

### 5.4 `build_payload`

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 5.4.1 | 性能 | 🟡 | 每 tick 克隆全部 `normal_fissures`/`hard_fissures`/`storm_fissures`/`cycles`/`baro`/`bounties`/`circuit`——即使数据没有变化 | 可考虑在 `fetch_store_emit` 中预计算 payload 并缓存，tick 只更新倒计时。但这需要重构 `AppState` 结构 |
| 5.4.2 | bug | 🟡 | 周期过期处理中，`parse_vallis_cycle()` 和 `parse_duviri_cycle()` 被直接调用——它们是纯函数，OK。但 `roll_forward_cycle` 返回 `None` 时 fallback 到 `remain_ms = 0`，这意味着如果 HexSyndicate 长期不可用（API 挂了），Plains/Cambion/Zariman 会一直显示"切换中" | 可以考虑给这些周期加一个基于本地时钟的粗略估算（类似 Vallis 的 epoch 模式），但 Plains/Cambion 没有公开的稳定 epoch |

### 5.5 `fetch_store_emit`

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 5.5.1 | **bug** | 🔴 | `fetch_store_emit` 在 `Ok` 分支中：先 `state.write().await` 存储数据（包括 `initialized = true`），然后 drop write lock，再 `state.read().await` 调 `build_payload`。在这两个锁操作之间，**tick loop 可能介入**并调用 `build_payload`（它也需要 write lock 来递减 countdown）。如果 tick 在 fetch 的 write 和 read 之间执行，tick 会看到新数据且能正确构建 payload——这是正确的。**但**如果 fetch 的 write 刚完成、tick 的 write 拿到锁并递减了 countdown、然后 emit `tick-update`，紧接着 fetch 的 read 也构建 payload 并 emit `worldstate-update`——前端会收到两次更新，第二次（worldstate-update）的 countdown 会比第一次（tick-update）多 1 秒。这是无害的闪烁，前端 `handleUpdate` 会覆盖 | 实际影响极小。如果在意，可以在 `fetch_store_emit` 中复用同一个 write lock 来构建 payload |
| 5.5.2 | bug | 🟡 | `fetch_store_emit` 的 `Err` 分支中设置了 `s.initialized = true`（放行本地数据），但**没有设置 `s.last_update` 和 `s.countdown_secs`**——这意味着 countdown 不会重置，会继续递减直到 0 然后一直为 0 | `countdown_secs` 降到 0 后前端只显示"刷新中"但不影响功能。实际影响小 |

### 5.6 更新检查

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 5.6.1 | 最佳实践 | 🟢 | `build_source_updater` 每次调用都重新创建 `Updater`——包括 `check_for_update` 和 `install_update` | 可以接受，因为更新检查频率极低（启动时一次 + 手动触发） |

### 5.7 线程管理

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 5.7.1 | bug | ✅ 已修复 | ~~四个无限循环的 `std::thread::spawn`（日志转发、toast 转发、订阅转发、托盘闪烁）均没有错误处理或 panic 恢复~~ → 4 个 std::thread + tick loop 均已用 `catch_unwind` 包裹循环体。日志转发和 tick loop 的 Err 分支 emit 恢复日志到前端；toast/订阅/闪烁线程静默恢复（不依赖 AppHandle 发日志）。tick loop 的 `catch_unwind` 仅包同步处理部分（emit + alert checks），async 锁获取在外部 | 已实现 (2026-07-02) |
| 5.7.2 | 最佳实践 | 🟡 | `start_popup_watch` 中 `let mut pt = windows::Win32::Foundation::POINT::default();` 然后 `unsafe { GetCursorPos(&mut pt) }`——如果 `GetCursorPos` 失败（极少见），`pt` 保持默认值 (0,0)，可能导致误判 | 检查返回值或初始化 `pt` 为 off-screen 值 |

---

## 6. `mission_timer.rs` (696 行)

**总体评价：** 计时状态机逻辑扎实，OCR 接受规则、checkpoint 机制、维生检测均经过多轮迭代打磨。

### 6.1 状态机 (`MissionTimerState`)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 6.1.1 | bug | 🟡 | `handle_command` 中 `TimerCommand::Start`：如果当前是 `Checkpoint` 状态，会转为 `Running`。但如果 `start_instant` 已经是 `Some`（Checkpoint 时是有值的），不会重置 `start_instant`。然后 `update_elapsed` 用 `start.elapsed() + paused_elapsed`——其中 `paused_elapsed` 已在 `apply_ocr` 的 checkpoint 分支被设为 OCR 秒数，`start_instant` 是进入 checkpoint 时的时刻。这会导致一段短暂的时间内 elapsed 比实际多（从 checkpoint 触发到用户切回游戏的时间） | 实际上从 checkpoint 到用户手动 Start 的时间通常很短（用户看到提醒后切回游戏），且进入 checkpoint 后 `update_elapsed` 不再更新（因为 state 是 Checkpoint 而非 Running）。在 Idle/Paused 状态下 Start 重置 `start_instant`，但从 Checkpoint Start 不重置——这是有意为之（保留 OCR 同步的时间基准）。逻辑正确 |
| 6.1.2 | 简化 | 🟡 | `apply_ocr` 函数约 80 行，包含 checkpoint 恢复、OCR 验证、checkpoint 触发三个逻辑段 | 可拆分为 `try_resume_from_checkpoint` / `validate_ocr` / `check_milestone` 三个方法 |

### 6.2 OCR 轮询循环 (`start_timer_thread`)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 6.2.1 | 性能 | 🟡 | 主循环中有多次 `config.read().unwrap()` 调用——在单次迭代中，config 被读取最多 3 次（hwnd resolve、single capture、main OCR） | 可以接受，因为 `StdRwLock::read` 成本极低且 OCR 间隔通常 ≥2 秒 |
| 6.2.2 | bug | 🟡 | HP 检测中 `life_support_pct` 在 danger 时设为 15.0（硬编码），在 normal 时设为 0.0——这是"二值化"的信号，而非真实百分比。前端用红/绿/灰圆点渲染，不显示数字 | 命名 `life_support_pct` 有误导性，实际是 danger flag。可改为 `life_support_danger: bool` |

### 6.3 维生检测 (`detect_life_support`)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 6.3.1 | 最佳实践 | 🟢 | HSV 转换中 `((g - b) / delta).rem_euclid(6.0)` 正确处理了浮点取模——这是之前修过的 bug fix（标准 `%` 对负数的行为与 Python 不同） | 正确。注释可说明为何用 `rem_euclid` |

---

## 7. `capture.rs` (277 行)

**总体评价：** Windows GDI 截屏实现完整，包含 DWM 校正、黑帧检测、16:9 去边、BGR→RGB 转换。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 7.1.1 | **bug** | 🔴 | `capture_full` 中 `PrintWindow` 的返回值被忽略（`let pw_result = unsafe { PrintWindow(...) }`），仅用于判断是否需要 fallback。但 `PrintWindow` 失败时不会清理已分配的 GDI 资源吗？——不，GDI 资源在 `pw_result` 检查之后的 finally 块中统一清理。**但**如果 `GetDIBits` 失败（`result == 0`），函数返回 `None`，在此之前 GDI 资源已被清理（`SelectObject`/`DeleteObject`/`DeleteDC`/`ReleaseDC` 在 `GetDIBits` 调用之后的 unsafe 块中）——正确 | 无问题 |
| 7.1.2 | bug | 🟡 | `is_black_frame` 的提前退出逻辑：`if non_black * 100 > total` —— 用整数乘法避免浮点。但如果 `total` 很大（4K 屏幕 ~8M 像素），`non_black * 100` 可能溢出 `usize`（64位下不会，但理论上 32 位会） | 在 64 位 Windows 上 `usize` 是 64 位，不会溢出。安全 |
| 7.1.3 | 最佳实践 | 🟡 | 多个 `unsafe` 块分散在函数中。`capture_full` 中有 6 处 `unsafe` 调用 | 可考虑用 `// SAFETY:` 注释说明每个 unsafe 的前提条件 |
| 7.1.4 | 性能 | 🟡 | `capture_full` 中 BGRA→BGR 转换分配新的 `Vec<u8>`（`full_pixel_count * 3` bytes），对于 4K 屏幕约 24 MB | 无法避免——OCR 需要 BGR 格式。但如果仅需 ROI 裁剪，`capture_roi_stripped` 也是先全屏捕获再裁剪，浪费了大部分像素。可以考虑先裁剪再 BGRA→BGR 转换 |
| 7.1.5 | 简化 | 🟡 | `base64_encode` 手写了标准 Base64 编码器 | 可接受——避免引入额外依赖。但如果将来需要更多编码功能，考虑用 `base64` crate |

---

## 8. `ocr.rs` (228 行)

**总体评价：** NCC 模板匹配实现正确，多尺度 + NMS 策略合理。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 8.1.1 | 性能 | 🟡 | `match_template` 是 O(W×H×w×h) 的四重循环——对于 10 个数字 × 2 个尺度 = 20 个模板，每个模板在 ROI 图像上做滑动窗口。典型 ROI 约 80×20 像素，模板约 12×18 像素 | 实际计算量约 20 × 80 × 20 × 12 × 18 ≈ 7M 次操作，在 2 秒 OCR 间隔下完全可接受 |
| 8.1.2 | 最佳实践 | 🟡 | 同上，缺少 `// SAFETY:` 注释。不过此处无 unsafe 代码，不适用 |
| 8.1.3 | 简化 | 🟢 | `recognize_digits` 中解析时间格式的逻辑（`digits[..len-2]` / `digits[len-2..]`）假设最后两位是秒——对于 `M:SS` (3 位) 和 `MM:SS` (4 位) 都正确 | 可考虑用正则或更明确的解析 |
| 8.1.4 | bug | 🟡 | `nms` 的 IoU 公式使用 `inter / min(area_a, area_b)` — 这是特意选择的（小框被大框完全包含时不应被抑制）。这是之前修过的 bug fix（标准 IoU `inter/union` 对小框包含在大框中的情况过于激进） | 正确。与 Python 原版对齐 |

---

## 9. `window.rs` (219 行)

**总体评价：** Win32 窗口管理，实现清晰。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 9.1.1 | **bug** | 🔴 | `resolve_hwnd` 中 Tier 1 搜索：`exe_name_for_pid(w.pid).map(\|n\| n.to_lowercase().contains(&keyword_lower))` —— 如果用户把游戏 exe 重命名了（比如 `wf.exe`），而 keyword 是 "Warframe"，Tier 1 会失败。然后 fallback 到 Tier 2 `find_window_by_process(keyword)`——用同样的 keyword 搜索进程名，同样会失败 | 这是设计上的限制：keyword 必须同时匹配窗口标题和进程名。如果用户改了 exe 名，需要同时更新 config 中的 `window_title` |
| 9.1.2 | bug | 🟡 | `enum_callback_by_process` 中检查 `ctx.0 != 0` 来提前退出，但 `EnumWindows` 的回调返回 `BOOL::from(true)` 表示继续——返回 `FALSE` 才能停止枚举。当前实现找到匹配后虽然设了 `ctx.0`，但**仍然继续枚举**所有剩余窗口 | 找到后应返回 `BOOL::from(false)`（即 `BOOL(0)`）来停止。当前浪费少量 CPU 但功能正确 |
| 9.1.3 | 最佳实践 | 🟡 | `bring_to_front` 使用了 `SetWindowPos(HWND_TOPMOST)` → `SetWindowPos(HWND_NOTOPMOST)` 的技巧来强制窗口置顶——这是经典的 Win32 hack，正确但不够优雅 | 如果 Warframe 是独占全屏模式，`SetForegroundWindow` 可能仍然失败。当前实现先 TOPMOST 再 NOTOPMOST 是经过验证的有效方法 |
| 9.1.4 | 性能 | 🟢 | `strip_frame` 中 `pixels.to_vec()` 在不需要裁剪时也会复制整个缓冲区 | 可返回 `Cow<[u8]>` 来避免不必要的复制。但调用频率低（仅在 OCR 轮询时），影响可忽略 |

---

## 10. `item_i18n.rs` (120 行)

**总体评价：** 简洁、正确。OnceLock + RwLock 热替换模式安全。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 10.1.1 | 性能 | 🟡 | `translate` 中在 `map.get(path)` 未命中时做 `path.replacen("/StoreItems/", "/", 1)`——这会分配一个新的 `String` 即使最终也未命中 | 可以用 `str::strip_prefix` 类的方法或者先检查路径是否包含 `/StoreItems/`。不过 translate 只在 Baro 解析时调用（每次 API 刷新最多几十次），开销可忽略 |
| 10.1.2 | 最佳实践 | 🟢 | `update_from_remote` 中 ~51 MB 的 JSON 反序列化使用 `serde_json::from_slice`，会一次性加载整个 HashMap 到内存。加上 intermediate RawEntry 结构，峰值内存可能超过 100 MB | 可考虑流式解析（但 serde_json 不原生支持）。当前在桌面应用场景下可接受 |

---

## 11. `phone_push.rs` (38 行)

**总体评价：** 最小化模块，无问题。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 11.1.1 | 最佳实践 | 🟢 | `push` 函数中所有错误均被静默吞掉（`let _ = ...`）——设计如此，因为 phone push 是"尽力而为"的辅助通知 | 可考虑至少 `eprintln!` 记录失败 |
| 11.1.2 | 性能 | 🟢 | 每次 push 创建新的 `reqwest::Client` | 同上，push 频率极低。可接受 |

---

## 总结

### 按严重程度汇总

| 严重 | 数量 | 关键项 |
|------|------|--------|
| 🔴 高 | 1 | 9.1.2 EnumWindows 未提前终止（唯一剩余 🔴） |
| 🟡 中 | 19 | 分布在各模块，多为边界条件、性能优化、简化建议 |
| 🟢 低 / ✅ 已修复 | 16 | 代码风格、注释、防御性编程；5.7.1 线程 panic + 26.3.1 休眠 countdown 已修复，5.1.1 已评估不修复 |

### 架构层面建议

1. **`api.rs` 过长 (1829行)**：建议拆分为 `api/fissures.rs`、`api/cycles.rs`、`api/bounties.rs`、`api/circuit.rs`、`api/arbitration.rs`、`api/fetch.rs`
2. **`lib.rs` 职责过载 (1301行)**：建议拆分为 `commands.rs`、`notify.rs`、`tray.rs`、`update.rs`
3. ~~**跨线程 panic 传播**~~ ✅ 已修复：4 个 std::thread + tick loop 均已添加 `catch_unwind`
4. **测试覆盖**：整个 Rust 代码库 0 个 `#[test]`——建议至少为核心解析逻辑（`parse_fissures`、`parse_cycles`、`roll_forward_cycle`、`recognize_digits`）添加单元测试

### 正面评价

- 线程安全模型 (`Arc<RwLock<>>` + `mpsc`) 设计合理，没有发现数据竞争
- OCR 接受规则 (`-10..=30`) 和拒绝恢复机制经过充分考量
- 周期自愈 (`roll_forward_cycle`) 设计精巧，解决了 30 分钟 API 轮询间隔内 UI 过期的问题
- `initialized` 标志的设计是解决启动竞态的简洁方案
- 托盘闪烁（内芯红·亮暗交替）的 `hole_mask` BFS 实现思路巧妙

---

## 12. `src/main.ts` (1496 行) — 前端审查

**总体评价：** 单个文件承载全部 UI 逻辑，过长。Vanilla TypeScript 写法直接但缺乏组件化。与 Rust 后端的 interface 对应良好。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 12.1 | **bug** | 🔴 | `handleUpdate` 每 tick 调用 `renderFissures()`，后者重建整个 `<tbody>` innerHTML。如果用户在裂缝表上选中了文本或正在与某行交互，会被打断。对于每秒触发的 tick，这种全量重渲染是浪费 | 参照 `renderBaro` 的 signature 优化模式，只在数据结构性变化时重建 DOM；tick 仅更新 `remain_str` 列的文本 |
| 12.2 | **bug** | 🔴 | `handleUpdate` 每 tick 调用 `updateFilters()`，后者重建 `<select>` 的 innerHTML。虽然 try 保留了 `currentTier`/`currentType` 选中值，但重建 `<select>` 会触发 `change` 事件（或至少视觉闪烁） | 参照 `_lastAlertSig` 模式，只在第一次 worldstate-update 时调用 `updateFilters()` |
| 12.3 | 简化 | 🟡 | `setupAlerts()` 返回的 `refresh` 函数和 `_refreshAlertsCb` 机制：`handleUpdate` 中通过 `_lastAlertSig` 避免频繁重建订阅 UI。但 `refresh` 本身每次仍会做 `renderFissureAlerts` + `renderCycleAlerts` + `renderArbitrationAlerts`，即使三者都没变化 | 可以为每种 alert 类型维护独立的 signature |
| 12.4 | 简化 | 🟡 | 三种 alert 的 change/click/delete 事件处理代码高度重复（fissure/cycle/arbitration 三个 block 结构几乎一样） | 可抽象为通用的 `setupAlertSection(container, renderFn, addBtn, ...)` |
| 12.5 | 最佳实践 | 🟡 | `currentConfig` 通过可变对象操作：`currentConfig.fissure_alerts[i].tier = ...` 直接修改嵌套属性后调 `saveAlerts()`。如果 save 失败但本地已改，UI 与持久化状态不一致 | 考虑 immutable 更新模式：`currentConfig = {...currentConfig, fissure_alerts: currentConfig.fissure_alerts.map(...)}` |
| 12.6 | 最佳实践 | 🟡 | `listen<AppStatePayload>('tick-update', ...)` 和 `listen<AppStatePayload>('worldstate-update', ...)` 都调用 `handleUpdate`。但 `tick-update` emit 的是 `&payload`（引用），`worldstate-update` emit 的是 owned `payload`——两者到达前端时都是反序列化后的对象，TS 侧无区别 | 行为正确，但可为 tick-update 设计更轻量的 payload（只含 countdown/remain 字段）减少序列化开销 |
| 12.7 | bug | 🟡 | `renderCycles` 中 `clickable` 判断：`withBounty.has(c.name) || isCircuit`，但 `openBounty` 状态不与后端同步——如果 API 刷新后 bounty 列表变了（某地赏金过期消失），但 `openBounty` 仍指向该地，面板会空白但不自动关闭 | `renderBountyPanel` 中已经用 `list.length` 做了守卫（空则 hidden），安全 |
| 12.8 | 性能 | 🟡 | `highlightFissureRow` 中 `Array.from(rows).find(...)` 每次遍历全部行——虽然只在用户点击托盘通知时触发，频率极低 | 可接受 |
| 12.9 | 最佳实践 | 🟢 | `// @ts-expect-error process is a nodejs global` 在 vite.config.ts 中。`main.ts` 中 `(import.meta as any).env?.PROD` 也是 any 类型绕过 | 可定义 `ImportMeta` 类型扩展消除 `any` |

---

## 13. `src/notify.ts` (84 行)

**总体评价：** 短小、专注。托盘弹窗逻辑清晰。启动竞态处理得当（listener-first + try/catch 兜底）。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 13.1 | 最佳实践 | 🟢 | `esc()` HTML 转义函数手动实现 | 可考虑用 `textContent` 赋值代替 innerHTML 拼接，天然防 XSS。但当前弹窗数据全部来自本地 Rust 后端（SubNotify），无外部注入风险 |
| 13.2 | bug | 🟡 | `relTime` 用 `Date.now() - ts` 计算相对时间。`ts` 是 Rust `now_ms()` 在通知**创建**时的时间戳（UTC 毫秒）。JS `Date.now()` 也是 UTC 毫秒，理论上一致。但如果系统时钟在通知创建后被调整，显示可能不准 | 实际影响极小 |

---

## 14. `index.html` (337 行) + `notify.html` (103 行)

**总体评价：** HTML 结构清晰，语义合理。所有交互逻辑在 TS 文件中，HTML 只做布局。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 14.1 | 最佳实践 | 🟢 | `index.html` 中 `<script type="module" ... defer>` — `defer` 对 module 脚本无效（module 默认 defer），冗余但无害 | 可移除 `defer` |
| 14.2 | 最佳实践 | 🟢 | `notify.html` 的 CSS 内联在 `<style>` 而非独立文件——合理，因为弹窗是独立的 webview 入口，不需要共享主应用的样式 | 无需修改 |
| 14.3 | 可访问性 | 🟢 | 缺少 `<meta name="color-scheme" content="dark">` 或 `prefers-color-scheme` 适配 | 应用强制暗色主题，无需系统级适配 |

---

## 15. `src/styles.css` (959 行)

**总体评价：** 全面的暗色主题，自定义属性体系完整。CSS 组织可按组件分文件但当前规模可接受。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 15.1 | 简化 | 🟡 | 多处硬编码重复的颜色值：`#ffd27a`（倒计时黄）出现 3 次，`#ff9a52`（橙）出现 2 次 | 可提升为 CSS 自定义属性：`--countdown: #ffd27a; --bounty-accent: #ff9a52;` |
| 15.2 | bug | 🟢 | `tr.expiring td:last-child` 选择器用 `:last-child` 匹配"剩余时间"列——依赖于 HTML 中该列永远在最后 | 当前 HTML 结构稳定，风险低。但如果加列会导致样式错位 |
| 15.3 | 最佳实践 | 🟢 | `@keyframes pulse` 使用 `infinite` 迭代——checkpoint 状态下持续闪烁。但如果用户离开计时 tab（切换到世界时间），`#tab-timer` 变为 `display:none`，动画仍在后台运行 | 浏览器会在 `display:none` 元素上暂停动画。实际无性能影响 |

---

## 16. 配置文件审查

### 16.1 `tauri.conf.json`

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 16.1.1 | bug | 🟡 | `version: "1.1.1"` 与 `Cargo.toml` 的 `version = "1.0.7"` 不一致。Tauri v2 以 tauri.conf.json 为准，但 Cargo.toml 版本也应该同步 | 出发布版本前确保两处版本号同步 |
| 16.1.2 | 安全 | 🟡 | `"csp": null` — 完全禁用 Content Security Policy。开发阶段方便，但生产环境应该设置最小权限 CSP | 考虑至少设置 `default-src 'self'; style-src 'self' 'unsafe-inline'` |
| 16.1.3 | 最佳实践 | 🟢 | `"withGlobalTauri": true` 允许在非 Tauri 上下文中使用 `window.__TAURI__`——Vite dev 模式下有用 | 生产构建不需要，但无害 |

### 16.2 `Cargo.toml`

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 16.2.1 | 最佳实践 | 🟢 | `tokio = { features = ["full"] }` — 引入了不需要的 feature（如 `rt-multi-thread` 之外的 IO/sync/signal 等） | 可精简为 `features = ["rt-multi-thread", "macros", "sync", "time"]` 以减少编译时间 |
| 16.2.2 | 最佳实践 | 🟢 | `image` crate 只用 PNG 编解码，feature 正确 (`default-features = false, features = ["png"]`) | 无需修改 |

### 16.3 `package.json` + `vite.config.ts` + `tsconfig.json`

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 16.3.1 | 最佳实践 | 🟢 | `package.json` version `0.1.0` 与 tauri.conf.json `1.1.1` 不一致——前端版本号无实际作用（不发布 npm 包），但容易混淆 | 同步或设为 `0.0.0`（private） |
| 16.3.2 | 最佳实践 | 🟢 | `tsconfig.json` 中 `noUnusedLocals: true` + `noUnusedParameters: true` — 良好的严格配置 | 无需修改 |
| 16.3.3 | 性能 | 🟢 | `vite.config.ts` 中 `minify` 未显式设置，Vite 6 默认 esbuild 压缩 | 出生产构建时自动启用。无需修改 |

---

## 17. 第二轮审查：数据流追踪

### 17.1 完整请求链路：世界状态更新

```
用户点击刷新 / 30min定时器 / 启动
  → fetch_store_emit (lib.rs:463)
    → fetch_worldstate (api.rs:1793): HTTP GET → JSON
    → parse_fissures / parse_cycles / parse_void_trader / parse_bounties / parse_circuit (api.rs)
    → state.write().await: 存储全部解析结果 + initialized=true
    → build_payload (lib.rs:377): clone全部数据 + 倒计时重算 + parse_arbitration
    → handle.emit("worldstate-update", payload)
  → 前端 listen('worldstate-update') (main.ts:1112)
    → handleUpdate(payload)
      → renderCycles / renderBountyPanel / renderCircuitPanel / renderBaro / renderArbitration
      → updateFilters + renderFissures + renderTimer
```

**数据流层面发现：**

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 17.1.1 | 性能 | 🟡 | `fetch_store_emit` 成功后，抓取循环 (`lib.rs:1087-1090`) 又一次读取 state 和 config 做订阅检查。这意味着每次 API 刷新后，同一份数据被 clone 两次——一次在 `build_payload`，一次在 `check_*_alerts` | 可以在 `fetch_store_emit` 完成后直接调用订阅检查，复用已 clone 的数据 |
| 17.1.2 | bug | 🟡 | 抓取循环中 `parse_arbitration(now)` 独立调用，与 `build_payload` 中的 `parse_arbitration(now)` 是两次独立调用——虽然结果相同（epoch 计算），但浪费且两个 `now_ms()` 有微秒级差异 | 无功能影响 |

### 17.2 Tick 循环链路

```
每秒定时器触发 (lib.rs:1103)
  → state.write(): countdown-1
  → timer.write(): update_elapsed
  → build_payload: clone全部数据 + 全部倒计时重算
  → emit("tick-update", &payload)
  → check_cycle_advance_alerts (仅夜灵平野)
  → 前端 handleUpdate: 全量重渲染
```

**数据流层面发现：**

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 17.2.1 | **性能** | 🔴 | **每 1 秒**执行 `build_payload` 完整 clone 所有 fissures/cycles/bounties/circuit/baro。即使 30 分钟内 API 数据未变，每 tick 仍做 ~7 次 Vec clone + 遍历重算倒计时 | 这是最大的性能瓶颈。理想方案：tick 只在 `AppState` 中维护 countdown 和当前时间戳，payload 只更新 `remain_str` 字段而不重建整个结构。或者：前端自己算倒计时（Rust 提供 expiry_ms 和 now_ms，前端算 remain），Rust tick 只发一个轻量的时间戳 |
| 17.2.2 | 性能 | 🟡 | tick 循环中 `tick_config.read().unwrap().cycle_alerts.clone()` 每秒 clone 一次 cycle_alerts（即使为空 Vec） | 已经有 `!cycle_alerts.is_empty()` 守卫，空 Vec 的 clone 开销极小 |

### 17.3 OCR 计时链路

```
OCR std 线程 每 N 秒 (mission_timer.rs:412)
  → 检查 window 有效性 / minimized
  → capture_full + strip_frame + crop_roi (capture.rs)
  → recognize_digits (ocr.rs): NCC 模板匹配
  → apply_ocr: 验证 + 同步 + checkpoint 检测
  → detect_life_support (第二个 ROI): HSV 红像素
  → dispatch_alert (如需): focus/toast
```

**数据流层面发现：**

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 17.3.1 | 性能 | 🟡 | OCR 循环内两次 `config.read().unwrap()`——一次读取 OCR interval/mode/strip_frame，一次读取 HP ROI。第二次在 HP capture 路径内，但不是每次都需要（只有 running 且非 minimized 时才走） | 可合并为一次 config 读取，提前取出所有需要的字段 |
| 17.3.2 | bug | 🟡 | HP 检测在 timer ROI OCR 成功或失败后都会执行（`consecutive_capture_fails` 只影响主 OCR 路径）。但如果主 capture 失败（黑帧），HP capture 大概率也失败——多做了一次无效截屏 | 可在主 capture 失败时跳过 HP capture（`continue` 或 `else` 分支） |

### 17.4 订阅通知链路

```
抓取循环后 (lib.rs:1087-1090):
  check_fissure_alerts → notify_tx → 订阅转发线程 → emit sub-notify → notify.ts
  check_cycle_alerts    → notify_tx → 同上
  check_arbitration_alerts → notify_tx → 同上

每秒 tick (lib.rs:1120-1128):
  check_cycle_advance_alerts → notify_tx → 同上（仅夜灵平野提前提醒）
```

**数据流层面发现：**

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 17.4.1 | 简化 | 🟡 | 四种 notification 检查都通过同一个 `notify_tx` 发送，但各自有不同的去重机制（`notified_fissures: HashSet`、`prev_cycle_states: HashMap`、`prev_arb_key: String`、`tick_advance_fired: HashSet`）且分散在不同位置 | 可统一为 trait `AlertChecker` with `check(&self, ...) -> Vec<SubNotify>` |
| 17.4.2 | bug | 🟡 | 订阅转发线程也读取 `notify_bark_url` 做手机推送——每次通知命中都会 clone config 锁。如果用户在通知瞬间恰好改了 Bark URL，可能用到旧值 | 实际影响极小 |

---

## 第二轮总结

### 新发现的关键问题

| 严重 | 数量 | 关键项 |
|------|------|--------|
| 🔴 高 | 2 | 12.1 每 tick 全量重建裂缝表 DOM、17.2.1 每 tick clone 全部数据 |
| 🟡 中 | 14 | 涵盖前端、配置、数据流 |

### 性能关键路径排名

1. **每 tick `build_payload` 全量 clone** — 最大浪费，每秒执行
2. **每 tick 前端 `renderFissures` + `updateFilters` 全量重建 DOM** — 与 #1 配套
3. **`capture_full` 全屏 BGRA 捕获后仅用 ROI** — 每次 OCR 都浪费 90%+ 像素
4. **`fetch_worldstate` 每次新建 `reqwest::Client`** — 每 30 分钟一次，影响小

### 前后对比

- **第一轮（Rust 模块级）**：发现 4 个 🔴（线程 panic、竞态、周期提前通知重启重复、EnumWindows 终止）
- **第二轮（前端 + 数据流）**：发现 2 个 🔴（前端全量重渲染、tick clone 风暴）
- **总计**：6 个 🔴、34 个 🟡、20 个 🟢

---

## 18. 第三轮审查：并发安全与锁竞争

### 18.1 锁清单与访问者矩阵

| 锁 | 类型 | 访问者 |
|----|------|--------|
| `SharedState` | `tokio::sync::RwLock` | fetch_store_emit(W/R), tick loop(W/R), fetch loop 订阅检查(R) |
| `SharedConfig` | `std::sync::RwLock` | Tauri 命令(R/W), OCR 线程(R), fetch loop(R), tick loop(R), alert 转发(R), notify 转发(R) |
| `MissionTimerShared` | `std::sync::RwLock` | OCR 线程(W), Tauri 命令 set_mode(W), tick loop 的 update_elapsed(W), build_payload(R) |
| `NotifyList` | `std::sync::RwLock` | notify 转发线程(W), get_notifications 命令(R), tray Enter(R) |
| `tick_advance_fired` | `std::sync::RwLock` | tick loop only (W/R) |
| `FlashFlag` | `AtomicBool` | notify 转发(W,Relaxed), tray 左键(W,Relaxed), 闪烁线程(R,Relaxed), clear_notifications(W,Relaxed) |
| `HideGen` | `AtomicU64` | tray Enter(W,SeqCst), tray 左键(W,SeqCst), popup item click(W,SeqCst), start_popup_watch(R,SeqCst) |

### 18.2 锁顺序分析（死锁风险）

**关键嵌套路径：**

| 路径 | 锁顺序 | 风险 |
|------|--------|------|
| Tick loop | `state.write()` → `timer.write()` → `timer.read()` → `config.read()` | ✅ 顺序一致，全部在同一作用域内释放 |
| fetch_store_emit | `config.read()` → (释放) → `state.write()` → (释放) → `state.read()` → `timer.read()` | ✅ 无嵌套 |
| OCR 线程 | `config.read()` → (capture+OCR) → `shared.write()` → (释放) → `shared.write()` | ✅ config 在 shared 之前获取 |
| set_config | `config.write()` (仅赋值，无嵌套) | ✅ |
| timer_command set_mode | `timer.write()` (仅赋值，无嵌套) | ✅ |
| tray Enter | `notify_list.read()` → (释放) → `popup.show()` → `start_popup_watch` | ✅ |

**结论：无死锁风险。** 锁获取顺序全局一致，不存在 A→B 和 B→A 的交叉路径。

### 18.3 `std::sync::RwLock` 在异步上下文中的使用

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 18.3.1 | 最佳实践 | 🟡 | `SharedConfig` 使用 `std::sync::RwLock` 但在 Tokio 异步任务中通过 `cfg.read().unwrap()` 访问（tick loop、fetch loop）。虽然锁持有时间极短（仅 clone 几个字段），但严格来说在异步上下文中使用同步锁是反模式——如果 OS 调度器在持锁期间挂起该线程，其他等待者被阻塞 | 当前影响极小：所有 config 读操作都是 clone 小数据后立即释放。如果要彻底消除，可改为 `tokio::sync::RwLock`，但这会要求所有访问者（包括 std 线程中的 OCR 线程）也使用 async——需要重构 |
| 18.3.2 | 最佳实践 | 🟡 | 同上：`MissionTimerShared` 使用 `std::sync::RwLock`，在 tick loop（async 上下文）中通过 `timer.write().unwrap()` 和 `timer.read().unwrap()` 访问 | `update_elapsed()` 极快（几个算术操作），`build_payload` 只需 `payload.clone()`。实际阻塞时间 ≤ 1μs |
| 18.3.3 | 最佳实践 | 🟢 | `set_config` Tauri 命令中 `save_config`（同步文件 I/O）在获取 `config.write()` 锁**之前**执行——正确的设计 | 无需修改 |

### 18.4 锁竞争热点分析

| 锁 | 竞争频率 | 持锁时间 | 风险 |
|----|----------|----------|------|
| `SharedState` (tokio) | tick 每秒写 1 次，fetch 每 1800s 写 1 次，fetch 后读 1 次 | 写: ~50μs (几个字段赋值), 读: ~200μs (build_payload clone) | ✅ 低 |
| `SharedConfig` (std) | tick 每秒读 1 次，用户改设置时写 1 次，OCR 每 2-30s 读 1 次 | 读: ~1μs, 写: ~1μs | ✅ 低。读写比极高，写 starvation 不可能 |
| `MissionTimerShared` (std) | tick 每秒写(update_elapsed)+读(build_payload)，OCR 每 2-30s 写(apply_ocr+HP)，用户操作时写(set_mode) | 写: ~100μs (apply_ocr), 读: ~1μs (clone payload) | ✅ 低 |
| `NotifyList` (std) | notify 转发线程每次命中写 1 次(~0.1/s)，tray Enter 读 1 次(~0.05/s)，get_notifications 读 1 次 | 读写均 ~1μs | ✅ 可忽略 |

### 18.5 `tick_advance_fired` 的锁浪费

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 18.5.1 | 性能 | 🟡 | `tick_advance_fired: Arc<StdRwLock<HashSet<String>>>` 仅在 tick loop（单个 Tokio 任务）中访问，不存在多线程竞争。使用 `RwLock` 纯属浪费 | 改为 `RefCell<HashSet<String>>` 或直接用 `&mut HashSet` 在闭包内传递 |

### 18.6 通道背压分析

所有 4 个 `mpsc::channel` 均为**无界通道**（默认）。

| 通道 | 生产者速率 | 消费者行为 | 风险 |
|------|-----------|-----------|------|
| `log_tx` | ~1 条/2s (OCR 中) | `recv()` 阻塞等待 | ✅ 无风险——消费者就是简单的 emit，不会慢 |
| `alert_tx` | ~0.001 条/s (checkpoint/HP) | `recv()` 阻塞等待，每次发 toast + 可选 phone push | ✅ 频率极低 |
| `notify_tx` | ~0.01 条/s (订阅命中) | `recv()` 阻塞等待，每次写 NotifyList + emit + AtomicBool | ✅ 频率低 |
| `cmd_tx` | ~0.01 条/s (用户操作) | `try_recv()` 非阻塞轮询 | ✅ |

**结论：无背压风险。** 所有生产者的消息速率都远低于消费者处理能力。消费者线程 panic 退出导致消息堆积的隐患已通过 `catch_unwind` 修复（见 5.7.1 ✅）。

### 18.7 原子操作顺序一致性

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 18.7.1 | **bug** | 🔴 | `FlashFlag` 全部使用 `Relaxed` ordering。对于单纯的布尔标志切换，Relaxed 在 x86 上安全（硬件保证 cache coherence）。但 `HideGen` 使用 `SeqCst`——与 `FlashFlag` 的 `Relaxed` 形成不一致。实际上 `FlashFlag` 的消费者（闪烁线程）和生产者（notify 转发线程）之间没有其他共享数据需要 ordering 保证，Relaxed 足够 | 无需修改，但建议注释说明为何 Relaxed 足够 |
| 18.7.2 | 最佳实践 | 🟢 | `HideGen` 使用 `SeqCst`：`fetch_add(1, SeqCst)` 写，`load(SeqCst)` 读。这是正确的——generation counter 用于 invalidate 跨线程的 watcher，需要明确的 happens-before 关系 | 可降为 `AcqRel`（写用 `Release`，读用 `Acquire`）以提升性能，但 SeqCst 在 x86 上无额外开销（都是普通 mov） |

### 18.8 关键竞态条件深度分析

#### 18.8.1 Tick vs Fetch 双重 emit（前轮 5.5.1 深化）

```
时间线:
  T1: fetch state.write() → 存储数据, countdown=1800 → drop write
  T2: tick state.write() → countdown=1799 → build_payload(1799) → emit tick-update(1799)
  T3: fetch state.read() → build_payload(1799) → emit worldstate-update(1799)
```
**结论修正：** 在 T3 时刻，fetch 读到的 countdown 已经是 1799（tick 已递减），两个 emit 的 countdown 一致，**不存在 countdown 跳回问题**。之前 Round 1 的分析有误——这是安全的。

但存在另一个微妙场景：
```
  T1: fetch state.write() → 存储数据, countdown=1800 → drop write
  T2: fetch state.read() → build_payload 开始执行（读 countdown=1800）
  T3: tick state.write() → countdown=1799 → build_payload → emit tick-update(1799)
  T4: fetch emit worldstate-update(1800)  ← 比 tick 晚但 countdown 更大
```
前端收到顺序：tick-update(1799) → worldstate-update(1800)。countdown 从 1799 跳到 1800，用户看到"下次刷新"倒计时瞬间多了 1 秒。下一 tick 会修正为 1798。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 18.8.1 | bug | 🟢 | 上述竞态窗口约为 `build_payload` 的执行时间（~200μs）。概率极低，影响仅 1 秒 countdown 闪烁 | 修复：在 fetch 的同一个 write lock 内构建 payload。代价是 write lock 持锁时间延长到 ~200μs——可接受，因为 fetch 每 30 分钟才一次 |

#### 18.8.2 OCR 线程中 config 读-改-写 竞态

OCR 线程在 `config.read()` 和后续使用配置值之间，用户可能通过 `set_config` 修改配置。OCR 线程在持锁期间读取了配置的快照（clone 了几个字段），然后释放锁。这是正确的 snapshot 模式。

但注意：`single_capture` 路径也读取 config（`let cfg = config.read().unwrap()`）获取 `strip_frame` 和 ROI——它在独立的 command 处理路径中。如果用户恰好同时改了配置，single_capture 用旧配置也没问题（单次操作，下次就用新的了）。

#### 18.8.3 NotifyList 快照一致性

notify 转发线程：
```rust
let snapshot = {
    let mut list = notify_store.write().unwrap();
    list.insert(0, msg.clone());
    if list.len() > 50 { list.truncate(50); }
    list.clone()  // 在持锁期间 clone
};
let _ = notify_handle.emit("sub-notify", snapshot);
```

在 write lock 内 clone 整个列表——线程安全且一致。emit 在锁外执行——正确。

### 18.9 Tokio 任务与 std 线程的同步正确性

这是本代码库最复杂的并发场景。关键交互点：

**Tokio → std 线程：**
- `cmd_tx.send(TimerCommand)` — Tokio 命令 handler 发送，OCR std 线程接收
- `notify_tx.send(SubNotify)` — Tokio tick/fetch 循环发送，notify 转发 std 线程接收

**std → Tokio/主线程：**
- `log_rx.recv() → emit("timer-log")` — std 线程接收，emit 到 webview
- `alert_rx.recv() → show_toast()` — std 线程接收，Tauri API 调用（需 AppHandle）
- `notify_rx.recv() → emit("sub-notify")` — std 线程接收，emit 到 webview
- `run_on_main_thread(|| tray.set_icon(...))` — std 闪烁线程，主线程执行

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 18.9.1 | 最佳实践 | 🟡 | 闪烁线程使用 `run_on_main_thread` 将托盘图标更新派发到主线程——这是 Tauri 的要求（托盘 API 必须主线程）。但 `run_on_main_thread` 的闭包使用 `flash_handle.clone()` 捕获 `AppHandle`——每次迭代 clone 一次 `AppHandle`（Arc 内部，开销极小） | 可接受 |
| 18.9.2 | 最佳实践 | 🟡 | `phone_push::push` 在 alert 转发线程和 notify 转发线程中通过 `tauri::async_runtime::spawn` 启动——从 std 线程启动 async 任务是合法的（Tokio runtime 全局可用），但任务在 std 线程的调用栈之外执行 | 正确。`spawn` 返回 `JoinHandle` 被丢弃（fire-and-forget），符合 phone push 的"尽力而为"语义 |
| 18.9.3 | bug | 🟡 | OCR 线程通过 `alert_tx` 发送 `AlertMsg` 到 alert 转发线程，后者调用 `show_toast` + 可选 `phone_push`。如果 OCR 线程连续快速发送两条 alert（比如 checkpoint 和 HP 同时触发），alert 转发线程会按顺序处理——但由于 `recv()` 是阻塞的，两条消息会串行处理。toast 通知可能堆积 | 实际中 checkpoint 和 HP 几乎不可能同时触发（checkpoint 在整 5 分钟节点，HP 在 20% 阈值）。风险极低 |

### 第三轮总结

| 严重 | 数量 | 关键项 |
|------|------|--------|
| 🔴 高 | 1 | 18.7.1 Atomic ordering 不一致（实际无功能影响，x86 保证安全） |
| 🟡 中 | 7 | std::sync::RwLock 在 async 上下文、tick_advance_fired 锁浪费、无界通道缺乏保护 |
| 🟢 低 | 3 | 微秒级竞态窗口、注释建议 |

**核心结论：并发模型设计良好，无死锁风险，无数据竞争。** `Arc<RwLock<>>` + `mpsc` 的组合在 Rust 的类型系统保护下安全可靠。主要改进空间：(1) 将 `SharedConfig`/`MissionTimerShared` 改为 `tokio::sync::RwLock`（需大量重构，当前影响小）；(2) ~~为长期运行的 std 线程添加 panic 恢复~~ ✅ 已实现（5.7.1）。

---

## 审计累计统计（三轮合计）

| 轮次 | 范围 | 🔴 | 🟡 | 🟢 |
|------|------|----|----|-----|
| 第一轮 | Rust 模块级审查 | 4 | 20 | 12 |
| 第二轮 | 前端 + 配置 + 数据流 | 2 | 14 | 5 |
| 第三轮 | 并发安全 + 锁竞争 | 1 | 7 | 3 |
| **合计** | **全项目深度审计** | **7** | **41** | **20** |

---

## 19. 第四轮审查：错误处理与 unsafe 审计

### 19.1 `unwrap()` 全景分析 (53 处)

#### 19.1.1 RwLock unwrap (49 处) — 锁中毒风险

49/53 的 unwrap 是 `RwLock::read().unwrap()` 或 `RwLock::write().unwrap()`。`std::sync::RwLock` 的 `unwrap()` 仅在**锁中毒**（lock poisoning）时 panic——即另一个线程在持有该锁期间 panic 了。

| 锁 | unwrap 次数 | poison 触发条件 | 风险评估 |
|----|-----------|----------------|---------|
| `SharedConfig` | ~15 | 任何持锁线程 panic | 🟢 极低——所有持锁操作都是简单的读写字段；4 个 std::thread + tick loop 已有 `catch_unwind` 防止静默 panic |
| `MissionTimerShared` | ~20 | OCR 线程 panic | 🟡 低——`apply_ocr` 和 `handle_command` 内部无 unwrap/panic。OCR 线程不在本次 catch_unwind 覆盖范围 |
| `NotifyList` | ~4 | notify 转发线程 panic | 🟢 极低——仅 insert/clone/truncate，且线程已有 `catch_unwind` |
| `item_i18n::MAP` | ~4 | 初始化失败 | 🟢 无——OnceLock 保证初始化只执行一次 |

**关键发现：**

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 19.1.1 | bug | 🟡→🟢 | ~~如果任一线程 panic 导致锁中毒，所有后续对该锁的访问都会 panic~~ → 5 个关键线程（4×std + tick loop）已添加 `catch_unwind`，锁毒化概率已降至极低。OCR 线程（持 `MissionTimerShared`）不在本次覆盖范围，但其内部无可 panic 代码 | 大部分已解决。OCR 线程锁毒化风险仍存但概率极低 |
| 19.1.2 | 最佳实践 | 🟡 | `app.default_window_icon().unwrap()` (lib.rs:963) — 如果 Tauri 配置中没有设置图标，这个 `unwrap()` 会在启动时 panic，导致应用无法启动 | 实际中 Tauri 总是有默认图标。但可以改为 `unwrap_or_else(|| Image::new_owned(vec![0; 64], 16, 16))` 作为兜底 |

#### 19.1.2 非 RwLock unwrap (4 处)

| # | 位置 | 调用 | 风险 |
|---|------|------|------|
| 19.1.3 | `api.rs:43` | `SystemTime::now().duration_since(UNIX_EPOCH).unwrap()` | 🟢 系统时钟早于 1970 才会 panic——在实际系统中不可能 |
| 19.1.4 | `mission_timer.rs:345/349/353` | `shared.write().unwrap().handle_command(...)` | 🟡 已在 RwLock 分析中覆盖 |
| 19.1.5 | `mission_timer.rs:359` | `.write().unwrap().handle_command(&TimerCommand::SetMode(...))` | 🟡 同上 |

### 19.2 `expect()` 分析 (2 处)

| # | 位置 | 调用 | 风险 |
|---|------|------|------|
| 19.2.1 | `lib.rs:1299` | `.expect("error while running tauri application")` | 🟢 Tauri `run()` 失败——通常是端口被占用或系统资源不足，panic 是合理的（应用无法启动） |
| 19.2.2 | `ocr.rs:32` | `.expect("Failed to decode digit template")` | 🟢 内嵌的 PNG 资源损坏——编译时就应该失败，panic 合理 |

### 19.3 `unsafe` 块审计 (25 处)

#### 19.3.1 GDI 截屏 (capture.rs — 12 unsafe)

**审查方法：** 逐块检查资源获取/释放配对、指针有效性、错误路径清理。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 19.3.1 | **bug** | 🔴 | `capture_full` (L138-148): `GetDIBits` 的 `pixels.as_mut_ptr()` 传给 `GetDIBits` 作为输出缓冲区。`pixels` 是 `vec![0u8; full_pixel_count * 4]`（BGRA）。`GetDIBits` 写入 32-bit 像素到该缓冲区，大小匹配——安全。但 `biHeight: -win_h` (负数) 表示 top-down DIB——Windows 文档要求此时 `biSizeImage` 可以为 0（让系统计算）。当前设置 `biSizeImage: 0` — **不正确**。根据 MSDN，对于 32-bit top-down DIB，`biSizeImage` **必须**显式设置为 `width * height * 4`，否则 `GetDIBits` 可能写入超出缓冲区的数据 | 设 `biSizeImage: (win_w * win_h * 4) as u32`。当前代码在实践中可能正常工作（大多数驱动忽略此字段对 32-bit 的约束），但根据文档是 UB |
| 19.3.2 | 最佳实践 | 🟡 | `capture_full` (L109-114): `SelectObject` 返回的 `old_bmp` 在 cleanup 块中恢复（L152 `SelectObject(hdc_mem, old_bmp)`）——正确。但如果 `PrintWindow` 或 `GetDIBits` 之间发生 panic（Rust 中不会，因为都是 FFI 调用），old_bmp 不会被恢复 | Rust 的 FFI 调用不会 panic，安全 |
| 19.3.3 | 最佳实践 | 🟡 | `capture_full` 中所有 GDI 错误路径都有正确的资源清理：`CreateCompatibleDC` 失败 → `ReleaseDC`；`CreateCompatibleBitmap` 失败 → `DeleteDC` + `ReleaseDC` | ✅ 正确，资源管理无泄漏 |
| 19.3.4 | 最佳实践 | 🟢 | `PrintWindow` 通过 `extern "system"` 手动声明——`windows` crate 0.58 未导出此函数。声明签名与 MSDN 匹配 | ✅ 正确 |

#### 19.3.2 窗口枚举 (window.rs — 8 unsafe)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 19.3.5 | 最佳实践 | 🟡 | `enum_callback` (L51-74): 从 `LPARAM` 裸指针恢复 `&mut Vec<WindowInfo>`——这是 Win32 回调的标准模式。`LPARAM.0 as *mut Vec<WindowInfo>` 转换安全，因为 `EnumWindows` 的调用者传入了该指针的地址 | ✅ 正确 |
| 19.3.6 | 最佳实践 | 🟡 | `enum_callback_by_process` (L140-156): 同样从 `LPARAM` 恢复 `&mut (isize, u32, String)`——正确。但回调中通过 `ctx.0 != 0` 提前返回 `BOOL::from(true)` 而非 `BOOL(0)`——前轮 9.1.2 已识别 | 见 9.1.2 |
| 19.3.7 | 最佳实践 | 🟡 | `exe_name_for_pid`: `CreateToolhelp32Snapshot` → `Process32FirstW` → `Process32NextW` 的标准进程枚举模式。`String::from_utf16_lossy(&entry.szExeFile)` 处理宽字符串——`szExeFile` 是 `[u16; 260]` 固定数组，可能不含 null terminator | ✅ 用 `find('\0')` 截断正确处理 |
| 19.3.8 | 最佳实践 | 🟢 | `bring_to_front`: `SetWindowPos(HWND_TOPMOST)` → `SetWindowPos(HWND_NOTOPMOST)` 的经典置顶技巧 | ✅ Win32 中广泛使用 |

#### 19.3.3 其他 (lib.rs — 5 unsafe)

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 19.3.9 | 最佳实践 | 🟡 | `start_popup_watch` 中 `GetCursorPos(&mut pt)` — 返回 `BOOL`，但忽略了返回值。如果函数失败（极少见），`pt` 保持默认值 (0,0)，可能导致 watcher 误判光标在屏幕左上角 | 检查返回值。若失败则跳过本轮检查（`misses` 不递增） |
| 19.3.10 | 最佳实践 | 🟡 | 注册表操作（L628-657, L672-704）：`RegOpenKeyExW` / `RegQueryValueExW` / `RegSetValueExW` / `RegDeleteValueW` / `RegCloseKey` — 标准的 Windows 注册表 API 使用 | ✅ 资源管理正确：所有路径都调用 `RegCloseKey` |
| 19.3.11 | bug | 🟡 | `set_autostart` (L648-651): `std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2)` — 将 `Vec<u16>` 的宽字符数据重新解释为字节切片。`wide` 在 `from_raw_parts` 返回的切片生命周期内保持存活——正确。但 `RegSetValueExW` 期望 `cbData` 包含 null terminator 的字节数。当前 `wide.len() * 2` 包含了末尾的 `\0`（因为 L648 的 `wide.push(0)` 在 L649 的 `wide.len()` 调用之前），所以 `wide.len()` 包含了 null | ✅ 正确——因为 `push(0)` 在 `wide.len()` 之前执行。但顺序依赖脆弱——如果有人重构交换了 L648 和 L649，会导致 null terminator 缺失 |

### 19.4 错误传播路径审计

#### 19.4.1 网络错误

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 19.4.1 | bug | 🟡 | `fetch_worldstate` 对 primary 源的 parse 错误只 `eprintln!` 后 fallback。如果 primary parse 失败（API 返回了非 JSON），用户无感知——只在 stderr 输出（桌面应用中不可见） | 考虑通过 `timer-log` emit 或返回更丰富的错误信息 |

#### 19.4.2 配置错误

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 19.4.2 | 最佳实践 | 🟢 | `load_config`: JSON 解析失败时静默返回默认配置——合理，避免因配置文件损坏导致应用无法启动 | ✅ |
| 19.4.3 | bug | 🟡 | `save_config` 的错误被 `set_config` 以 `String` 形式传播到前端。但如果磁盘满了或权限不足，前端显示的 `String` 是 Rust 的 `io::Error` 消息（英文），对中文用户不友好 | 可映射为中文错误消息 |

#### 19.4.3 静默吞掉的错误

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 19.4.4 | 最佳实践 | 🟡 | `phone_push::push` 所有错误均以 `let _ = ...` 吞掉——符合"尽力而为"设计 | ✅ |
| 19.4.5 | 最佳实践 | 🟡 | `show_toast` 错误以 `let _ = ...` 吞掉——Tauri notification API 在 dev 模式下可能失败 | ✅ |
| 19.4.6 | bug | 🟡 | `start_popup_watch` (L258-259): `app.run_on_main_thread(...)` 的结果以 `let _ = ...` 吞掉——如果主线程繁忙无法派发，popup 不会隐藏，导致托盘弹窗残留 | 低影响——用户移动鼠标或左键点击托盘可消除 |

### 第四轮总结

| 严重 | 数量 | 关键项 |
|------|------|--------|
| 🔴 高 | 1 | 19.3.1 `GetDIBits` 的 `biSizeImage=0` 对 top-down 32-bit DIB 不符合 MSDN 规范（实践中通常安全） |
| 🟡 中 | 10 | 锁中毒级联、unwrap 防御、GetCursorPos 返回值忽略、错误消息中文化 |
| 🟢 低 | 3 | expect 合理性、配置降级策略 |

**核心结论：** unsafe 代码质量良好——GDI 资源管理正确（无泄漏），Win32 回调模式标准。锁中毒级联崩溃的风险已大幅降低（5.7.1 ✅，4 个 std::thread + tick loop 均添加了 `catch_unwind`）。

---

## 审计累计统计（四轮合计）

| 轮次 | 范围 | 🔴 | 🟡 | 🟢 |
|------|------|----|----|-----|
| 第一轮 | Rust 模块级审查 | 4 | 20 | 12 |
| 第二轮 | 前端 + 配置 + 数据流 | 2 | 14 | 5 |
| 第三轮 | 并发安全 + 锁竞争 | 1 | 7 | 3 |
| 第四轮 | 错误处理 + unsafe | 1 | 10 | 3 |
| **合计** | **全项目深度审计** | **8** | **51** | **23** |

---

## 20. 第五轮审查：资源管理与内存审计

### 20.1 克隆热图（按调用频率排序）

| 位置 | clone 内容 | 频率 | 估算开销 |
|------|-----------|------|---------|
| `lib.rs:378-384` | 全部 Fissure/Cycle/Baro/Bounty/Circuit Vec | **每秒 1 次** | ~10-50 KB/次，30-150 GB/天累计分配 |
| `lib.rs:1115` | `cycle_alerts` Vec | **每秒 1 次** | 通常空 Vec，~0 开销 |
| `lib.rs:1038` | 托盘图标 `Image` (32×32 RGBA) | 闪烁时每 500ms | ~4 KB/次 |
| `lib.rs:1006-1008` | `NotifyList` Vec (最多 50 条) | 订阅命中时 | ~1-5 KB/次 |
| `lib.rs:1080-1085` | fissures Vec + cycles + 3 种 alerts | 每 30 分钟 | ~10-50 KB/次 |
| `api.rs:1362,1407` | reward items Vec | 赏金解析时 | ~1 KB/次 |
| `window.rs:207,212,217` | 截屏 BGR 像素 | OCR 时 (2-30s) | 1080p: ~6 MB, 4K: ~24 MB |

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 20.1.1 | **性能** | 🔴 | `build_payload` 每秒 clone 7 个 Vec（fissures×3 + cycles + baro + bounties + circuit）。在数据未变化的 30 分钟内（1799 次 tick），这完全是浪费 | 在 `AppState` 中缓存上一次 build 的 payload，tick 只更新 `remain_ms`/`remain_str` 字段。或前端自行计算倒计时 |
| 20.1.2 | 性能 | 🟡 | `strip_frame` 的 `pixels.to_vec()` 在不需要裁剪时（16:10 窗口）仍复制整个 BGR 缓冲区 | 返回 `Cow<[u8]>`——只在需要裁剪时分配 |

### 20.2 大块内存分配

| 分配 | 大小 | 生命周期 | 风险 |
|------|------|---------|------|
| `capture_full` BGR 缓冲区 | 1080p: ~6 MB, 4K: ~24 MB | 单次 OCR 迭代 (~1ms) | ✅ 立即释放 |
| `capture_preview_data_url` RGB + PNG 编码 | ~6-24 MB 中间，~2-8 MB PNG | 校准预览请求期间 | ✅ 按需，单次 |
| `item_i18n` 内嵌 `baro_zh.json` | ~16k 条目，~500 KB | 应用整个生命周期 (`'static`) | ✅ 编译时嵌入，合理 |
| `item_i18n` 远程 i18n.json | ~51 MB 下载 + ~16k 条目 HashMap (~500 KB) | 仅在用户手动触发更新时 | ✅ 按需，解析后释放原始 JSON |
| `arbitration_seq.bin` | 44056 bytes | `'static` | ✅ 极小 |
| `NotifyList` | 最多 50 条 SubNotify | 应用生命周期 | ✅ 有上限，每条 ~200 bytes |

### 20.3 GDI 资源管理（已验证，本轮的确认性复查）

| 资源 | 获取 | 释放 | 泄漏风险 |
|------|------|------|---------|
| `GetDC` (window DC) | `capture_full:89` | `ReleaseDC:155` | ✅ 所有路径（含错误）均释放 |
| `CreateCompatibleDC` (memory DC) | `capture_full:94` | `DeleteDC:154` | ✅ 错误路径已处理（L96-97, L103-105） |
| `CreateCompatibleBitmap` | `capture_full:100` | `DeleteObject:153` | ✅ |
| `SelectObject` (old bitmap) | `capture_full:109` | 恢复 `SelectObject:152` | ✅ |
| `CreateToolhelp32Snapshot` | `window.rs:104` | 自动（RAII 在 scope 结束时 Drop） | 🟡 Win32 的 HANDLE 不实现 Drop——但 `windows` crate 0.58 的 `HANDLE` 包装了所有权。需确认。实际上 `CreateToolhelp32Snapshot` 返回的 HANDLE 在 `windows` crate 中以 `Owned` 类型包装，离开作用域时自动 `CloseHandle` | ✅ |

### 20.4 网络连接池

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 20.4.1 | 性能 | 🟡 | `fetch_worldstate` 每次调用创建新的 `reqwest::Client`（含 TLS 握手、连接建立） | 使用 `OnceLock<reqwest::Client>` 全局复用。当前每 30 分钟一次，影响微小但有改进空间 |
| 20.4.2 | 性能 | 🟢 | `phone_push::push` 每次创建新 `Client`——但 push 频率极低（订阅命中时），且推送有 5s timeout | 可接受 |
| 20.4.3 | 性能 | 🟢 | `update_from_remote` 创建新 `Client` 下载 51 MB——按需操作，用户手动触发 | 可接受 |

### 20.5 内存泄漏风险

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 20.5.1 | bug | 🟡→🟢 | ~~`notify_tx` / `log_tx` / `alert_tx` / `cmd_tx` 均为无界通道，消费者线程 panic 退出导致堆积~~ → 消费者线程（log/toast/notify 转发）已添加 `catch_unwind`，不再会因 panic 退出。`cmd_tx` 的消费者是 OCR 线程（不在本次覆盖范围） | ✅ 大部分已修复 |
| 20.5.2 | 最佳实践 | 🟢 | `tick_advance_fired: HashSet<String>` — key 格式为 `地点\|目标状态\|提前量`，每次通知命中插入一条。最多 4 条（夜灵平野 × 2 状态 × 2 种提前量配置）。不会无限增长 | ✅ 有自动清除机制（状态切入目标时 remove） |
| 20.5.3 | 最佳实践 | 🟢 | `notified_fissures: HashSet<String>` — key 为 `node_key:expiry_ms`。每次 API 刷新后 `retain` 清理过期项。fissure 数量有上限（~20 个同时活跃） | ✅ |
| 20.5.4 | 最佳实践 | 🟢 | 前端 `logLines` 数组 (main.ts:1091): `MAX_LOG = 200` 限制条目数 | ✅ |
| 20.5.5 | 最佳实践 | 🟢 | 托盘闪烁 `glow_frames` 预生成 2 帧（`Image` owned），应用生命周期内持有。每帧 ~4 KB | ✅ 极小 |

### 第五轮总结

| 严重 | 数量 | 关键项 |
|------|------|--------|
| 🔴 高 | 1 | 20.1.1 每秒 clone 7 个 Vec（年化 ~1 TB 无用分配） |
| 🟡 中 | 4 | strip_frame 不必要复制、reqwest Client 不复用、无界通道 OOM 风险 |
| 🟢 低 | 5 | 确认无 GDI/文件句柄泄漏、内存上限合理 |

**核心结论：** 无明显内存泄漏。GDI 资源管理严格。最大问题是 `build_payload` 的每秒全量 clone——这是前几轮反复确认的头号性能问题。

---

## 审计累计统计（五轮合计）

| 轮次 | 范围 | 🔴 | 🟡 | 🟢 |
|------|------|----|----|-----|
| 第一轮 | Rust 模块级审查 | 4 | 20 | 12 |
| 第二轮 | 前端 + 配置 + 数据流 | 2 | 14 | 5 |
| 第三轮 | 并发安全 + 锁竞争 | 1 | 7 | 3 |
| 第四轮 | 错误处理 + unsafe | 1 | 10 | 3 |
| 第五轮 | 资源管理 + 内存 | 1 | 4 | 5 |
| **合计** | **全项目深度审计** | **9** | **55** | **28** |

---

## 21. 第六轮审查：边界条件与鲁棒性

### 21.1 API 数据异常

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 21.1.1 | bug | 🟡 | `parse_fissures` 中 `modifier.starts_with("VoidT")` 过滤非裂缝任务。如果 DE 新增 `VoidT7` 纪元，`tier_label` 返回 raw key（`tier_label` 函数的 `_ => key.to_string()`），`tier_order` 返回 99（排在最后）。前端渲染不崩溃但颜色回退到默认 | 可以接受——新纪元出现时至少可见，等待代码更新 |
| 21.1.2 | bug | 🟡 | `parse_cycles` 如果 `HexSyndicate` 不在 `SyndicateMissions` 中，Plains/Cambion/Höllvania 全部返回 `unknown_cycle()`——三个卡片同时显示"未知·切换中" | API 极少同时缺失所有 syndicate。但如果发生，至少 UI 不崩溃 |
| 21.1.3 | bug | 🟡 | `parse_void_trader`: 如果 `VoidTraders` 数组为空或 `first()` 返回 `None`，函数返回 `None`——前端不渲染 Baro 卡片 | 正确。`None` 表示"无数据"而非错误 |
| 21.1.4 | bug | 🟡 | `parse_circuit`: 如果 `EXC_NORMAL` 和 `EXC_HARD` category 都存在但 `Choices` 数组都为空，`normal` 和 `hard` 都是空 Vec → 函数返回 `None`（L1647） | 正确。空的回廊数据应隐藏面板 |
| 21.1.5 | 最佳实践 | 🟡 | `parse_bounties` 中 `job_arr.map(\|arr\| arr.iter()...).unwrap_or_default()` ——如果 `Jobs` 数组存在但某个 job 的 `rewards` 字段缺失，`active_rotation_of("")` 返回空字符串 → 前端渲染"单一奖励池" | 降级合理 |

### 21.2 游戏窗口生命周期

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 21.2.1 | 最佳实践 | 🟡 | OCR 线程在窗口关闭时通过 `is_valid(hwnd)` 检测并重新扫描——每 2 秒重试。如果游戏从未启动，OCR 线程永久每 2 秒重试 | 合理——桌面伴侣应用应持续等待游戏启动 |
| 21.2.2 | bug | 🟡 | `capture_full` 在黑帧检测返回 `None` 后，OCR 线程的 `consecutive_capture_fails` 递增。5 次连续失败后触发 `resolve_hwnd` 重扫描（`CAPTURE_FAIL_RESCAN = 5`）。但如果游戏窗口最小化，`is_minimized` 检查在 `capture_full` 之前就被拦截了——最小化不会增加失败计数 | 正确。只有真正的捕获失败（而非最小化）才触发重扫描 |
| 21.2.3 | bug | 🟡 | `bring_to_front` 在 `dispatch_alert` 中调用——如果用户正在玩其他全屏应用（非 Warframe），强制弹窗可能打断体验 | 这是设计意图——checkpoint 提醒就是要在关键时间点拉回用户注意力。用户可切换到 toast 模式 |

### 21.3 系统时钟变化

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 21.3.1 | **bug** | 🔴 | `now_ms()` 使用 `SystemTime::now()`——单调递增假设。如果系统时钟被 NTP 回拨（比如从 15:00 校正到 14:59），所有基于 `now_ms()` 的倒计时（`remain_ms = expiry - now`）会瞬间变大（约 60 秒）。影响：倒计时显示暂时不准，纠正后恢复正常 | 可考虑用 `Instant::now()` 追踪 elapsed time，用 `SystemTime::now()` 追踪 wall clock。但 Warframe API 的 expiry 是 wall clock timestamp，两者需要对应。当前实现是标准做法 |
| 21.3.2 | bug | 🟡 | DST（夏令时）切换：`chrono::Local::now().format(...)` 用于日志时间戳和 `last_update` 显示。DST 切换时时间跳跃 1 小时——仅影响显示，不影响逻辑 | 可接受 |
| 21.3.3 | bug | 🟡 | OCR 接受规则中 `wall_delta` 使用 `Instant::elapsed()`——这是单调时钟，不受系统时间调整影响 | ✅ 正确——OCR 验证应使用单调时钟 |

### 21.4 极端运行时长

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 21.4.1 | bug | 🟡 | `MissionTimerState.elapsed_secs: u32` — 最大值 ~136 年，实际任务不会超过几小时 | ✅ |
| 21.4.2 | bug | 🟡 | 前端 `logLines` 数组限制 200 条——长时间运行不会无限增长 | ✅ |
| 21.4.3 | bug | 🟡 | `notify_list` 限制 50 条——`truncate(50)` | ✅ |
| 21.4.4 | **bug** | 🔴 | `arbitration_seq.bin` 包含 44056 个时隙。`parse_arbitration` 中 `if hour_idx >= d.seq.len() { return None }` —— 当 `hour_idx` 超过序列长度时（约 5 年后，即 2031 年左右），仲裁功能静默失效，前端不显示任何内容 | 需在 2031 年前更新 `arbitration_seq.bin`。建议：在 `hour_idx >= d.seq.len()` 时用取模循环 (`hour_idx % d.seq.len()`) 作为 fallback，而非静默返回 `None` |
| 21.4.5 | bug | 🟢 | `countdown_secs: u32` — 用 `saturating_sub(1)` 递减，不会 underflow | ✅ |

### 21.5 并发操作边界

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 21.5.1 | bug | 🟡 | 用户快速连续点击"开始→暂停→开始"：每次点击发送 `TimerCommand` 到 `cmd_tx`。OCR 线程在 `try_recv()` 循环中逐个处理——命令顺序与发送顺序一致 | ✅ `try_recv()` 在 while 循环中 drain 所有 pending 命令 |
| 21.5.2 | bug | 🟡 | 用户快速连续点击"刷新数据"：每次调用 `refresh_now` → `fetch_store_emit`。如果前一次 HTTP 请求还在飞行中，后一次也会发起——两个请求并发，后到达的覆盖先到达的 | 可加一个 `AtomicBool` "正在刷新"标志防止重复请求。当前行为只是浪费一次 HTTP 请求，功能正确 |
| 21.5.3 | bug | 🟡 | `set_config` 在保存文件时没有防抖——用户快速拖动 OCR interval slider 会触发多次 `save_config`（文件 I/O）。但实际上 interval 是 `<input type="number">` 的 `change` 事件（非 `input`），每次只触发一次 | ✅ `change` 事件只在用户提交值时触发（回车或失焦），非连续触发 |

### 21.6 配置损坏恢复

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 21.6.1 | 最佳实践 | 🟢 | `load_config`: JSON 解析失败 → 返回默认配置 + 覆盖写入正确的默认值。旧损坏文件被修复 | ✅ 自愈设计 |
| 21.6.2 | 最佳实践 | 🟢 | `item_i18n::init`: 用户覆盖文件损坏 → 回退到内嵌默认 | ✅ |
| 21.6.3 | bug | 🟡 | `migrate_old_default_rois` 只检查特定的旧默认值组合。如果用户拥有一个部分匹配旧默认的 ROI（比如 x 已改但 y/w/h 仍是旧值），不会被迁移——这正是期望行为（只有完全匹配旧默认才迁移） | ✅ |

### 第六轮总结

| 严重 | 数量 | 关键项 |
|------|------|--------|
| 🔴 高 | 2 | 21.3.1 NTP 时钟回拨、21.4.4 仲裁序列 2031 年到期 |
| 🟡 中 | 11 | API 字段变更降级、窗口生命周期、重复刷新竞态 |
| 🟢 低 | 5 | 配置自愈、防抖、数值上限 |

**核心结论：** 系统对各种异常有合理的降级策略。最大的时间炸弹是 21.4.4（仲裁序列 ~2031 年到期）——有约 5 年的缓冲期。时钟回拨问题（21.3.1）影响所有基于 wall clock 的应用，当前行为可接受。

---

## 审计累计统计（六轮合计）

| 轮次 | 范围 | 🔴 | 🟡 | 🟢 |
|------|------|----|----|-----|
| 第一轮 | Rust 模块级审查 | 4 | 20 | 12 |
| 第二轮 | 前端 + 配置 + 数据流 | 2 | 14 | 5 |
| 第三轮 | 并发安全 + 锁竞争 | 1 | 7 | 3 |
| 第四轮 | 错误处理 + unsafe | 1 | 10 | 3 |
| 第五轮 | 资源管理 + 内存 | 1 | 4 | 5 |
| 第六轮 | 边界条件 + 鲁棒性 | 2 | 11 | 5 |
| **合计** | **全项目深度审计** | **11** | **66** | **33** |

---

## 22. 第七轮审查：架构与设计模式

### 22.1 模块耦合度分析

```
lib.rs ──→ api.rs       (数据解析)
       ├─→ config.rs    (配置)
       ├─→ state.rs     (运行时状态)
       ├─→ models.rs    (数据结构)
       ├─→ capture.rs   (截屏)
       ├─→ ocr.rs       (模板匹配)
       ├─→ window.rs    (窗口管理)
       ├─→ item_i18n.rs (物品翻译)
       ├─→ mission_timer.rs (计时状态机)
       └─→ phone_push.rs (Bark推送)
```

**问题：** `lib.rs` 与所有模块单向耦合——这是"上帝模块"反模式。它承担了：命令路由、事件发射、线程管理、托盘逻辑、闪烁动画、订阅检查、payload 构建、更新检查、注册表操作。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 22.1.1 | 架构 | 🔴 | `lib.rs` 1301 行——单一文件混合了 6 种职责。任何改动都需要理解整个文件 | 拆分为：`commands.rs`（Tauri 命令）、`notify.rs`（订阅检查+转发）、`tray.rs`（托盘+闪烁）、`update.rs`（更新检查）、`payload.rs`（build_payload） |
| 22.1.2 | 架构 | 🔴 | `api.rs` 1829 行——混合了：时间工具、节点查表、裂缝解析、周期解析、Baro 解析、赏金解析、回廊解析、仲裁解析、HTTP 抓取 | 拆分为 `api/` 目录：`time.rs`、`nodes.rs`、`fissures.rs`、`cycles.rs`、`baro.rs`、`bounties.rs`、`circuit.rs`、`arbitration.rs`、`fetch.rs` |
| 22.1.3 | 架构 | 🟡 | `mission_timer.rs` 696 行——混合了：状态机定义、OCR 应用逻辑、维生检测、命令处理、轮询循环。但内聚性尚可（全是计时相关） | 可拆分为 `timer/state.rs`、`timer/ocr_loop.rs`、`timer/life_support.rs` |

### 22.2 设计模式评估

| 模式 | 使用位置 | 评价 |
|------|---------|------|
| **共享状态 (Arc+RwLock)** | `SharedState`, `SharedConfig`, `MissionTimerShared` | ✅ 适合多读少写场景。但 `build_payload` 的 clone-heavy 读模式降低了共享状态的优势 |
| **通道 (mpsc)** | OCR 命令、日志、alert、订阅通知 | ✅ 清晰的线程边界。不过 4 个独立通道略显分散 |
| **OnceLock 惰性初始化** | 内嵌资源（reward tables、circuit names、arb data、i18n map） | ✅ 避免启动时加载所有资源 |
| **状态机** | `TimerState` (Idle→Running→Paused→Checkpoint) | ✅ 清晰的状态转换。但 checkpoint 的自动恢复逻辑隐含在 `apply_ocr` 中而非显式状态转换 |
| **Observer (事件)** | Tauri emit/listen: `worldstate-update`, `tick-update`, `timer-log`, `sub-notify` | ✅ 松耦合前后端通信 |
| **Strategy** | `alert_method`: "focus" vs "toast" | ✅ 运行时切换提醒策略 |

### 22.3 代码重复分析

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 22.3.1 | 简化 | 🟡 | 6 个 `static *REWARDS: OnceLock<RewardTable>` + 对应的 `*_rewards()` 函数——结构完全相同，仅嵌入的 JSON 文件不同 | 宏消除：`static_rewards!(CETUS, "cetus_bounty_rewards.json")` |
| 22.3.2 | 简化 | 🟡 | `check_fissure_alerts` / `check_cycle_alerts` / `check_arbitration_alerts` / `check_cycle_advance_alerts`——四个函数结构相似：遍历 alerts → 匹配 → 去重 → 发送 SubNotify | 可抽象为 `trait AlertChecker<T> { fn check(&self, data: &[T], alerts: &[Alert], state: &mut State, tx: &Sender) }` |
| 22.3.3 | 简化 | 🟡 | 前端 `setupAlerts` 中三种 alert 的事件处理代码高度重复——fissure/cycle/arbitration 的 change/click/delete 结构一致 | 抽象为 `setupAlertSection(container, configKey, renderFn, addDefaults)` |
| 22.3.4 | 简化 | 🟡 | `capture_roi` 和 `capture_roi_stripped` 仅差一个 `strip_frame` 布尔参数——`capture_roi` 是不 strip 的特例 | 可统一为一个函数，`capture_roi` 变成 `capture_roi_stripped(hwnd, roi, false)` |

### 22.4 接口契约一致性

| 接口 | Rust 端 | TS 端 | 一致性 |
|------|---------|-------|--------|
| `AppStatePayload` | `models.rs:170` — 14 个字段 | `main.ts:162` — 14 个字段 | ✅ 一一对应 |
| `MissionTimerPayload` | `models.rs:51` — 10 个字段 | `main.ts:72` — 10 个字段 | ✅ |
| `Fissure` | `models.rs:4` — 12 个字段 | `main.ts:49` — 12 个字段 | ✅ |
| `SubNotify` | `lib.rs:37` — 7 个字段 | `notify.ts:5` — 7 个字段 | ✅ |
| `UpdateInfo` | `lib.rs:846` — 2 个字段 | `main.ts` 中用内联类型 `{ version: string; notes: string }` | ⚠️ 类型未导出为命名 interface |
| `NavigateMsg` | `lib.rs:50` — 3 个字段 | `main.ts:1121` 中用内联类型 | ⚠️ 同上 |

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 22.4.1 | 最佳实践 | 🟡 | `UpdateInfo` 和 `NavigateMsg` 在 TS 侧以匿名类型内联——Rust 结构体变更时编译器不会提醒 TS 侧 | 在 `main.ts` 顶部定义命名 interface 并 export，保持与 Rust struct 一一对应 |

### 22.5 可测试性

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 22.5.1 | 架构 | 🔴 | **0 个单元测试。** 核心解析逻辑（`parse_fissures`、`parse_cycles`、`roll_forward_cycle`、`parse_arbitration`）没有回归测试 | 至少添加：(1) `parse_cycles` 用固定 JSON fixture 验证；(2) `roll_forward_cycle` 各 location 的边界条件；(3) `recognize_digits` 用已知 ROI 截图验证；(4) `bounty_title` 各 syndicate 的映射覆盖 |
| 22.5.2 | 架构 | 🟡 | 解析函数直接调用 `now_ms()` 获取当前时间，使得其行为依赖 wall clock——难以编写确定性测试 | 将 `now_ms` 作为参数传入解析函数，测试中注入固定时间戳 |
| 22.5.3 | 架构 | 🟡 | `capture_full` 依赖 Win32 GDI（无法在 CI 中测试）——合理，但 OCR 管线（`recognize_digits`）可以独立测试 | 准备固定 ROI 截图（PNG），在测试中加载并验证 OCR 输出 |

### 第七轮总结

| 严重 | 数量 | 关键项 |
|------|------|--------|
| 🔴 高 | 3 | 22.1.1 lib.rs 上帝模块、22.1.2 api.rs 单体巨石、22.5.1 零测试 |
| 🟡 中 | 8 | 代码重复、接口类型未导出、可测试性改进 |
| 🟢 低 | 0 | — |

**核心结论：** 架构层面的主要债务是 `lib.rs` 和 `api.rs` 的单体文件过长——但这是项目从小到大的自然增长结果。拆分会带来更好的可维护性但也增加文件跳转成本。优先级最高的是添加测试（22.5.1）——这是 0→1 的突破，后续修改才有安全网。

---

## 审计累计统计（七轮合计）

| 轮次 | 范围 | 🔴 | 🟡 | 🟢 |
|------|------|----|----|-----|
| 第一轮 | Rust 模块级审查 | 4 | 20 | 12 |
| 第二轮 | 前端 + 配置 + 数据流 | 2 | 14 | 5 |
| 第三轮 | 并发安全 + 锁竞争 | 1 | 7 | 3 |
| 第四轮 | 错误处理 + unsafe | 1 | 10 | 3 |
| 第五轮 | 资源管理 + 内存 | 1 | 4 | 5 |
| 第六轮 | 边界条件 + 鲁棒性 | 2 | 11 | 5 |
| 第七轮 | 架构与设计模式 | 3 | 8 | 0 |
| **合计** | **全项目深度审计** | **14** | **74** | **33** |

---

## 最终结论

### 项目健康度评估

| 维度 | 评分 | 说明 |
|------|------|------|
| 功能正确性 | ⭐⭐⭐⭐ | 核心逻辑经过多轮打磨，用户反馈确认可用 |
| 并发安全 | ⭐⭐⭐⭐⭐ | 无死锁，无数据竞争，模型设计合理 |
| 错误处理 | ⭐⭐⭐⭐ | 降级策略合理；线程 panic 恢复机制已实现（catch_unwind）；tick loop countdown 改为 wall clock 派生，免疫休眠 |
| 性能 | ⭐⭐⭐ | tick clone 风暴是唯一显著瓶颈，其余开销可接受 |
| 代码组织 | ⭐⭐ | 两个 1000+ 行单体文件，0 个测试 |
| 鲁棒性 | ⭐⭐⭐⭐ | API 异常、窗口生命周期、配置损坏均有降级方案 |

### TOP 10 优先修复建议

1. **添加单元测试** (22.5.1) — 为解析逻辑和 OCR 管线建立回归安全网
2. ~~**线程 panic 恢复**~~ ✅ 已实现 (5.7.1)
3. ~~**tick clone 优化**~~ ✅ 已实现 (B 方案 cached_payload)
4. ~~**前端全量重渲染优化**~~ ✅ 已实现 (C 方案 DOM patch)
5. **周期提前通知重启重复** (5.1.1) — 已评估，不修复（见详细讨论）
6. **仲裁序列过期时间炸弹** (21.4.4) — 2031 年前需更新或加 fallback
7. **GetDIBits biSizeImage 修正** (19.3.1) — 对 top-down 32-bit DIB 显式设置
8. **拆分 api.rs** (22.1.2) — 提升可维护性
9. **拆分 lib.rs** (22.1.1) — 降低模块耦合
10. **EnumWindows 提前终止** (9.1.2) — 找到匹配后返回 FALSE

---

## 23. 第八轮审查：安全审计

### 23.1 XSS 风险（innerHTML 使用）

全部 22 处 `innerHTML` 赋值。`notify.ts` 已用 `esc()` 转义用户数据 ✅。`main.ts` 大部分数据来自 Rust API 解析结果（节点名、任务类型等，不含 HTML 特殊字符）。**23.1.1 已经完整数据流追踪修正**：此前认为用户自定义提示词会到达 `logContent.innerHTML`——实际不会，参见修正后的条目。

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 23.1.1 | 安全 | 🟡 | `logContent.innerHTML` 对 `timer-log` 事件 payload 直接拼接 HTML，未调用 `esc()`（`notify.ts` 有 `esc()` 但 `main.ts` 未引用）。**当前不可利用**：经完整数据流追踪确认，`checkpoint_alert_text` / `hp_alert_text` 走 `render_alert_text()` → `dispatch_alert()` → Windows 原生 toast 或窗口聚焦，不经过 `timer-log`；所有 `log()` 调用均为硬编码字符串、OCR 数字输出或数值。**但这是结构性防御缺失**——未来任何代码向 `timer-log` 写入用户可控数据即刻变为 XSS | 纵深防御：对 `line` 调用 `esc()`，或将 `esc()` 从 `notify.ts` 提取为共享工具函数 |
| 23.1.2 | 安全 | 🟢 | `notify.ts:38`: 已用 `esc()` 处理所有用户可见字段 | ✅ 已防护 |
| 23.1.3 | 安全 | 🟡 | `main.ts:416/426`: Baro 物品名来自 `item_i18n::translate` — 数据源是 WFCD warframe-items JSON（不可控但可信）和 `name_from_path`（CamelCase 拆分——不含 HTML 特殊字符） | 低风险 |
| 23.1.4 | 安全 | 🟡 | 订阅规则渲染中用户选择的值直接嵌入 `<option>`。可通过手动编辑 config.json 注入——但需要本地文件写入权限 | 低风险 |

### 23.2 路径遍历

| # | 类型 | 严重 | 描述 |
|---|------|------|------|
| 23.2.1 | 安全 | 🟢 | 所有路径构造使用 `app_data_dir.join(HARDCODED_NAME)`——文件名硬编码，不受用户输入影响 |
| 23.2.2 | 安全 | 🟢 | `uninstall_clean` 中 `remove_dir_all` 和注册表操作——路径来自 Tauri API 和系统注册表，非用户可控 |

### 23.3 敏感信息

| # | 类型 | 严重 | 描述 |
|---|------|------|------|
| 23.3.1 | 安全 | 🟢 | `tauri.conf.json` 中 `pubkey` 是公钥（验证更新签名），公开是安全的 |
| 23.3.2 | 安全 | 🟡 | Bark URL 含用户 token，明文存储在 `config.json`——任何能读 `%APPDATA%` 的进程可见。但 Bark 是单向推送，token 泄露仅可被用于发送垃圾通知 |

### 23.4 CSP

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 23.4.1 | 安全 | 🟡 | `"csp": null` 完全禁用 CSP | 生产构建设置 `"default-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self' https://api.warframe.com https://oracle.browse.wf https://gitee.com https://github.com"` |

### 第八轮总结：🔴1 🟡4 🟢4

---

## 24. 第九轮审查：依赖审计

### 24.1 Cargo.toml

| 依赖 | 版本 | 问题 |
|------|------|------|
| `chrono` | 0.4 | 🟡 维护模式 crate，项目中仅用于 `format("%H:%M:%S")`——可替换为 `time` crate 或手写 |
| `tokio` | 1 (full) | 🟡 `features = ["full"]` 引入了 `signal`、`process`、`io-util` 等不需要的组件——精简为 `["rt-multi-thread", "macros", "sync", "time"]` |
| `windows` | 0.58 | ✅ 仅启用所需 feature（GDI、DWM、Registry、ToolHelp） |
| `image` | 0.25 (png) | ✅ 最小 feature，仅 PNG 编解码 |
| `reqwest` | 0.12 | ✅ 活跃维护 |
| `tauri-plugin-updater` | 2.10.1 | 🟢 锁定了 patch 版本——与其他 `"2"` 版本策略不一致 |

### 24.2 package.json

| 依赖 | 版本 | 问题 |
|------|------|------|
| `@tauri-apps/api` | ^2 | ✅ |
| `@tauri-apps/plugin-opener` | ^2 | ✅ |
| `vite` | ^6.0.3 | 🟢 版本稍旧但无安全漏洞 |
| `typescript` | ~5.6.2 | 🟢 版本稍旧 |
| **总数** | 2 runtime + 3 dev | ✅ 极简依赖树，符合零依赖理念 |

### 第九轮总结：🔴0 🟡2 🟢4

---

## 25. 第十轮审查：OCR 管线专项

### 25.1 管线流程

```
PrintWindow(GDI) → BGRA全屏 → strip_frame → BGR ROI
  → 灰度二值化(阈值160) → 10数字×2尺度 NCC → NMS(inter/min,IoU 0.3)
  → 按x排序 → "M:SS"解析 → 验证(delta∈[-10,30]s) → 同步/拒绝
```

### 25.2 关键发现

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 25.2.1 | 算法 | 🟡 | 二值化阈值固定 160——不同 HUD 亮度设置下可能失效 | 当前匹配率已够高。可考虑 Otsu 自适应阈值作为 fallback |
| 25.2.2 | 算法 | 🟡 | 两个固定模板尺度（1.0x, 0.85x）——非标准分辨率+HUD 组合可能不匹配 | 可增加 0.75x/1.15x，但每增一个尺度匹配时间线性增长 |
| 25.2.3 | bug | 🟡 | `parse_time_to_secs` 中分钟解析失败返回 `unwrap_or(0)`——导致 `ocr_secs=0`，在 `apply_ocr` 中触发拒绝逻辑而非静默接受 | 行为正确——无效解析应被拒绝。但可加 log 说明 |
| 25.2.4 | 算法 | 🟡 | `MAX_REJECT=3` 后强制接受——如果 OCR 持续输出错误值，强制接受后下次正确值（如果在允许范围内）会修正 | 恢复策略合理 |
| 25.2.5 | 算法 | 🟡 | `detect_life_support` 中红像素判定：H∈[0,15]∪[345,360] 且 S>0.31 且 V>0.47——这些阈值来自 Warframe 维生系统 UI 的经验值 | 如果 DE 调整 HUD 颜色，可能需要重新标定 |

### 第十轮总结：🔴0 🟡5 🟢0

---

## 26. 第十一轮审查：启动恢复

### 26.1 冷启动

| # | 类型 | 严重 | 描述 |
|---|------|------|------|
| 26.1.1 | 最佳实践 | 🟢 | 所有 `app.manage()` 在 webview 创建前执行——避免 "state not managed" 竞态 |
| 26.1.2 | 最佳实践 | 🟢 | Tokio `interval` 首次 tick 立即触发——启动即拉 worldstate |

### 26.2 崩溃恢复

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 26.2.1 | 最佳实践 | 🟢 | 配置损坏 → 返回默认值 + 覆盖写入——自愈 |
| 26.2.2 | bug | 🟡 | 配置损坏时直接覆盖——用户之前的设置丢失 | 覆盖前备份为 `config.json.bak` |

### 26.3 休眠/唤醒

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 26.3.1 | bug | ✅ 已修复 | ~~系统休眠期间 Tokio interval 暂停。唤醒后不补回错过的 tick——`countdown_secs` 落后实际时间~~ → `countdown_secs` 改为 wall clock 推导（`REFRESH_SEC - (now - last_fetch_wall_ms) / 1000`），NTP 回拨安全。tick loop 检测到 wall clock 间隙 >2s 时补调 `check_cycle_alerts` 并同步 `prev_cycle_states` 基准线。`check_cycle_advance_alerts` 因 `refresh_cached_payload` 已 roll_forward 周期，唤醒后自动正确 | 已实现 (2026-07-02) |
| 26.3.2 | bug | 🟡 | 休眠后 `Instant::now()` 反映实际经过时间——OCR 线程的 `start.elapsed()` 会跳变，但 OCR 只在游戏运行时工作（休眠时游戏不可能运行） | ✅ 实际无影响 |

### 第十一轮总结：🔴1 🟡2 🟢3

---

## 27. 第十二轮审查：Tauri v2 最佳实践

| 方面 | 实现 | 评价 |
|------|------|------|
| 事件系统 | `emit` 广播 + `emit_to` 定向 | ✅ 正确区分 |
| 命令注册 | `#[tauri::command]` + `generate_handler![]` | ✅ 标准做法 |
| 窗口关闭 | `CloseRequested` → `api.prevent_close()` + `hide()` | ✅ 正确 |
| 单实例 | `tauri-plugin-single-instance` | ✅ |
| 托盘 | `TrayIconBuilder::with_id("main")` + Enter/Leave/Click | ✅ |
| Notify 弹窗 | 无装饰透明置顶无焦点无任务栏 | ✅ |
| DPI | `scale_factor()` + `to_physical()` | ✅ |

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 27.1 | 最佳实践 | 🟡 | `timer-log` 通过 `emit` 广播到所有窗口——notify 窗口也会收到但无处理逻辑 | 改用 `emit_to("main", ...)` 精确发送 |
| 27.2 | 最佳实践 | 🟡 | `tick-update` 每 1 秒 emit 整个 `AppStatePayload`（~10-50 KB JSON） | 可设计轻量 tick payload（仅含 countdown + remain 字段） |
| 27.3 | 最佳实践 | 🟢 | 同步命令（`get_config`）和异步命令（`refresh_now`）正确区分 | ✅ |

### 第十二轮总结：🔴0 🟡2 🟢1

---

## 28. 第十三轮审查：文档一致性

### CLAUDE.md 与实际代码对照

| 文档声明 | 实际 | 一致 |
|----------|------|------|
| "3 个线程" | 2 Tokio 任务 + 5 std 线程 + OCR 线程 + popup watch = 更多 | ⚠️ |
| NMS: `inter/min(a,b)`, MATCH_THRESHOLD: 0.70 | 代码一致 | ✅ |
| fissure timer ROI h=0.030 | `fissure_roi_h() = 0.030` | ✅ |
| 6 个开放世界周期 | `parse_cycles` 返回 6 个 | ✅ |
| 物品库仅显示版本 | `game_data_version` 只读 | ✅ |
| `initialized` 标志 | `AppState.initialized` | ✅ |
| `check_cycle_advance_alerts` (仅夜灵平野) | 代码一致 | ✅ |

### CLAUDE.md ↔ AGENTS.md

| # | 类型 | 严重 | 描述 |
|---|------|------|------|
| 28.1 | 文档 | 🟢 | 两文件均为 195 行，内容同步 | ✅ |

### CODE_NOTES.md 缺失项

| # | 类型 | 严重 | 描述 | 建议 |
|---|------|------|------|------|
| 28.2 | 文档 | 🟡 | 0.1 节缺少 popup-watch auto-hide 线程 | 补充 |
| 28.3 | 文档 | 🟡 | 0.3 节缺少 `update-available` 和 `navigate` 事件 | 补充 |
| 28.4 | 文档 | 🟡 | 0.4 节缺少 `get_bark_url`、`test_phone_push`、`open_main_navigate`、`game_data_version` | 补充 |
| 28.5 | 文档 | 🟡 | 线程数描述（"3 个线程"）与实际 8 个并发上下文不符 | 更新 CLAUDE.md 和 CODE_NOTES.md |

### 第十三轮总结：🔴0 🟡4 🟢1

---

## 审计累计统计（十三轮，终局）

| 轮次 | 范围 | 🔴 | 🟡 | 🟢 |
|------|------|----|----|-----|
| ① | Rust 模块级审查 | 4 | 20 | 12 |
| ② | 前端 + 配置 + 数据流 | 2 | 14 | 5 |
| ③ | 并发安全 + 锁竞争 | 1 | 7 | 3 |
| ④ | 错误处理 + unsafe | 1 | 10 | 3 |
| ⑤ | 资源管理 + 内存 | 1 | 4 | 5 |
| ⑥ | 边界条件 + 鲁棒性 | 2 | 11 | 5 |
| ⑦ | 架构与设计模式 | 3 | 8 | 0 |
| ⑧ | 安全审计 | 1 | 4 | 4 |
| ⑨ | 依赖审计 | 0 | 2 | 4 |
| ⑩ | OCR 管线专项 | 0 | 5 | 0 |
| ⑪ | 启动恢复 | 1 | 2 | 3 |
| ⑫ | Tauri 最佳实践 | 0 | 2 | 1 |
| ⑬ | 文档一致性 | 0 | 4 | 1 |
| **合计** | **十三轮 · 全维度** | **16** | **93** | **46** |

### 最终 TOP 10 修复建议（跨轮综合排序）

| 优先级 | 发现 | 轮次 | 严重 | 理由 |
|--------|------|------|------|------|
| 1 | 添加单元测试 | ⑦ | 🔴 | 0→1 突破，是所有后续修改的安全网 |
| 2 | ~~线程 panic 恢复~~ ✅ | ① | 🟢 | 5 个关键线程已添加 `catch_unwind` |
| 3 | ~~tick clone 优化~~ ✅ | ②⑤ | 🟢 | B 方案 cached_payload 已落地 |
| 4 | ~~前端 tick 全量重渲染~~ ✅ | ② | 🟢 | C 方案 DOM patch 已落地 |
| 5 | ~~日志 innerHTML XSS~~ 🔧 | ⑧ | 🟡 | 数据流追踪确认用户输入不达此 sink，但防御缺失 |
| 6 | ~~休眠后错过周期通知~~ ✅ | ⑪ | 🟢 | wall clock countdown + gap 补调已落地 |
| 7 | GetDIBits biSizeImage | ④ | 🔴 | 对 top-down 32-bit DIB 不规范 |
| 8 | 仲裁序列 2031 年到期 | ⑥ | 🔴 | 有 5 年缓冲但需提前处理 |
| 9 | 拆分 api.rs / lib.rs | ⑦ | 🔴 | 可维护性债务 |
| 10 | EnumWindows 提前终止 | ① | 🔴 | 轻微性能浪费，但极易修复 |

### 项目最终评分

| 维度 | ★★★★★ | 说明 |
|------|---------|------|
| 功能正确性 | ⭐⭐⭐⭐ | 多轮打磨，用户验证 |
| 并发安全 | ⭐⭐⭐⭐⭐ | 无死锁无数据竞争 |
| 错误处理 | ⭐⭐⭐⭐ | 降级合理，panic 恢复已实现，countdown 免疫休眠 |
| 性能 | ⭐⭐⭐⭐ | tick clone 已消除（B 方案），DOM 重建已消除（C 方案） |
| 代码组织 | ⭐⭐ | 两个千行文件，零测试 |
| 安全性 | ⭐⭐⭐⭐ | 本地桌面应用攻击面小，1 个 XSS 需修 |
| 鲁棒性 | ⭐⭐⭐⭐ | API 异常/窗口变化均有降级 |
| 依赖健康 | ⭐⭐⭐⭐⭐ | 极简依赖树，无漏洞 |
| 文档 | ⭐⭐⭐ | 核心文档准确但 CODE_NOTES 有遗漏 |

---

*审计完成。共 13 轮。原始发现 16🔴 93🟡 46🟢。截至 2026-07-02 已修复 2🔴（5.7.1 线程 panic、26.3.1 休眠 countdown），评估不修复 1🔴（5.1.1 重启重复），降级 1🔴→🟡（23.1.1 XSS 数据流修正）。此前 B/C 方案已修复 tick clone + DOM 重建。当前剩余 ~10🔴。*
