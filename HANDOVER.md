# VoxAlic 技术交接文档（AI 接手用）

> 这份文档写给**任何接手本项目的 AI 助手**。假设你对本项目一无所知。请先**完整读完本文件**，再读仓库根目录的 `CLAUDE.md`（架构细节）和 `CODE_NOTES.md`（函数级标注），然后才动手。
> 最后更新：2026-06-29，对应已发布版本 **v1.0.5**。

---

## 0. 一句话项目简介

**VoxAlic** 是一个 **Tauri v2 桌面应用**（仅 Windows），给《Warframe》玩家用。功能：监控游戏世界状态（昼夜循环、虚空裂缝、奸商 Baro、仲裁等）+ 一个**基于屏幕 OCR 的任务计时器**（识别游戏内倒计时，到 5 分钟节点提醒、维生系统低于阈值提醒）。界面**全中文**。

- 后端：**Rust**（`src-tauri/`）
- 前端：**纯 TypeScript + 原生 DOM**（`src/`），**没有 React/Vue/任何框架**，且**禁止引入框架**——保持零依赖风格。
- 用户是中文 Warframe 玩家，沟通用中文。

---

## 1. 最重要的硬规则（违反会出事）

| # | 规则 | 原因 |
|---|------|------|
| 1 | **绝不提交私钥** `warframe-monitor.key`（密码 `[REDACTED]`）。已在 `.gitignore`。 | 这是 Tauri 更新签名私钥，泄露=别人能伪造更新包。 |
| 2 | **不要手动 `git push gitee`**。Gitee 由 CI 全权 force-push（代码+tag+latest.json+release）。你只 push `origin`(GitHub)。 | 手动推会和 CI 打架，覆盖 CI 生成的 Gitee `latest.json`。 |
| 3 | **构建可分发 exe 必须用 `npx tauri build`**，不能用裸 `cargo build --release`。 | Tauri v2 用 `custom-protocol` cargo feature 区分 dev/prod 前端。裸 cargo 构建出的是 **dev 模式 exe**，运行时去连 `localhost:1420` → 用户看到"localhost 拒绝连接"。详见 CLAUDE.md「Building a distributable exe」。 |
| 4 | **改 `CLAUDE.md` 要同步 `AGENTS.md`**（后者是给 Codex 的镜像）；改功能要同步 `CODE_NOTES.md`。 | 多份文档要一致，否则下一个 AI 读到过期信息。 |
| 5 | **前端不准引入框架/构建依赖**。 | 项目刻意保持 vanilla TS。 |
| 6 | 提交信息风格见 §5。版本提交**只写版本号**（如 `v1.0.5`），不写描述。 | 用户明确要求。 |

---

## 2. 仓库与密钥地图

### 远端
- `origin` = GitHub：`https://github.com/SwingRainbow/VoxAlic.git`（主仓库，你 push 这里）
- `gitee` = Gitee：`https://gitee.com/Swing_Rainbow/vox-alic.git`（国内加速镜像，**只读，CI 维护**）
- 主分支：`master`

### GitHub Secrets（已配置在仓库 Settings → Secrets，CI 用）
| Secret 名 | 用途 |
|-----------|------|
| `TAURI_SIGNING_PRIVATE_KEY` | 更新包签名私钥内容 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 私钥密码（即 `[REDACTED]`） |
| `GITEE_TOKEN` | Gitee API token，用于推代码/建 release。当前值 `[REDACTED]`（用户选择**不重置**）。 |

### 本地私钥
- `warframe-monitor.key`（密码 `[REDACTED]`）——本地签名用，**已 gitignore，永不提交**。

---

## 3. 构建 / 运行 / 开发

构建环境（Windows）需要 MinGW WinLibs 在 PATH（`winget install BrechtSanders.WinLibs.POSIX.UCRT`），否则 `cargo build` 报 `dlltool: program not found`。Debug 构建会因 "export ordinal too large" 失败——**永远用 `--release`**。

```bash
npm run dev          # 仅 Vite 前端 dev server（端口 1420）
npm run tauri dev    # 完整 Tauri 开发（Rust + 前端热重载）
npm run tauri build  # 生产构建（产出 NSIS 安装包）—— 发布走这个
npm run build        # 仅 tsc 编译 + Vite 生产打包前端
```

- **没有** lint / test 命令。Rust 无 `#[test]`，前端无单测。`tsc` 在 build 流程里做类型检查。
- 开发版 vs 发行版二进制分工（见记忆 `feedback-dev-vs-release-binaries`）：
  - 桌面图标 = 安装的**发行版**。
  - `src-tauri/target/release/voxalic.exe` = **开发版**，平时改代码就在它上面用 `--no-bundle` 重建测试。
  - 如果只想快速验证代码、不出安装包：`npx tauri build --no-bundle`（仍带 custom-protocol，是真正的 prod exe，只是不打 NSIS 包）。

---

## 4. ⭐ 发布流程（核心操作，照着做）

发布是**自动化的**：你只需改 3 个版本号文件 + 写 changelog + 打 tag 推上去，**GitHub Actions 自动构建、签名、发 GitHub release、镜像并发 Gitee release**。

### 4.1 改版本号（必须 3 个文件一致）
版本号在 **3 个地方**，必须同步（`package.json` **不算**，它锁死在 `0.1.0`，是故意解耦的，别动它）：
1. `src-tauri/tauri.conf.json` → `"version": "x.y.z"`
2. `src-tauri/Cargo.toml` → `version = "x.y.z"`（在 `name = "voxalic"` 那段下）
3. `src-tauri/Cargo.lock` → 找 `name = "voxalic"` 的包，改它的 `version = "x.y.z"`

### 4.2 写更新日志（决定「检查更新」弹窗显示什么）
编辑根目录 **`CHANGELOG.md`**，在最上面加一节：
```markdown
## vX.Y.Z
- 这一版做了什么（每条一行，用户能看懂的中文）
- ...
```
**这是更新内容的唯一来源。** 链路是：
`CHANGELOG.md 的 ## vX.Y.Z 段` → CI「Extract release notes」步骤用正则抠出来写进 `release_notes.txt` → 写入 `latest.json` 的 `notes` 字段 → 更新插件读到变成 `update.body` → 后端 `check_for_update` 的 `notes` → 前端 `update-modal-notes` 弹窗显示。
- 正则在 `.github/workflows/release.yml` 的「Extract release notes」步骤：`(?ms)^##\s+vX.Y.Z\s*\r?\n(.*?)(?=^##\s+v|\z)`，即抠出该版本标题到下一个 `## v` 之前的内容。
- 抠不到就回退成单行 `VoxAlic vX.Y.Z`。**如果用户反馈弹窗只显示一行版本号，多半是这步正则没匹配上或编码问题——去看 Actions 里这步的日志。**
- `release_notes.txt` 是 CI 临时产物，已 gitignore，本地不会有。

### 4.3 提交 + 打 tag + 推
```bash
git add -A
git commit -m "vX.Y.Z"        # 版本提交只写版本号（见 §5）
git push origin master
git tag vX.Y.Z
git push origin vX.Y.Z         # ← 推 tag 触发 CI（.github/workflows/release.yml 监听 v* tag）
```
推 tag 后去 GitHub Actions 看 `Release` workflow 跑。它会自动：
1. `npx tauri build` 云端构建
2. 用私钥签名 NSIS 安装包（产出 `.sig`）
3. 生成 GitHub 版 `latest.json`（URL 指向 GitHub release）
4. 建 GitHub Release（附 exe + sig + latest.json，正文 = `release_notes.txt`）
5. 生成 Gitee 版 `latest.json`（URL 指向 Gitee），force-push 代码+tag 到 Gitee master，调 Gitee API 建 release 并上传 exe

> 更新检查是**双源**的：客户端先试 Gitee 的 `latest.json`（`https://gitee.com/Swing_Rainbow/vox-alic/raw/master/latest.json`），不行再用 GitHub。两个源的 `latest.json` 由 CI 分别生成，签名一致（同一份 CI 构建产物）。

### 4.4 ⭐ 发布后必做：归档安装包（每次都要！）
这是一条**固定规则**（记忆 `feedback-archive-release-installers`）：
- **时间点**：每次发版、CI 跑完后。
- **做什么**：检查 `src-tauri/target/release/bundle/nsis/` 里有没有**刚发布版本**和**上一个版本**的安装包，缺了就**从 GitHub release 下载原版**（不要本地重建，重建出的二进制/签名和用户拿到的不一致）：
  ```bash
  curl.exe -sL -o "src-tauri/target/release/bundle/nsis/VoxAlic_{ver}_x64-setup.exe" \
    "https://github.com/SwingRainbow/VoxAlic/releases/download/v{ver}/VoxAlic_{ver}_x64-setup.exe"
  ```
- 目标：该文件夹保留**所有历史发布版本**的安装包，供分发/回溯。
- 已归档（截至 2026-06-29）：1.0.0 / 1.0.1 / 1.0.1_dev / 1.0.2 / 1.0.3 / 1.0.4 / 1.0.5。

---

## 5. 提交规范
- **版本提交**：信息**只写版本号**，如 `v1.0.5`，不写任何描述（用户要求，记忆 `feedback-commit-style`）。
- **其他提交**：`type: 简短描述`（如 `ci: ...`、`docs: ...`、`fix: ...`）。
- 每条提交信息**结尾加**：
  ```
  Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
  ```
- ⚠️ 注意：你（接手的 AI）跑的工具可能是 **Bash 工具（POSIX sh）或 PowerShell**，二者语法不同。给 `git commit` 传多行信息时：Bash 用 heredoc 写到文件再 `-F file`；PowerShell 用 `@'...'@` here-string。**别把 PowerShell 的 `@'...'@` 喂给 Bash 工具**（会把 `@` 当成字面字符进提交信息——本项目踩过这个坑）。

---

## 6. 架构速览（细节见 CLAUDE.md）

### 前端 `src/`
- `src/main.ts` —— 整个 UI。通过 Tauri `invoke`（调命令）和 `listen`（收事件）驱动。Tab 布局：世界时间 / 虚空裂缝 / 任务计时 / 设置。
- `handleUpdate(payload)` 收 `AppStatePayload` 重渲染。
- `src/styles.css` —— 全部样式，暗色主题，CSS 变量在 `:root`。
- `index.html` —— 结构。更新弹窗的 `#update-modal-notes` 用 `white-space:pre-wrap` 已支持多行。

### Rust 后端 `src-tauri/src/`
| 模块 | 职责 |
|------|------|
| `lib.rs` | App 构建：共享状态、3 个线程、托盘、Tauri 命令、关闭到托盘；更新检查 `check_for_update` |
| `models.rs` | 所有 serde 结构体 |
| `state.rs` | `AppState`（`Arc<tokio RwLock>`） |
| `api.rs` | 抓 `api.warframe.com` worldstate JSON，解析裂缝/循环/节点/Baro/赏金/无限回廊，中文翻译表 |
| `config.rs` | `AppConfig`，持久化到 `{app_data_dir}/config.json` |
| `capture.rs` | Windows GDI 截屏（`PrintWindow`+`GetDIBits`），DPI 感知 |
| `ocr.rs` | 自研数字模板匹配（10 个 PNG 模板，NCC 相关 + NMS 去重）→ `"M:SS"` |
| `mission_timer.rs` | 计时器状态机 + 独立 OCR 轮询线程（mpsc 命令通道） |
| `window.rs` | Win32 窗口枚举/校验/层级 |
| `item_i18n.rs` | 物品名本地化（资产路径→简中），可热更新 |

### 线程模型
1. 主线程：Tauri 事件循环。
2. Tokio 异步任务：Tick 循环（每 1s）、Worldstate 抓取循环（每 1800s）。
3. 独立 `std::thread`：OCR 轮询（1–30s 可配）。跨边界协调只能走 `Arc<RwLock<AppState>>` + `mpsc`。

### 关键 Tauri 命令（前端 invoke）
`refresh_now`、`get_config`/`set_config`、`timer_command`、`list_windows`/`select_window`、`single_capture`、`capture_preview`、`test_recognize`、`test_alert`、`update_item_names`、`item_names_count`、`check_for_update`。

---

## 7. 当前状态与待办（2026-06-29）

- **已发布 v1.0.5**。本版亮点：订阅提醒（裂缝/昼夜/仲裁）从右下角系统通知改为**托盘红点闪烁+悬停弹窗**（后台挂机不易错过）；裂缝列表命中订阅条件的条目**金色描边+🔔**高亮；任务计时提醒仍保留系统通知；物品库显示游戏版本标注；魔胎之境赏金面板「声望」改为「母亲石印」。
- **待用户核对**：魔胎之境赏金面板的「母亲石印」**数量 N** 仍用 `xpAmounts` 求和，对代币而言量级可能不对——等用户游戏内确认。
- 后台挂机场景下屏幕 OCR 截图**不可用**（窗口被遮挡），所以那种场景优先用 **Toast/托盘** 提醒，不是 OCR（记忆 `background-afk-usage`）。

---

## 8. 记忆系统（你也应该读）

本项目在 `C:\Users\Administrator\.claude\projects\C--Users-Administrator-Desktop-tauri-warframe-monitor\memory\` 有持久记忆，索引在该目录的 `MEMORY.md`。接手后**先读 `MEMORY.md`**，里面有用户偏好、固定规则、项目现状的逐条指针。关键几条已在本文档 §1/§4.4/§5 体现。

规划文件在仓库根：`task_plan.md`（阶段）、`progress.md`（会话日志）、`findings.md`（研究发现）——想知道"为什么这么做"去翻这三个。

---

## 9. 给接手 AI 的最后叮嘱
1. 动手前先读 `CLAUDE.md` + `CODE_NOTES.md` + `MEMORY.md`。
2. 不确定就问用户，别瞎改（尤其发布、签名、Gitee 相关）。
3. 任何改动同步更新 `CODE_NOTES.md` 和（若涉及）`CLAUDE.md`/`AGENTS.md`。
4. 平台是 Windows，注意 Bash 工具与 PowerShell 的语法差异。
5. 用户沟通用中文。
