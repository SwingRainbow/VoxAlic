# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

> **Note:** `AGENTS.md` is a mirror of this file (for Codex.ai). Keep both in sync when making changes.

## Build / Run / Development

```bash
npm run dev          # Start Vite dev server (port 1420), then `npx tauri dev`
npm run build        # TypeScript compile + Vite production build
npm run tauri dev    # Full Tauri dev (Rust + frontend hot-reload)
npm run tauri build  # Production Tauri build (MSI/bundle)
```

**Build environment (Windows):** Requires MinGW WinLibs in `PATH`. Installed via WinGet:
`winget install BrechtSanders.WinLibs.POSIX.UCRT`
Path: `%LOCALAPPDATA%\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_...\mingw64\bin`
Without it, `cargo build` fails with `dlltool: program not found`. Debug builds fail with "export ordinal too large" — always use `--release`.

**Building a distributable exe — `custom-protocol` is mandatory.** Tauri v2 decides dev-vs-prod frontend via the `custom-protocol` cargo feature (`dev = cfg!(not(feature = "custom-protocol"))` in tauri-macros). A bare `cargo build --release` does NOT enable it, producing a *dev-mode* exe that loads `devUrl` (`http://localhost:1420`) at runtime → "localhost 拒绝连接" when no Vite server is up. Always build the shippable binary with `npx tauri build` (CLI adds the feature + runs `npm run build`), or, if invoking cargo directly, `cargo build --release --features custom-protocol`. The `[features] custom-protocol = ["tauri/custom-protocol"]` block in `src-tauri/Cargo.toml` must not be removed. Verify a build is prod by confirming the hashed `dist/assets/index-*.js` filenames are embedded in the exe (a dev exe lacks them); note the `localhost:1420` string is present in both because the full config is baked in.

There are no lint or test commands configured. The Rust side has no `#[test]` targets. The frontend uses `tsc` in the build pipeline but has no unit tests.

## Architecture Overview

**Tauri v2 desktop app** — Warframe worldstate monitor with OCR-based in-mission timer.

### Frontend (`src/`)
- **Vanilla TypeScript** single-page app — no React/Vue/shadcn. Avoid introducing frameworks; keep the zero-dependency approach.
- `src/main.ts` drives the entire UI via Tauri `invoke` (commands) and `listen` (events). Tab-based layout: 世界时间, 虚空裂缝, 任务计时, 设置.
- UI updates flow through `handleUpdate(payload)` which receives `AppStatePayload` and re-renders cycles, fissures, and timer.
- `src/styles.css` contains all UI styling — dark theme with CSS custom properties in `:root`.

### Rust Backend (`src-tauri/src/`)

**Module map with responsibility:**

| Module | Responsibility |
|--------|---------------|
| `lib.rs` | App builder: wires shared state, 3 threads, tray icon, Tauri commands, close-to-tray behavior |
| `models.rs` | All `Serialize`/`Deserialize` structs: `Fissure`, `CycleInfo`, `MissionTimerPayload`, `AppStatePayload`, `BaroItem`, `BaroInfo` |
| `state.rs` | `AppState` (fissures, cycles, countdown, baro) — `Arc<tokio::sync::RwLock<>>` |
| `api.rs` | Fetches Worldstate JSON from `api.warframe.com/cdn/worldState.php`, parses fissures, cycles (5 open-world zones), ~160 node lookup table, tier/type Chinese translations, Baro void trader parsing |
| `config.rs` | `AppConfig` / `MissionTimerConfig` with serde defaults, persisted to `{app_data_dir}/config.json` |
| `capture.rs` | Windows GDI screen capture via `PrintWindow` + `GetDIBits` — captures a region of a game window as BGR pixel buffer. DWM-aware for DPI correctness |
| `ocr.rs` | In-house digit template matching: 10 embedded PNG templates, NCC correlation + NMS dedup → `"M:SS"` string |
| `mission_timer.rs` | Timer state machine (`Idle→Running→Paused/Checkpoint`) + dedicated OCR polling thread with `mpsc` command channel |
| `window.rs` | Win32 window enumeration (`EnumWindows`), validation, z-order manipulation (`BringWindowToTop`), 16:9 frame stripping |
| `item_i18n.rs` | Item-name localization (asset-path → 简中). Hot-swappable `OnceLock<RwLock<HashMap>>`, embedded default + user-downloadable override |

### Thread Architecture

Three concurrent execution contexts:

1. **Main thread** — Tauri event loop, serving the webview, emitting events.
2. **Tokio async tasks** (spawned in `lib.rs` setup):
   - **Tick loop**: fires every 1s, decrements countdown, updates elapsed time, emits `tick-update`.
   - **Worldstate fetch loop**: fires every 1800s (30 min), calls Warframe API, emits `worldstate-update`. Also runs immediately at startup.
3. **Dedicated `std::thread`** (`mission_timer::start_timer_thread`): OCR polling loop at configurable interval (1–30s). Communicates via `mpsc::channel<TimerCommand>`. Log messages forwarded to frontend via `timer-log` events.

The Tokio async tasks and the `std::thread` OCR thread cannot call each other directly — all cross-boundary coordination goes through the shared `Arc<RwLock<AppState>>` and the `mpsc` channel.

### Key Commands (invoke from frontend)

- `refresh_now` — immediate worldstate fetch
- `get_config` / `set_config` — read/write persistent config
- `timer_command` — Start, Stop, Reset, SetMode for the mission timer
- `list_windows` — enumerate visible game windows matching the configured title
- `select_window` — save a selected window HWND to config
- `single_capture` — one-shot screen capture + OCR (doesn't modify timer state)
- `capture_preview` — returns the current game frame (frame-stripped to match OCR) as a `data:image/png;base64` URL for the calibration canvas
- `test_recognize` — runs OCR once on an explicit fractional ROI (the box the user just drew) without touching timer state or config
- `test_alert` — fires a sample reminder using the saved `alert_method` so the user can verify it (toast or window-focus)
- `update_item_names` — async: download WFCD i18n.json, rebuild the compact 简中 map, persist + hot-swap. Returns entry count
- `item_names_count` — current number of loaded item-name translations (for the 设置 → 物品库 status line)

### OCR System

Template matching using NCC (Normalized Cross-Correlation):
- 10 digit templates (0–9) embedded at compile time via `include_bytes!` from `src-tauri/resources/digit_templates/`
- **Multi-scale matching**: templates loaded at 1.0x and 0.85x; both scales are tried and the highest NCC score wins. Needed because 1728×1080/HUD-140% produces smaller glyphs than 2304×1440/HUD-130%.
- Captured ROI is binarized (threshold 160), then scanned with each template
- NMS (IoU 0.3) filters overlapping detections
- Detected digits sorted left-to-right, parsed as `M:SS` or `MM:SS`
- Match threshold: 0.70 (`MATCH_THRESHOLD` in `mission_timer.rs`)
- Life support uses HSV red-pixel density in a **separate ROI** (normal vs fissure mode use different ROIs — `life_support_roi` / `fissure_hp_roi`). Red hue: 0–15° or 345–360° (`.rem_euclid(6.0)` for correct wrap-around), S > 0.31, V > 0.47. Binary danger detector: red-pixel% > 1% = danger (`LIFE_SUPPORT_RED_THRESHOLD`). Frontend renders a traffic-light dot: gray (idle), green 正常 (running/safe), red with glow (danger). The `life_support_level` field in `MissionTimerPayload` carries `"normal"` or `"danger"`.

**OCR parity with Python original** (`C:\Users\TDD\Desktop\warframe_monitor\`): Template PNGs are byte-identical and the three historical pipeline-parameter gaps are now **resolved** in the Rust port (the P0/P1 fixes are landed, not pending):
1. **Match threshold**: 0.70, matching Python (was 0.65).
2. **NMS formula**: `inter / min(a, b)` in `ocr.rs::nms`, matching Python (was standard IoU `inter / (a + b - inter)`).
3. **Fissure timer ROI height**: default `h=0.030` (was 0.075).

### ROI Calibration (frontend)

The 任务计时 tab has a calibration panel (`setupCalibration()` in `main.ts`, `.calib-panel` in `index.html`). Flow: 截取画面 calls `capture_preview` → draws the exact OCR frame onto a `<canvas>` (backing-store size = capture pixels, so drawn boxes map 1:1) → user drags 时间框 / 维生系统框 over the image → 测试识别 calls `test_recognize` on the time box → 保存校准 writes the two boxes (converted to fractions) into the current mode's ROIs via `updateTimerConfig`. The active mode (普通 / 裂缝) is read from the `timer-mode` radio and decides which ROI pair (`normal_roi`+`life_support_roi` vs `fissure_roi`+`fissure_hp_roi`) is seeded and saved.

### Timer State Machine

Four states in `mission_timer.rs`: `Idle → Running ↔ Paused`, `Running → Checkpoint → Running`

- **Checkpoint** triggers when a valid OCR reading crosses a 5-minute bucket boundary (`CHECKPOINT_INTERVAL_SECS = 300`): `current_bucket > last_bucket` where `bucket = ocr_secs / 300`. The reached **milestone in minutes** is `current_bucket * 5`, surfaced in the log (`"⚠ 10分钟节点"`), the `status_text` (`"10分钟节点 — 请切回游戏"`), and the reminder body (via the `{min}` placeholder). Fires a reminder if `checkpoint_auto_focus` is enabled, and enters `Checkpoint` state. Resumes to `Running` automatically when a new (different) OCR value is detected.

### Reminders / Alerts

Two alert sources — the 5-minute checkpoint and the 维生≤20% HP alert — share one delivery mechanism, chosen by `mission_timer.alert_method` config (`"focus"` = force the game window to front via `bring_to_front()`, default; `"toast"` = Windows notification). The OCR thread is a bare `std::thread` with no `AppHandle`, so for toasts it sends an `AlertMsg { title, body }` (text already resolved, since the OCR thread has config access) over an `mpsc` channel (alongside the existing `log_tx`) to a forwarding thread in `lib.rs` that owns an `AppHandle` and calls `show_toast()` (via `tauri-plugin-notification`). `dispatch_alert()` in `mission_timer.rs` picks focus-vs-toast. Windows toasts only render reliably from an *installed* build (needs an AppUserModelID); a bare `tauri dev` exe may show nothing. Enable toggles: `checkpoint_auto_focus` gates the checkpoint reminder, `hp_alert_enabled` gates the HP reminder.
- **Custom reminder text**: `checkpoint_alert_text` / `hp_alert_text` config fields hold user-editable templates (set in 设置 tab). `render_alert_text()` substitutes `{min}` (checkpoint milestone) and falls back to the built-in default when the field is blank. `test_alert` previews the configured checkpoint wording with `{min}` → 5.
- **OCR acceptance rule**: `(-10..=30).contains(&(ocr_delta - wall_delta))` — OCR can run up to 30 s ahead of wall clock but not more than 10 s behind; or, with no prior baseline, `|OCR − wall_timer| ≤ 60 s`.
- **Rejection recovery**: after 3 consecutive rejects (`MAX_REJECT = 3`) the reading is force-accepted with log `"⟳ 重置基准"`.

### Cycle Calculation

The 5 open-world cycles use two different computation strategies:
- **Plains / Cambion / Zariman** — parsed from HexSyndicate/ZarimanSyndicate API JSON. Plains and Cambion share `HexSyndicate` (150 min rotation); Zariman uses `ZarimanSyndicate` (same 150 min period). Day/night is derived from the syndicate expiry: night = 50 min before bounty expiry.
- **Orb Vallis** — epoch-hardcoded (`1_541_837_628_000 ms` reference), 1600 s cycle (400 s warm + 1200 s cold); computed locally without API.
- **Duviri** — epoch-hardcoded, 7200 s per mood, 5-mood rotation: 悲伤 → 恐惧 → 喜悦 → 愤怒 → 嫉妒; computed locally without API.

#### Cycle Self-Healing (`roll_forward_cycle`)

When a syndicate-based cycle's phase end passes between 30-min API polls, `roll_forward_cycle()` in `api.rs` recomputes it locally so the UI never shows stale "切换中". The function is called from `build_payload()` in `lib.rs`:
- **Plains / Cambion**: reconstructs bounty expiry from current phase end, adds `HEX_CYCLE_MS` (150 min = 9,000,000 ms) until expiry is in the future, then rebuilds via `build_hex_cycle()`.
- **Zariman**: rolls the expiry forward in 150-min jumps, then recomputes the faction (Grineer/Corpus) from the rolled window's activation via `zariman_is_corpus()` (parity against the `ZARIMAN_CORPUS_ANCHOR_MS` known-Corpus anchor). The faction alternates every window and is fully locally derivable — `parse_zariman_cycle()` uses the same helper (the old `duration > 30min` heuristic was always true → always "Grineer").
- Vallis and Duviri are epoch-computed and don't need this — they're recalculated from scratch each tick.

### Baro Void Trader

`parse_void_trader()` in `api.rs` extracts `VoidTraders[0]` from the worldstate JSON. When Baro is at a relay (`start_ms <= now < end_ms`), his manifest of items is available; otherwise only the arrival countdown is shown.

**Item naming**: Baro's manifest uses StoreItems asset paths (`/Lotus/StoreItems/.../QuantaVandal`). `parse_void_trader()` first tries `item_i18n::translate(path)` for a 简中 name (see Item-Name Localization below); when that misses, `name_from_path()` derives a readable English name by taking the last path segment and splitting CamelCase (e.g. `QuantaVandal` → `Quanta Vandal`).

**Frontend rendering** (`renderBaro()` in `main.ts`):
- Active state: scrollable table with item name, ducat price (`PrimePrice`), and credit price (`RegularPrice`), labeled as 杜卡德 / 星币.
- Waiting state: shows location and arrival countdown with a "尚未到达" placeholder.
- **Scroll preservation**: the panel uses a structural signature (`baroSig = active|location|itemCount`). When only the countdown changes (same sig), only the countdown text node is patched — the货物 table DOM is not rebuilt, preventing the scrollbar from snapping to the top on every tick.

**Countdown logic** in `build_payload()` (`lib.rs`): if Baro hasn't arrived yet but `now >= start_ms`, the `active` flag is flipped to `true` locally. The displayed countdown target is `end_ms` when active, `start_ms` when waiting.

### Item-Name Localization (`item_i18n.rs`)

Translates Warframe asset paths (Baro manifest `ItemType`, bounty reward names) to 简中.
- **Source of truth**: WFCD/warframe-items `data/json/i18n.json` (~51 MB, 14 languages, keyed by `uniqueName`; note it has **no `en`** key — English is derived elsewhere). We ship a compact `uniqueName → 简中` map at `src-tauri/resources/baro_zh.json` (~16k entries, embedded via `include_str!`) so translation works offline on first run.
- **Hot-swap design**: `static MAP: OnceLock<RwLock<HashMap<String,String>>>`. `init(app_data_dir)` loads the user's downloaded override (`{app_data_dir}/baro_zh.json`) if present, else the embedded default. `update_from_remote()` (invoked by `update_item_names`) downloads the full i18n.json, parses **only** each entry's `zh.name`, writes the compact override, and swaps the in-memory map — no restart needed.
- **Lookup** (`translate`): tries the path verbatim, then with the `/StoreItems/` segment stripped (Baro paths are `/Lotus/StoreItems/...` but i18n keys omit `StoreItems`). Falls back to `name_from_path()` (CamelCase split) when unknown.
- Surfaced in 设置 → **物品库** with a 检查更新 button + entry-count status.

### Bounty Panels

Click any open-world cycle card (世界时间 tab) → inline `#bounty-panel` expands with that location's active bounty board.

All 6 syndicates are live in `BOUNTY_SOURCES` (`api.rs:1057`):

| Tag | 地点 | Jobs source |
|-----|------|-------------|
| `CetusSyndicate` | 夜灵平野 | live `Jobs` array |
| `SolarisSyndicate` | 奥布山谷 | live `Jobs` array |
| `EntratiSyndicate` | 魔胎之境 | live `Jobs` array |
| `ZarimanSyndicate` | 扎里曼 | synthesized (Jobs always empty in API) |
| `HexSyndicate` | 霍瓦尼亚/六人组 | synthesized (Jobs always empty in API) |
| `EntratiLabSyndicate` | 解剖圣所 | synthesized, **card = 魔胎之境** (shares Cambion Drift in-game) |

Synthesized syndicates (Zariman / Hex / 解剖圣所) have their jobs generated locally in `parse_bounties()` because the worldstate `Jobs` array is always empty for them.

- **Titles** (`bounty_title()`): maps the `jobType` last path segment to a Chinese narrative title (e.g. `RescueBountyResc` → 搜索并救援), with a keyword fallback for unmapped types. Narmer bounties (`/Narmer/` in path) use their own titles + suffix （合一众）; `min_level >= 100` appends （钢铁之路）.
- **Reward pools** (`reward_rotations()`): three rotations A/B/C per level range, pre-translated and embedded in per-syndicate JSON files under `src-tauri/resources/` (e.g. `cetus_bounty_rewards.json`, `solaris_bounty_rewards.json`, `hex_bounty_rewards.json`, etc.; shape `{"min-max": {"A":[{name,rarity,chance}], "B":…, "C":…}}`). Each pool is sorted by rarity (Common→Legendary) then descending chance. **These snapshots are NOT refreshed by 检查更新** — only item names are.
- **Active rotation** (`active_rotation_of()`): the live worldstate `rewards` field encodes the board-wide active rotation as the letter after `Table` in `Tier{A-E}Table{A/B/C}Rewards`. It advances each 150-min bounty refresh (= one day/night cycle). All jobs on a board share one active rotation.
- **Frontend** (`renderBountyPanel()` in `main.ts`): orange-branded header; A/B/C rotation buttons where only the active one is enabled (labeled （当前）) and the other two are `.locked` 🔒 (kept visible but disabled — by design). Per-bounty blocks show num/title/等级; rewards render as a 4-col zebra grid colored by rarity (`rarityCls()`), chance shown on hover.

### Duviri Circuit Panel (无限回廊)

Click the 双衍王境 cycle card → `#circuit-panel` expands with the current week's Circuit rotation.
- **Parsing** (`parse_circuit()` in `api.rs`): reads `EndlessXpChoices` from worldstate. `normal` = Warframe names (英文，DE doesn't localize them consistently); `hard` = Incarnon Genesis weapon names in 简中 via `item_i18n`. Weekly reset on Monday.
- **Resource**: `src-tauri/resources/circuit_names.json` maps Incarnon weapon asset paths to 简中 names (static, maintained manually).
- **Frontend** (`renderCircuitPanel()` in `main.ts`): shows 普通回廊 Warframe list + 钢铁之路 Incarnon weapon list with expiry countdown.

### Window Capture Stack (Windows-only)

`PrintWindow` (GDI) with `PW_RENDERFULLCONTENT` fallback, pinned to `src-tauri/Cargo.toml` dependency on `windows = "0.58"`. Uses DWM frame bounds for true window dimensions (DPI-aware). ROI coordinates are stored as fractions of window size (0.0–1.0).

`capture.rs` pipeline: `capture_full` (whole-window BGR buffer) → `strip_frame` (remove 16:9 letterbox **only if** aspect ratio ≥ 16:9 × 0.95; skipped for 16:10 windows to avoid miscroping) → `crop_roi` (fraction → pixel slice). Black-frame detection (`is_black_frame`: non-black pixels ≤ 1%) returns `None`; 5 consecutive failures trigger `resolve_hwnd` rescan (`CAPTURE_FAIL_RESCAN = 5`).

### Configuration Persistence

Config lives at `{app_data_dir}/config.json` (Tauri-managed app data dir). Default config created on first run. `close_to_tray` defaults to `true`. OCR defaults: 2s interval, strip frame enabled, checkpoint auto-focus enabled, HP alert enabled. Reminder text defaults: `checkpoint_alert_text = "⚠ 到达 {min} 分钟节点 — 请切回游戏"`, `hp_alert_text = "🚨 维生系统 ≤ 20% — 请补充维生胶囊"` (blank values fall back to these at runtime).

Default ROI fractions (as proportion of window size; fissure timer height lowered to 0.030 in stage 26):
- Normal timer: `(x=0.005, y=0.415, w=0.07, h=0.03)`
- Fissure timer: `(x=0.005, y=0.46, w=0.07, h=0.030)`
- Normal life support: `(x=0.035, y=0.300, w=0.095, h=0.050)`
- Fissure life support (`fissure_hp_roi`): `(x=0.035, y=0.385, w=0.095, h=0.050)`

Missing config fields use serde defaults. `load_config` calls `migrate_old_default_rois()` to silently upgrade ROIs that still match old defaults; user-customised values are preserved. No config versioning beyond this migration pattern.

### I18N

All user-facing strings are Chinese (zh-CN). The UI is Chinese-only. The Warframe API returns English keys which are translated via lookup tables in `api.rs` (mission types, tier names, node names, bounty titles). **Item names** (Baro manifest, bounty rewards) are localized differently — via the data-driven, updatable `item_i18n` map rather than hardcoded tables (see Item-Name Localization).

### API Dependency Snapshot

`API_SNAPSHOT.md` records every JSON path, field type, enum value, and fallback the code depends on from DE's worldstate API. **Before each commit, verify the API hasn't changed** — if it has, update the snapshot. This is the single source of truth for what the code expects from `api.warframe.com/cdn/worldState.php`.
