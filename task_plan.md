# 任务计划 — Tauri Warframe Monitor

## 目标
用 Tauri v2 (Rust + TypeScript) 从零构建 Warframe 裂缝监控桌面应用，替代 Python/Tkinter 原版。

## 阶段

| 阶段 | 状态 | 说明 |
|------|------|------|
| 0. 环境准备 | ✅ complete | 安装 Rust + MinGW + Tauri 脚手架 |
| 1. 数据模型 | ✅ complete | models.rs — Fissure, CycleInfo, AppStatePayload |
| 2. API 层 | ✅ complete | api.rs — HTTP 请求 + JSON 解析 + 200+ 节点查找 |
| 3. 状态管理 | ✅ complete | state.rs — Arc<RwLock<AppState>> |
| 4. 托盘 + 循环 | ✅ complete | lib.rs — 后台 fetch/tick + 系统托盘 + 关闭隐藏 |
| 5. 前端 UI | ✅ complete | index.html + styles.css + main.ts |
| 6. 构建发布 | ✅ complete | MSI (7MB) + NSIS (4MB) + exe (23MB) |
| 7. Bug 修复 | ✅ complete | 世界时间不动 + 筛选计数 + 数据映射 |
| 8. 设置功能 | ✅ complete | 设置 tab + 关闭行为可配置 |
| 9. 任务计时 | ✅ complete | OCR 屏幕捕获 + 模板匹配 + 维生检测 + 5min 截点提醒 |
| 10. 任务计时完善 | ✅ complete | 窗口选择、日志、单次截图、双时间、截点倒计时、识别率、弹窗开关 |
| 11. 自检修复 | ✅ complete | 配置覆盖修复、ocr_raw 字段、HP 告警去重、mode radio 竞态、epoch 周期自动刷新、窗口自动检测 |
| 12. 世界时间 + 窗口修复 | ✅ complete | Plains/Cambion night_start 50min、Duviri 去 epoch、Cambion Fass/Vome、PID 过滤、清除 stale HWND |
| 13. FIX_GUIDE 修复 | ✅ complete | ROI坐标、连续拒绝基准重置、时间硬同步、sleep短轮询、维生布尔判断、NMS实际宽高、浮点容差、匹配阈值0.65、代码优化合并 |
| 14. 截图流修复 | ✅ complete | capture 重构（先抓全窗→剥帧→裁 ROI，修复顺序 bug）/黑帧检测/连续失败重扫 — commit 2e88f78，构建+冷启动通过 |
| 15. 新机器环境搭建 | ✅ complete | Rust 1.96.0 GNU + MinGW WinLibs 16.1.0 + 全量构建通过 + exe 冷启动正常 |
| 16. localhost 拒绝连接修复 | ✅ complete | 根因：裸 `cargo build` 不启用 `custom-protocol` feature → 编出 dev 模式 exe 去连 localhost:1420。修复：Cargo.toml 补回 feature 段 + 用 `--features custom-protocol` 重构。二进制验证资源已嵌入、运行时不连 1420 |
| 17. 全量代码审查+优化 | ✅ complete | 通读全部源码 + clippy。修复截点检测逻辑 bug（value_since 字段）、lib.rs 抓取逻辑去重(×4→1)、清除 4 个 clippy lint。评估见 `code-review.md`。⚠ 截点改动需游戏内验证 |
| 18. 多分辨率 ROI/OCR 校准 | ✅ complete | Codex 完成代码侧校准：更新普通/裂缝 timer 与维生默认 ROI、OCR 加入 0.85x 多尺度模板、16:10 窗口跳过 16:9 剥帧。仍缺 2304×1440 普通模式截图与游戏内 `single_capture` 回归 |

## 阶段 18 任务书 — 多分辨率 ROI/OCR 校准（交给 Codex）

### 背景（已由截图分析得出，不要重复推导）

`pic/` 下有 3 张可用游戏截图（1728×1080 普通、1728×1080 裂缝、2304×1440 裂缝），命名规则：`分辨率 时间 维生% [裂缝]`，无"裂缝"后缀即普通模式。已按整窗高度比例测量左侧"生存/维生系统/时间"面板各元素位置：

| 元素（整窗高度分数） | 1728×1080 普通 | 1728×1080 裂缝 | 2304×1440 裂缝 |
|------|------|------|------|
| 维生系统 % | ~0.31 | ~0.40 | ~0.39 |
| 时间数字 | ~0.42–0.44 | ~0.475–0.505 | ~0.48–0.505 |

**已确认的事实：**
1. 两个分辨率都是 **16:10**（1728/1080 = 2304/1440 = 1.6），不是 16:9。
2. **两个分辨率在裂缝模式下落点几乎相同**（时间 ~0.48–0.50、维生 ~0.39）。这是巧合：16:10 同比 + HUD 缩放反向抵消（1728 用 140%、2304 用 130%）。改任一台 HUD 缩放即失效。
3. **真正使位置变化的是「普通 vs 裂缝」模式**，不是分辨率。裂缝模式多一行"收集到的反应物 X/10"把维生/时间整列下推（维生 +0.09、时间 +0.05）。项目分两套 ROI（普通/裂缝）正是为此，设计正确。
4. **隐患：绝对像素字号不同**。占比接近但 2304×1440 数字像素更大（行高 1440 vs 1080）。OCR 用固定尺寸数字模板 + NCC(阈值 0.65)，跨分辨率可能匹配失败。
5. **数据缺口：没有 2304×1440 普通模式截图**，无法校准 2304 普通 ROI。

### 当前默认 ROI（`src-tauri/src/config.rs`，窗口尺寸分数）
- 普通 timer：`x=0.005, y=0.395, w=0.06, h=0.025`
- 裂缝 timer：`x=0.005, y=0.465, w=0.06, h=0.025`
- 维生：`x=0.04, y=0.305, w=0.08, h=0.04`

### 任务（按优先级）

**T1 — ROI 分数校准（必做，低风险）**
- 普通 timer：实测时间数字在 ~0.42–0.44，当前 `y=0.395, h=0.025`（罩 0.395–0.420）**偏高**，只盖到"时间:"标签和数字顶部。建议下移到约 `y=0.415, h=0.03`，并验证宽度 `w=0.06`（x=0.005）是否盖全 `MM:SS`（"01:01"右侧可能被裁，考虑 w 加到 0.07）。
- 裂缝 timer：实测 ~0.475–0.505，当前 `y=0.465, h=0.025` 基本可用，可微调到 `y=0.475, h=0.03`。
- 维生 ROI：普通模式实测 ~0.31、裂缝模式 ~0.39——**两种模式维生位置也不同**。当前只有单一维生 ROI `y=0.305`（只对普通模式准）。需决策：是否让维生 ROI 也随模式切换（普通/裂缝两套），或扩大 h 同时覆盖。建议与 timer 一样按模式分两套。
- 用 `pic/` 三张图逐一核对：可参考我用过的方法——`System.Drawing` 裁剪 + 叠加比例网格（脚本思路见 progress 本节）。改完后用 `single_capture` 在真实游戏上回归。

**T2 — 评估固定尺寸模板的跨分辨率失配（重要，改动较大，先评估再动手）**
- 量出三张图里单个数字的实际像素宽高，对比 `src-tauri/resources/digit_templates/*.png` 模板尺寸。
- 若差异显著（经验上 >±15% 即影响 NCC），方案任选其一并在 findings.md 记录权衡：
  - (a) 多尺度模板匹配：对模板做若干缩放档位后取最高分；
  - (b) 按分辨率/HUD 缩放分组多套模板；
  - (c) 捕获 ROI 后归一化缩放到模板基准尺寸再匹配。
- **不要在没量数据前直接重做模板。** 先确认是否真失配。

**T3 — 补数据（需用户配合，非阻塞）**
- 请用户补一张 **2304×1440 普通模式生存**截图放入 `pic/`，命名遵循现有规则。拿到后校准 2304 普通 ROI。

### 验收标准
- `pic/` 三张图上，校准后的 ROI 框（可视化叠加）准确罩住时间数字与维生条。
- `cargo build --release --features custom-protocol` exit 0、零警告（构建须带 WinLibs PATH，见 findings.md）。
- 改动写入 `findings.md`（根因+权衡）、`progress.md`（本次会话）、`task_plan.md`（阶段 18 标 complete）。
- T2 若结论是"需改模板/匹配逻辑"，在游戏机上用 `single_capture` 实测两种分辨率识别成功率后再合入。

### 约束与陷阱（务必遵守）
- ROI 一律存**窗口尺寸分数 0.0–1.0**，不要写死像素。
- 16:9 剥帧逻辑（`window.rs` strip_frame）只对 16:9 窗口生效；本项目两个分辨率都是 **16:10**，确认剥帧不会误裁——若 strip_frame 假设 16:9，需评估是否对 16:10 产生偏移（潜在 bug，一并核查）。
- 构建只能 `--release`（debug 报 ordinal 过大）；分发用 MSI/NSIS，不直接复制 exe。
- 不要引入前端框架，保持零依赖 vanilla TS。

---

## 阶段 18 执行结果 — Codex

- ROI 默认值：普通 timer 调整为 `x=0.005, y=0.415, w=0.07, h=0.03`；裂缝 timer 调整为 `x=0.005, y=0.46, w=0.07, h=0.075`；普通/裂缝维生 ROI 分离为 `y=0.300` 与 `y=0.385`，并统一扩大到 `x=0.035, w=0.095, h=0.050`。
- 配置迁移：读取旧 `config.json` 时，仅将仍等于旧默认值的 ROI 自动迁移到新默认，保留用户手动改过的 ROI。
- OCR 模板：固定 1.0x 模板在 2304×1440 裂缝截图可识别，但在 1728×1080/HUD 140% 样本上明显漏识别；采用低侵入方案 (a) 多尺度模板匹配，额外加载 `0.85x` 模板，保留原模板以覆盖 2304×1440 样本。
- 剥帧逻辑：`window::strip_frame` 新增宽高比保护，当前窗口宽高比低于目标 16:9 的 95% 时不裁切，避免把 16:10 截图误裁成 16:9 内容。
- 可视化核对图：生成在 `pic/_codex_roi_analysis/roi_overlay_contact_sheet.png`，用于查看旧/新 ROI 覆盖位置。
- 验证：`npm run build` 通过；`cargo build --release --features custom-protocol` 通过。尚未做真实游戏内 `single_capture` 回归，仍缺 2304×1440 普通模式截图。

| 19. 维生 UI 简化 | ✅ complete | 移除进度条，改为红绿灯圆点指示器。后端不变，仅前端三文件（index.html / styles.css / main.ts）— commit b2c1b44 |
| 20. ROI 框选校准工具 | ✅ complete（验证通过 + 已提交 a2b5fc7） | 在已捕获的整窗截图上手动框选时间/维生区，每局可重校。后端 `capture_preview` + `test_recognize`；前端校准面板(canvas 拖框+测试+保存)。**用户实测：框选识别非常成功**。同会话修复截点恢复未更新基准的 bug |
| 21. 提醒方式二选一 + 测试按钮 | ✅ complete（已提交 a2b5fc7） | 新增 `alert_method`（focus 强制弹窗 / toast Windows通知），节点+维生共用；引入 `tauri-plugin-notification`，OCR 线程经 `mpsc<AlertKind>` 转发 toast 给持 AppHandle 的线程；`test_alert` 命令 + 设置 tab 测试按钮。维生提醒从 `checkpoint_auto_focus` 解耦 |
| 22. 任务计时 tab UI 重排 | ✅ complete（已提交 a2b5fc7） | 消除臃肿：Hero 卡片(时间+维生灯+按钮) + 紧凑配置条 + 校准面板改可折叠 `<details>`，模式单选移入校准面板。纯前端 + main.ts 删 `#calib-mode` |
| 23. 全量代码审查 + 修复 + 优化 | ✅ complete（已提交 a2b5fc7） | 用 CodeGraph 辅助通读全部源码。修 7 处：div_ceil(clippy)、命令处理去重(helper)、deadline 等待丢 SingleCapture、维生空捕获误判危险、ocr 多余 clone、前端筛选去重、去 `as any`。clippy 零警告，构建通过。详见 findings.md 审查表 |
| 24. 文档补全 + Baro 倒计时中文格式 | ✅ complete（已提交 e616711） | CLAUDE.md 补 Baro/周期自愈章节 + 同步 AGENTS.md；Baro 倒计时改中文含天/小时/分钟/秒 |
| 25. Python vs Rust OCR 逐项对比 | ✅ complete（已提交 52cebbe） | 逐文件对比 Python 原版 OCR 与 Rust 移植，定位 7 处差异，给出 P0-P3 修复优先级 |
| 26. OCR 管线参数对齐 Python（P0+P1+P2） | ✅ complete（已提交 0a59068） | 匹配阈值 0.65→0.70；NMS 公式改 `inter/min(area_a,area_b)`；裂缝 ROI 默认高 0.075→0.030 + 迁移；HSV 阈值 S>0.4,V>0.3→S>0.31,V>0.47 |
| 27. 节点提醒动态分钟数 + 自定义提示词 + 设置页整理 | ✅ complete（已提交 edc3ed5） | 用户实测 OCR 效果不错。节点提醒由固定"5分钟节点"改为按里程碑显示（5/10/15…），日志/状态文本/提醒体全部体现；新增 `checkpoint_alert_text`/`hp_alert_text` 配置，`{min}` 占位符替换，留空回退默认。UI 迭代两轮定为**弹窗编辑**（提醒方式行加「自定义提示词…」按钮 → 居中浮层）；「关闭窗口行为」改名「关闭模式」，删除灰色说明小字 |
| 28. UI/布局优化（精致紧凑） | ✅ complete（待提交，纯 CSS） | 用户选范围=世界时间/设置/全局，方向=精致紧凑。全局：扩 `:root` 变量(`--surface-2`/`--radius`/`--border-soft`/`--accent-dim`)、细滚动条、字体平滑、Tab 栏加底色与 hover。世界时间：周期卡 flex→grid 等宽、字号层次重排、时间等宽字体；Baro 面板紫色左条 + 地点 chip + 倒计时等宽。设置：`.settings-group` 收成居中 680px 卡片、item 内分隔线。状态栏加底色 + 刷新按钮 hover 高亮。**未改 DOM / main.ts / Rust**。⚠ 一轮反复：周期卡初版去色底只留细彩条被用户否（"不如之前"）→ 回滚为 day/night **整块渐变色底**+同色边框+hover 微光，保留网格/字号改进。共 5 次构建 |
| —（调查，无代码改动） | ✅ | 裂缝字段审计：worldState 裂缝条目没用到的仅 `_id`/`Region`/`Seed`，官方无阵营字段（findings 续4）。Baro 地点调查：`VoidTraders[0].Node` 是 API 提前给的下一站（SaturnHUB→Kronia Relay/土星），非残留（findings 续5）。可选后续：裂缝加阵营列 |
| 30. 夜灵平野赏金面板（点卡片内嵌弹出） | ✅ complete（已并入提交 6800fb4） | 初版：表格列（类型/等级/MR/阶段/声望），无奖励物品 |
| 35. 扎里曼赏金按阵营 + 任务描述（进行中） | 🔄 in_progress | 用户给 `坚守者G.png`/`坚守者C.png`，要求加"节点-目标"描述（如"奥金工场-作为指挥官击杀10名敌人"），API 无则自建表（findings 续19）。**规律**：节点↔任务类型固定（翠径=移动防御/涂沃主厅=虚空覆涌/奥金工场=虚空决战/永视弧域=虚空洪流/哈拉科防线=歼灭）；阵营决定"节点落哪个等级槽"的排序 + 目标文字。奖励两阵营相同。**✅ 阵营 epoch 部分已落地（阶段 36，commit f3b2439）**：`ZARIMAN_CORPUS_ANCHOR_MS=1_780_384_080_000` + `zariman_is_corpus(activation)`，`parse_zariman_cycle`/`roll_forward_cycle` 已用（周期卡阵营现正确）。**剩余（未做）**：`synthesize_zariman_jobs(faction)` 按阵营排序+标题+描述 → 填充 `BountyJob.desc`（字段已加，现填空串）→ 前端 `BountyJob` 接口加 `desc` 并渲染。⚠ 目标文字仅 2 样本，疑似逐周期变 → best-effort，节点+任务类型稳定 |
| 36. 深读代码审查 + Bug 修复 + 优化 | ✅ complete（已提交 f3b2439） | 用户："深度阅读项目代码，评估并检查bug再优化代码"。通读全部 11 个 Rust 模块 + main.ts(786) + clippy(零警告) + tsc(--noEmit 通过)。**修 3 项**：①编译破损 `static_bounty_job` 漏补 `BountyJob.desc`(E0063)；②**扎里曼周期卡阵营恒 Grineer**（`parse_zariman_cycle` 用 `duration>30min` 判断恒真，窗口恒 150min）→ epoch 奇偶推算（见阶段 35），锚点经 warframestat `/pc/zarimanCycle` 实测验证（当前 Corpus）；③启动重复抓取（lib.rs 手动 initial + interval 首 tick 立即返回 → 连抓两次）删手动 initial。**文档**：CLAUDE/AGENTS OCR 段 stale 更新（阈值 0.70、HSV 0.31/0.47、三项 OCR gap 已 resolved）+ Zariman roll_forward 阵营可本地推算。**审查为干净**：clippy 零警告/锁顺序无死锁/timer 状态机/截点桶/OCR 接受规则正确。**未改（建议）**：前端 `renderBountyPanel` 每 tick 全量重建 innerHTML（滚动重置，可仿 baroSig 优化）；innerHTML 未转义（可信源，低风险） |
| 37. 魔胎之境赏金接入（阶段 34 子项） | ✅ complete（待提交） | 用户给 `英择谛.png` + "补充轮次表"。生成 `resources/deimos_bounty_rewards.json`（`_gen_deimos.py`，12 键 6 常规+3隔离库`v`+3奥秘`av`，各 A/B/C，0 漏翻；修嵌套数量 `3X 1,500 Credits Cache`→`1,500 现金匣 ×3`）。api.rs：`BOUNTY_SOURCES`+`("EntratiSyndicate","魔胎之境")`、`bounty_title` Entrati 分支（6 jobType 标题+空=隔离库+lvl100 钢铁之路）、`deimos_rewards()`/`deimos_rotations()`（vault `v`/`av` 前缀消歧 + 按 job 自身 Table 字母选**单池**）、`parse_bounty_job` 分流、抽 `sort_pool()`。main.ts `SYNDICATE_DISPLAY['魔胎之境']='英择谛'`。**关键**：英择谛.png 确认每赏金单池（非 A/B/C 选择器）→ 复用 `singleRot`。tsc 通过 / clippy 零警告 / `npx tauri build` MSI+NSIS 成功。待游戏内核对面板 |
| 38. 双衍王境无限回廊（Circuit）面板 | ✅ complete（构建通过，待提交） | 落地：① `_gen_circuit.py`→`circuit_names.json`(767 token→中文)；② 后端 `CircuitInfo` + `parse_circuit`(EndlessXpChoices/Schedule) + formatters DRY(`fmt_dhms`/`fmt_remain_days`)；③ 前端 `renderCircuitPanel`（双衍卡点开，普通回廊战甲/钢铁回廊武器 chips + 周一刷新倒计时，互斥单开）。tsc/clippy 零警告/build 成功。待游戏内核对。 **原 todo**：| 用户："无尽回廊这个是需要做的"。**数据源已确认在 worldState**（findings 续26）：`EndlessXpChoices`（当前选项）+ `EndlessXpSchedule`（Activation/Expiry，每周一刷新）。两类：`EXC_NORMAL`=普通回廊·战甲（3 个）、`EXC_HARD`=钢铁回廊·Incarnon 武器（5 个）。**翻译复用现有表**（findings 续27，无需手翻）：token →CamelCase 还原英文名→`_all_tmp.json`→uniqueName→`baro_zh.json`→中文；战甲名落英文（国服不翻译战甲，正确）。落地步骤：① 生成脚本（仿 `_gen_*.py`）遍历全部战甲+Incarnon 武器产出 `circuit_names.json`(token→中文) 嵌入；② 后端解析 `EndlessXpChoices`/`EndlessXpSchedule` → 新 payload 结构（普通/钢铁两组 + expiry）；③ 前端：点双衍王境周期卡展开面板（仿赏金面板内嵌），显示本周战甲/武器 + 周一刷新倒计时。⚠ token 未命中映射时回退英文（可接受）。顶层另有 `Conquests`/`Descents` 未纳入 |
| 39. 霍瓦尼亚/1999/六人组赏金（阶段 34 子项） | ✅ complete（已提交 d5a64e0→b8e1f38→f582312） | 用户："先做出来给我看，等会发机器人图"。HexSyndicate Jobs 恒空（findings 续29）→ 仿扎里曼静态合成。`_gen_hex.py`→`hex_bounty_rewards.json`（7 档单池 C，0 漏翻；夜语者77/侦察者/Cyte-09 部件均解析）。api.rs：BOUNTY_SOURCES+`("HexSyndicate","霍瓦尼亚")`、`hex_rewards()`、`synthesize_hex_jobs()`(7档)、parse_bounties Hex 分支、bounty_title Hex 分支、`parse_hex_cycle()` 加霍瓦尼亚周期卡（无昼夜，仅赏金刷新窗口 🌃）、roll_forward 霍瓦尼亚分支。main.ts SYNDICATE_DISPLAY 霍瓦尼亚→六人组。**机器人图 六人组.png 核对（新旧两周期对比，findings 续31+续33）**：① **等级档位修正**（b8e1f38）`55-60…115-120`→真实 `65-75,75-80,85-90,95-100,105-110,115-120,125-130`（原猜测整体偏低一档；奖励池内容/顺序本就对→仅重打键），两周期图都确认 → **保留**；② **标题不可写死**（f582312 改回通用）：新旧两图同一等级档标题不同 → 任务类型每 150min 刷新轮换，worldState 空 Jobs 拿不到当前周期 → `bounty_title` Hex 分支保持通用「六人组赏金」（曾在 b8e1f38 按 level match，已撤）；目标描述同样轮换→desc 留空。clippy 零警告/build MSI+NSIS 成功。⚠ 同隐患波及扎里曼标题（按 level 写死，单图验证），待不同周期图复核 |
| 40. 解剖圣所赏金（阶段 34 子项，挂魔胎卡） | ✅ complete（构建通过待提交，待游戏核对） | EntratiLabSyndicate 空 Jobs（Seed 29454）→ 静态合成单池（WFCD `entratiLabRewards` 仅 rotation C，5 档 55-60/65-70/75-80/95-100/115-120 跳档）。`_gen_entrati_lab.py`→`entrati_lab_bounty_rewards.json` 0 漏翻。**挂卡设计**：BountyInfo 加 `card` 字段（解剖圣所→魔胎之境），前端 `withBounty`/面板按 card 过滤 + renderBountyPanel 改**多 section**（魔胎卡=英择谛+解剖圣所两段堆叠）。声望=音魂货币→standing 0 不显示；标题种子轮换→通用「解剖圣所赏金」。详见 findings 续40/续42。⚠ 待核：等级档是否需 +10、两段观感 |
| 34. 其余赏金地点（待办） | 🔄 部分（魔胎37 + 1999=阶段39 + 解剖圣所=阶段40 完成） | 用户："赏金还有很多，不止殁世幽都，还有霍瓦尼亚中央商场-1999、解剖圣所等"。**魔胎之境（阶段37）+ 1999/霍瓦尼亚（阶段39）已落地**。剩余：解剖圣所(EntratiLab jobs=0，需静态合成，但暂无该地奖励表/无周期卡)。同构扩展（加 `BOUNTY_SOURCES` + `SYNDICATE_DISPLAY` + 生成奖励表 + `rewards_for` 分支 + 必要的标题映射）。**已确认数据源**（2026-06-02 实测）：① **殁世幽都 / 魔胎之境**（Cambion Drift）= `EntratiSyndicate`，**现有 9 个 live jobs**（`DeimosMissionRewards` 路径，TierA-E×TableA/B/C，含 `VaultBountyTier*` 隔离库），WFCD `deimosRewards.json`(120KB)；周期卡名需确认（魔胎之境/殁世幽都）。② **解剖圣所**（Sanctum Anatomica / Albrecht 实验室 / Cavia）= `EntratiLabSyndicate`（warframestat 名 "Cavia"），当前 jobs=0，WFCD `entratiLabRewards.json`(5.8KB)。③ **霍瓦尼亚中央商场 1999**（Höllvania）= `HexSyndicate`（warframestat 名 "The Hex"），当前 jobs=0，WFCD `hexRewards.json`(8.4KB)。⚠ **`HexSyndicate` 冲突待查**：api.rs 现已用 `HexSyndicate` 算 夜灵平野/魔胎之境 昼夜周期（CLAUDE.md「Plains/Cambion share HexSyndicate 150min」）——需确认 1999 赏金是否同一 tag、会否互相干扰。④ 翻译沿用同 join（重下 `_all_tmp.json`）；1999/Höllvania 物品多为新内容，需测覆盖率补 MANUAL。⑤ 注意各地点轮次模型可能不同（Cetus/Fortuna/Cambion=A/B/C，Zariman=单池），逐一核对 | 用户先去 review 已做的，后续再做这些 |
| 33. 扎里曼赏金（坚守者 / The Holdfasts） | ✅ complete（已提交 8a7a95a） | 同构第三平原。`BOUNTY_SOURCES`+`("ZarimanSyndicate","扎里曼")`；`SYNDICATE_DISPLAY` 加 扎里曼→坚守者。奖励表 `resources/zariman_bounty_rewards.json`（WFCD `zarimanRewards.json`，5 档 50-55…110-115，0 漏翻；新增 Credits=现金、虚空刺翎/绒翎/翼翎/冠翎/胶丸、英择谛灯笼 等 MANUAL）。**关键差异**：①当前 worldState 的 `ZarimanSyndicate` 轮换间隙 **无 Jobs**（`Nodes:[]`），故标题改按**等级区间映射**（移动防御50/虚空覆涌60/虚空决战70/虚空洪流90/歼灭110，对照 坚守者.png）而非 jobType——`bounty_title(tag, …)` 加 Zariman 早返回分支。②**Zariman 单一奖励池，不分轮次**（用户确认 + WFCD 仅 C 轮有内容 + 赏金wiki.png 佐证：希图斯/福尔图娜/魔胎都有轮次 A/B/C，唯 Zariman 单池）→ 生成表每档只保留非空轮次（单 C）；前端 `renderBountyPanel` 加 `singleRot`（jobs 全部 rotations≤1）检测：隐藏 A/B/C 选择器、改显「单一奖励池」徽章、底部说明改单池版（新 `.rot-single` 样式）。③钢铁档为 +100 级（150-215），`reward_rotations` 加 `min>=150 → 取 min-100` 归一化复用基础池；钢铁后缀对 Zariman 用 `>=150` 阈值（避免普通 110-115 被误标钢铁）。生成脚本 `_gen_zariman.py`（未跟踪）。`cargo check` 通过 | ✅ **已静态合成**（findings 续18）：DE worldState 对 `ZarimanSyndicate` 恒不下发 Jobs（窗口 active 也空），但扎里曼赏金固定→不需要 Jobs。`parse_bounties` 对 Zariman 走 `synthesize_zariman_jobs()`：内置 5 档 (50-55…110-115) 静态模板，title 按等级映射、rewards 取单池表，倒计时用 syndicate `Expiry`（Jobs 空也有）。`static_bounty_job()` 复用。阵营(Grineer/Corpus)/精确轮次需 seed 推算→省略（面板不显示）。构建 EXIT=0（bot8x6im3） |
| 32. 奥布山谷赏金（Solaris United） | ✅ complete（已提交 6e15d0e / 标题 621dc78 / 头部 be6cc16 1016b19） | 复用 Cetus 同构：`BOUNTY_SOURCES` 加 `("SolarisSyndicate","奥布山谷")`（名字对齐 Vallis 周期卡→前端零改动自动可点）。生成 `resources/solaris_bounty_rewards.json`（WFCD `solarisBountyRewards.json`× 同 join，7 档 5-15…50-70+100-100，**0 漏翻**，MANUAL 补 Fortuna 资源/债券/孢子/热泥/泰帕结核）。`reward_tier` 修复 `VenusTier…` 前缀（split("Tier") 取后字母，原 strip_prefix 对 Venus 失效）。`reward_rotations(tag,…)`+`solaris_rewards()`/`rewards_for(tag)` 按 syndicate 分流，tag 经 `parse_bounty_job`→`parse_bounties` 传递。`bounty_title` 加 6 个 Venus 任务名+Narmer Venus 变体；`bounty_type_zh` 加 Recovery=回收/Ambush=伏击。生成脚本 `_gen_solaris.py`（根目录,未跟踪） | 标题已用 `索拉里斯.png` 核对修正：猎人杀手/冷餐/存活证明/伏击信使/尘土部队/焦土大地(钢铁之路)/粉碎邪教(合一众)。奖励物品用户确认对得上。后续：Entrati（魔胎之境）同构 + 奖励池接入「检查更新」 |
| 31. 赏金卡片重做 + 奖励池中文 + 当前轮次 + 任务名 | ✅ complete（已提交 6800fb4） | 奖励池中文(WFCD 掉落表×i18n)；卡片重做(橙头+4列斑马网格)；按轮次 A/B/C；**当前轮次识别**(rewards 的 Table 字母)→只显示当前轮、锁定非当前轮；任务名权威硬表(lvl100→钢铁之路, Narmer 独立标题)。多轮迭代：chip→纯文字网格、合并→分轮次→锁定 | 用户给希图斯参考图否决初版（"零散/简陋"），要带奖励池的卡片。数据：WFCD `warframe-drop-data/cetusBountyRewards.json` × `warframe-items/All.json`(name↔uniqueName) × `baro_zh.json`(uniqueName→zh) join → 离线生成 `resources/cetus_bounty_rewards.json`(7 档预翻译,0 漏翻)。翻译用"每名优先取有 zh 的 uniqueName"+ bilingual 补 + 手动补(Endo=内融核心 等内部名) + "XX蓝图/部件"分解。后端 BountyJob 加 title/rewards(A/B/C 去重合并+rarity)，按等级区间 match。前端重做卡片(橙头+每赏金块+奖励网格按稀有度配色)。clippy 零警告。⏳ 构建中。生成脚本 `_gen_bounty.py`(根目录,未跟踪)。标题仍 best-effort | 解析 `SyndicateMissions[CetusSyndicate].Jobs`(7) → `BountyJob`(类型/等级/MR/阶段/声望/档位) + `BountyInfo`(地点/刷新倒计时)。models+state+api(`parse_bounties`,`bounty_type_zh`,`reward_tier`,`BOUNTY_SOURCES` 可扩展其他平原)+lib(fetch 存/tick 刷倒计时/payload)。前端：有赏金的周期卡加「赏金」标签+可点，点击在卡片下方内嵌展开 `#bounty-panel`(档位徽章/类型/等级/MR/阶段/声望表 + 刷新倒计时 + 关闭)，再点收起。**奖励物品池暂缺**（rewards 字段只是奖励表引用，需后接掉落表）。clippy 零警告。⏳ 构建中。后续：其他平原（Solaris/Entrati）同构扩展 + 奖励物品 |
| 29. Baro 货物中文 + 可更新物品表 | ✅ complete（待提交，构建通过） | 弃用用户初始 bilingual JSON（英文名键、MOD 翻不了），改用 **WFCD/warframe-items `i18n.json`**（uniqueName 键、含 zh、16427 条、MOD 全覆盖）。抽精简 `uniqueName→简中` 表（1.3MB）embed 进 `resources/baro_zh.json`。新模块 `item_i18n.rs`：`OnceLock<RwLock<HashMap>>`，启动 `init()` 优先读 `{app_data}/baro_zh.json` 否则用内置；`translate()` 原路径+去 `StoreItems/` 段两次查；`update_from_remote()` 下载 51MB→抽 zh→写 app_data→热替换。`parse_void_trader` 命中中文否则英文兜底。设置页加「检查更新」按钮(`update_item_names`/`item_names_count` 命令)。⏳ 构建中；命中率待 6-12 Baro 上线用真实 Manifest 实测 |

## 阶段 20 实现要点

**关键设计**：因游戏永远在后台（挂机场景），屏幕区域框选不可行（后台窗口在屏幕上被遮挡）。改为在 `PrintWindow` 捕获的整窗截图上框选，预览图走与 OCR 完全相同的 `capture_full`+`strip_frame`，框选分数 1:1 对齐裁剪坐标。

**改动文件**：
- `capture.rs`：`capture_preview_data_url`(BGR→RGB→PNG→手写 base64 编码器，不引依赖)
- `lib.rs`：`capture_preview` / `test_recognize` 命令 + managed templates clone + invoke_handler 注册 + 私有 `resolve_hwnd`
- `mission_timer.rs`：`MATCH_THRESHOLD` 改 pub；`apply_ocr` 截点恢复补回基准
- `index.html` / `styles.css` / `main.ts`：校准面板（截取画面/框选时间框/框选维生系统框/测试识别/保存校准）

## 阶段 21 实现要点 — 提醒方式 + 测试

- 配置 `mission_timer.alert_method`：`"focus"`(默认，`bring_to_front`) / `"toast"`(`tauri-plugin-notification`)
- OCR 线程(std::thread)无 AppHandle → toast 经 `mpsc<AlertKind>` 转发给 `lib.rs` 持 AppHandle 的转发线程发通知（与 `log_tx` 同模式）；`dispatch_alert()` 在 `mission_timer.rs` 选 focus/toast
- `AlertKind::message()` 给 (title, body)；`test_alert` 命令 + 设置 tab 测试按钮
- ⚠ **Windows toast 只在"已安装"构建稳定显示**（需 AppUserModelID）；裸 `tauri dev` exe 可能不弹

## 阶段 22 实现要点 — UI 重排

- `index.html`：`.timer-hero`(时间显示+`.timer-meta` 维生灯/下一节点/按钮) → `.timer-config-strip`(OCR间隔+两提醒开关) → `<details class="calib-panel">`(模式单选 + 框选工具，默认收起) → 日志卡片
- 模式单选 `name="timer-mode"` 移入校准面板，保留所有 id/事件；移除 `#calib-mode` 元素与 main.ts `modeLabel`/`updateModeLabel`
- `styles.css`：新增 `.timer-hero` / `.timer-config-strip` / `.cfg-item` / `details.calib-panel` / `.calib-summary` / `.calib-body`；删 `.timer-main`/`.timer-left`/`.timer-right`/`.timer-controls`/`.timer-interval`/`.timer-toggles`/`.calib-header`

## 阶段 23 候选方向 — 识别精度（新主线）

用户实测后**框选工具已通过**，剩下的核心痛点是 **OCR 识别精度**（时间 + 维生）。用户回忆 **Python 原版模板匹配精度很高**。下一步排查路径：
- 拿到 Python 版 OCR 源码 → 对比预处理差异（二值化方式/阈值、形态学清理、模板来源分辨率、匹配算法 NCC vs 其他）
- 或用户提供失败 ROI 截图 → 定位是预处理 / 模板质量 / 匹配算法
- 当前实现：NCC 模板匹配，阈值 0.65，二值化 160，多尺度 1.0x/0.85x

## 阶段 24 实现要点 — 文档补全 + Baro 倒计时格式

### CLAUDE.md / AGENTS.md 补全
- 模块表：`models.rs` + `BaroItem`/`BaroInfo`、`state.rs` + `baro`、`api.rs` + Baro 解析
- 新增「周期自愈 (`roll_forward_cycle`)」章节：Plains/Cambion 重建 bounty 后滚动、Zariman 仅滚动保留阵营
- 新增「Baro Void Trader」章节：后端解析、`name_from_path` CamelCase 拆分、前端 `baroSig` 滚动条防弹回优化、倒计时本地翻转
- 顶部加注 `AGENTS.md` 是 CLAUDE.md 镜像，保持同步

### Baro 倒计时中文格式 (`fmt_remain_baro`)
- 独立函数，不影响 `fmt_remain()`（裂缝/周期仍用 h/m/s 短格式）
- 格式：`"X天X小时X分钟X秒"` / `"X小时X分钟X秒"` / `"X分钟X秒"` / `"已离开"`
- `parse_void_trader()` 和 `build_payload()` 中 Baro 的 `remain_str` 改用此函数
- 编译通过，零警告

### 改动文件
- `CLAUDE.md`、`AGENTS.md` — 文档同步
- `src-tauri/src/api.rs` — 新增 `fmt_remain_baro()` + `parse_void_trader()` 调用
- `src-tauri/src/lib.rs` — import + `build_payload()` 调用

## 阶段 25 执行结果 — Python vs Rust OCR 逐项对比

### 分析范围
- Python 源码：`C:\Users\TDD\Desktop\warframe_monitor\`（`ui/mission_timer.py` + `wf_capture_worker.py`）
- Rust 源码：`src-tauri/src/ocr.rs` + `mission_timer.rs` + `capture.rs`
- 模板对比：10 个数字模板逐文件 MD5 校验

### 结论：模板文件完全相同，差异在匹配管线参数

7 处差异，按影响力排序：

| # | 差异点 | Python | Rust | 影响等级 |
|---|--------|--------|------|----------|
| 1 | NMS 重叠公式 | `inter/min(area_a,area_b)` | `inter/(area_a+area_b-inter)` | **P0** — Python 抑制力是 Rust 的 1.7-2.0× |
| 2 | 匹配阈值 | 0.70 | 0.65 | **P0** — Rust 多接受 0.65-0.70 区间的噪声 |
| 3 | 裂缝时间 ROI h | 0.025 | 0.075 | **P1** — Rust 捕获 3× 高度，引入更多背景噪声 |
| 4 | 维生 HSV S/V 阈值 | S>0.31, V>0.47 | S>0.4, V>0.3 | **P2** — V=0.3 太低，暗像素误判红 |
| 5 | NCC 数值精度 | OpenCV `double` + SIMD | 手写 `f32` 四层循环 | P3 — 小模板差异 <1% |
| 6 | strip_frame 策略 | 16:9 宽度白名单 | 宽高比启发式 | 无差异（均正确跳过 16:10） |
| 7 | 多尺度模板 | 无（单尺度） | 1.0x + 0.85x | Rust 改进，不是退步 |

### 修复方案（待用户确认后执行）

**P0（一行+一处改动，预期收益最大）：**
- `mission_timer.rs:9`：`MATCH_THRESHOLD = 0.65` → `0.70`
- `ocr.rs:194-227`：NMS 公式 `inter/union` → `inter / min(area_a, area_b).max(1)`

**P1（需游戏内验证）：**
- `config.rs`：裂缝 timer ROI h `0.075` → `0.03`

**P2（两个常量改动）：**
- `mission_timer.rs:339-374`：HSV 阈值 `s>0.4` → `s>0.31`，`v>0.3` → `v>0.47`

### 关键教训
- 模板文件完全一致但精度差距明显 → 问题不在模板，在**匹配管线参数**
- 最隐蔽的差异是 NMS 公式——作者可能有意用了非标准 IoU，恰好对 Warframe 字体效果更好
- Python 的三参数组合（阈值 0.70 + 激进 NMS + 紧凑 ROI）是实战调优的结果，移植时丢失了这些隐性知识

## 阶段 27 实现要点 — 动态分钟数 + 自定义提示词

用户实测阶段 26 的 OCR 改动「效果还不错，就是一开始慢了点」，并提出两个需求。

**需求 1 — 节点提醒随时间显示里程碑（5/10/15…）**
- `mission_timer.rs` `apply_ocr`：截点跨桶时算 `milestone_min = current_bucket * (CHECKPOINT_INTERVAL_SECS/60)`
- 日志 `"⚠ {min}分钟节点"`、`status_text` `"{min}分钟节点 — 请切回游戏"`、提醒体三处都带上里程碑

**需求 2 — 自定义提示词（留空用默认）**
- `config.rs`：新增 `checkpoint_alert_text`（默认 `"⚠ 到达 {min} 分钟节点 — 请切回游戏"`）/ `hp_alert_text`（默认 `"🚨 维生系统 ≤ 20% — 请补充维生胶囊"`）
- `mission_timer.rs`：`AlertKind` 枚举改为 `AlertMsg { title, body }`（文本在 OCR 线程内已解析，因其有 config 访问权）；`render_alert_text()` 替换 `{min}` + 空串回退默认
- `lib.rs`：alert 通道类型 `AlertKind`→`AlertMsg`，转发线程直接发 `msg.title/body`；`test_alert` 预览配置的节点提示词（`{min}`→5）
- 前端（UI 迭代两轮后定稿）：提醒方式行加「自定义提示词…」按钮 → 点开**居中弹窗** `#alert-text-modal`（两个输入框 + `{min}` 说明 + 完成按钮，点背景/完成关闭）；`main.ts` 加载/change 保存 + 弹窗开关；`styles.css` 加 `.modal-overlay`/`.modal-box`/`.settings-text-row`。「关闭窗口行为」改名「关闭模式」，删掉所有灰色说明小字

**说明（"一开始慢了点"）**：首次同步无基线，需 `|OCR−wall|≤60s` 才接受，配合 2s 轮询间隔，开局锁定要 1–2 个周期——属预期行为，本阶段未改。若用户希望更快，可后续降低首读容差或缩短初始轮询。

**验证**：`cargo check` 零警告；`tsc --noEmit` 通过；`npx tauri build` 出 MSI/NSIS 成功（共 3 次构建）。游戏内验证待用户实测。

## 阶段 41–45 — 分发打磨（2026-06-28，已发布 v1.0.1）

软件已改名 **VoxAlic**，仓库 https://github.com/SwingRainbow/VoxAlic 。本日围绕"可对外分发"做了一轮打磨：

| 阶段 | 状态 | 说明 |
|------|------|------|
| 41. v1.0.0 发布 | ✅ | GitHub Actions 工作流稳定（签名用 `npx tauri signer sign -f` 文件方式，**勿改回 env var**）。Release 产出 exe+sig+latest.json |
| 42. 品牌图标 | ✅ | ChatGPT DALL-E 六边形星形编织结 → Python 重上色 **#FFB929 金→#562583 紫渐变 + 透明背景** → `npx tauri icon` 生成全尺寸。**关键坑见下** |
| 43. 任务计时锁定 | ✅ | **仅生产版锁定，开发版可用**：`src/main.ts` DOMContentLoaded 用 `(import.meta as any).env?.PROD` 判断，给 timer tab 加 `.locked`+disabled+🔒；click handler 拦截 locked/disabled。`.tab-btn.locked` 样式在 styles.css。index.html 按钮保持原样（不硬编码） |
| 44. 单实例 | ✅ | `tauri-plugin-single-instance`（lib.rs 注册，回调 `get_webview_window("main")` → show+set_focus）。第二次点击图标唤起已有窗口而非新开 |
| 45. v1.0.1 发布 + 更新实测 | ✅ | 版本号三处同步（tauri.conf.json/Cargo.toml/Cargo.lock）→ commit `9ea819b` + tag `v1.0.1` → push 触发 CI。**用户在 1.0.0 实测「检查更新」→ 1.0.1 完美运行** |

### ⚠ 图标嵌入关键坑（务必牢记）
- **`cargo clean -p voxalic` 清不掉 tauri-build 缓存的图标资源（.res）**，导致重建后 exe 仍嵌入旧图标。**必须完整 `cargo clean`** 才能强制重新嵌入 ICO。
- 验证方法：`[System.Drawing.Icon]::ExtractAssociatedIcon($exe)` 提取后看图（从全新文件名复制再提取可排除 shell 路径缓存）。
- 托盘图标来自内嵌 PNG（tauri.conf），Explorer/桌面图标来自 exe 的 PE 资源 ICO——**两者来源不同**，托盘变了不代表 exe 资源变了。
- Windows 图标缓存顽固：删 `%LOCALAPPDATA%\Microsoft\Windows\Explorer\iconcache*`/`thumbcache*` + 重启 explorer + `ie4uinit -show`；桌面快捷方式 .lnk 单独缓存，删除重建最快。

### 其他踩坑/经验
- 国内 cargo 慢：`~/.cargo/config.toml` 配 **USTC sparse 镜像** `sparse+https://mirrors.ustc.edu.cn/crates.io-index/`（rsproxy 的 git index URL 已失效）。
- 快速验证不打安装包：`npx tauri build --no-bundle`。
- NSIS 安装包文件名默认已含版本号：`VoxAlic_1.0.1_x64-setup.exe`。

### 待办（下阶段，按用户优先级）
1. **别人电脑实测** — 用户会把安装包发给他人测试，关注 SmartScreen 警告（无商业签名）、WebView2、下载慢。等反馈。
2. **更新弹窗显示 changelog** — 留到 v1.0.2。UI 已就绪（`update-modal-notes` white-space:pre-wrap + 「稍后再说」按钮），缺口是 `release.yml:71` `notes="VoxAlic v$v"` 写死。方案：建 `CHANGELOG.md`，工作流解析当前版本段 → latest.json notes + release body。
3. **更新下载加速** — GitHub 对国内慢（安装包 ~60MB）。用户倾向**后续用 Gitee 码云镜像**。备选：国内对象存储 OSS/COS、GitHub 公共代理。现先不弄。

### 安全约束（务必保持）
- 私钥 `warframe-monitor.key` 已 gitignore，**绝不提交**
- 密钥密码 `[REDACTED]` → GitHub Secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- GitHub Secrets 为 **Repository** secrets（非 Environment）

## 阶段 46–49 — Gitee 镜像 + 双源更新 + CI 全自动（2026-06-28，已上线 v1.0.4）

| 阶段 | 状态 | 说明 |
|------|------|------|
| 46. 安装包瘦身 | ✅ | `tauri.conf.json` `webviewInstallMode`：`offlineInstaller`(203MB) → `downloadBootstrapper`(**8.86MB**)。为上 Gitee（单附件上限 100MB）+ 国内下载体验。Win10/11 多预装 WebView2 |
| 47. 双源更新（GitHub/Gitee 可选） | ✅ | `lib.rs` `build_source_updater(app, source)` 按 source 切 endpoint：`gitee`→`https://gitee.com/Swing_Rainbow/vox-alic/raw/master/latest.json`，否则 GitHub。`check_for_update`/`install_update` 加 `source: String` 参数。前端加 `name="update-source"` 单选（Gitee 默认），main.ts `updateSource()` 传参。加 `url="2"` 依赖。`latest.json` 从 .gitignore 移除（纳入跟踪） |
| 48. Gitee 手动发布（v1.0.2/1.0.3） | ✅ | Gitee 仓库 `Swing_Rainbow/vox-alic`，remote 名 `gitee`。手动流程：build→sign→生成 Gitee latest.json→push origin+gitee→Gitee API 建 release（`Invoke-RestMethod` UTF-8 form 编码避免乱码）+ `curl -F` 传附件。验证 raw latest.json + 下载 200 OK |
| 49. CI 全自动 | ✅ | `release.yml` 加「Publish to Gitee」步：GitHub 发布后 CI 生成 Gitee latest.json → commit + **force-push** 代码+latest.json+tag 到 Gitee（token URL）→ Gitee API 建 release + 传附件。checkout `fetch-depth: 0`。Secret `GITEE_TOKEN`。**v1.0.4 首次全自动，四项验证全通过**（GitHub+Gitee 两边 release+latest.json 均 1.0.4，下载 200） |

### 现行发版流程（三步，CI 全包）
1. 改版本号三处：`tauri.conf.json` / `Cargo.toml` / `Cargo.lock`
2. `git push origin master`
3. `git push origin v1.0.x`（tag）

CI 自动：构建 → 签名 → GitHub Release → Gitee 镜像+发行版+安装包+latest.json。

### ⚠ 关键约束
- **别再手动 `git push gitee`** —— Gitee 由 CI force-push 全权管理，手动推会冲突/分叉。只推 origin。
- 本地 nsis 不再自动有新版安装包（云端构建）。需要本地包：`npx tauri build` 或从 release 下载。
- `GITEE_TOKEN` = `1208ca...`（用户暂未重置，已在聊天明文出现过；若日后泄露顾虑，重置后只更新 GitHub Secret，别贴聊天）。

### 待办
1. **更新弹窗 changelog** — UI 已就绪，缺口：`release.yml` 两处 notes 都写死 `VoxAlic v{ver}`。方案：建 CHANGELOG.md，CI 发版时解析当前版本段填入 GitHub+Gitee 两边 latest.json 的 notes + release body。
2. **发给别人实测** — 关注 SmartScreen/WebView2 联网/下载速度。
3. **双源更新游戏外实测** — 装 1.0.2/1.0.3 → 选 Gitee → 更新到 1.0.4。

## 阶段 50 — 物品库 Gitee 决策 + 本地垃圾清理（2026-06-28）

### 物品库（item_i18n）是否做 Gitee 加速 → 决定不做
- **内置表**：`src-tauri/resources/baro_zh.json`（1.3MB / 16427 条 `uniqueName→简中`），`item_i18n.rs:17` 用 `include_str!` 编译进 exe，首次离线即用。
- **三层优先级**：① 用户下载覆盖 `{app_data_dir}/baro_zh.json`（点过检查更新才有）→ ② 内置默认 → ③ `name_from_path()` CamelCase 拆词兜底（显示英文）。
- **检查更新逻辑**（`item_i18n.rs:84` `update_from_remote`，前端 `main.ts:920`→`update_item_names`）：从 WFCD 官方第三方仓库 GitHub raw 下 i18n.json（51MB），抽 `zh.name` 压紧凑表，落盘+热替换。**数据源是别人的仓库（GitHub raw 写死 `item_i18n.rs:20`），非自有，没法 CI 推 Gitee 镜像。**
- **结论：不做 Gitee 加速**。低频（只影响发版后 DE 新出物品）+ 内置兜底 + 发版自带新库。用户原话「内置的话每次新版本发布都会更新库，没什么好担心」。
- ⚠️ 内置表不自动更新（当前文件日期 Jun 1）。"发版即更新库"前提=发版前手动刷新：软件内点检查更新→把 `{app_data_dir}/baro_zh.json` 拷回 `src-tauri/resources/baro_zh.json`→再 build。忘了不致命（有英文兜底）。

### 本地磁盘垃圾清理（不影响 git，删的都是 gitignore 挡着的本地文件）
- **已删一次性临时数据 ~63MB**：`_all_tmp.json`(51M)、`_deimos_tmp.json`、`_ws_tmp.json`、`warframe-bilingual_2026-06-01.json`。
- **7 个 `_gen_*.py` 全保留**（bounty/solaris/deimos/zariman/hex/entrati_lab/circuit）：检查后无一冗余，每个对应一个仍在用的赏金奖励资源，是资源生成方式的唯一记录。
- ⚠️ **副作用**：这 7 个脚本都靠 `json.load(open('_all_tmp.json'))`，删了输入文件后直接跑会报错。未来要刷新赏金表需重下 WFCD all.json 改名放回。**当下零影响**（软件/构建/CI/git 全正常，删的是开发原料不是运行依赖）。
- 规划文档、截图、`target/`、`node_modules/`、密钥——按用户要求全部未动。

### 未来事项（已记 memory，当下不用管）
- DE 出新声望/开放区域时会连带新赏金任务+奖励池+新物品。更新路径清单见 memory `reference-new-syndicate-update-path.md`：赏金任务改 `api.rs` BOUNTY_SOURCES、奖励池仿 `_gen_*.py` 生成、物品名刷内置表。现有 6 个 syndicate 是模板。

### memory 更新
- `project-voxalic-release.md`：加物品库 Gitee 决策段。
- 新建 `reference-new-syndicate-update-path.md`：新声望更新清单。
- `MEMORY.md` 索引同步（发布现状行更新到 v1.0.4，新增新声望路径行）。

## 阶段 51 — 开发版重建 + 订阅托盘提醒 + 代码图谱（2026-06-28，dev 已构建，未提交）

本阶段全部在**开发版**（`target/release/voxalic.exe`，`--no-bundle`）上迭代，未打安装包未发版。

### 51.0 开发版/发行版分工（用户约定）
- **桌面图标 = 安装的发行版**（稳定版）；**`target/release/voxalic.exe` = 测试开发版**，平时改代码都在它上面。
- 流程：开发版测新功能 → 稳定后才 `npx tauri build` 打包发布。重建开发版用 `npx tauri build --no-bundle`（带 custom-protocol，~1–1.5min）。重建前先 `Stop-Process voxalic`（否则占用 WebView2Loader.dll → os error 32）。
- 把本地开发版从 1.0.3 重建到 1.0.4（版本号三处已一致）。memory `feedback-dev-vs-release-binaries`。

### 51.1 订阅提醒改造：toast → 托盘闪烁 + 悬停弹窗 + 裂缝标注
**需求**：三类订阅（裂缝/周期/仲裁）原走右下角 Windows toast（一闪即逝，AFK 回来看不到）→ 改为**持久托盘指示 + 悬停弹窗**；**任务计时提醒保持 toast 不变**。裂缝列表额外把命中订阅的行**标注**出来（条目多，方便找）。

**后端（`lib.rs`）**
- 新 `SubNotify{kind,icon,title,detail,ts}` + 独立 `notify_tx/rx` 通道。`check_fissure/cycle/arbitration_alerts` 改发 `SubNotify`（不再走 toast 通道）。
- 托管态：`NotifyList`(Arc<RwLock<Vec<SubNotify>>>) + `FlashFlag`(Arc<AtomicBool>) + `notify_tx`（供测试命令）。
- 订阅转发线程：累积列表(上限50) + emit `sub-notify` + 置 `FlashFlag=true`。
- 托盘闪烁线程：读 `FlashFlag`，每 500ms 在 正常/内芯红 帧间切图标。
- 托盘 `with_id("main")`；事件：**Enter** 显弹窗（emit 当前快照防竞态 + 定位光标上方 + show 不抢焦点）、**Leave** 隐、**左键** 唤主窗+停闪。"notify" 窗失焦自动隐。
- 命令：`get_notifications`/`clear_notifications`/`test_notification`（注入假提醒，免等真裂缝）。
- ⚠️ 坑：托盘图标须 `Image::new_owned` 转 owned 才能进 'static 闪烁线程（借用 `default_window_icon` → E0521 app escapes）；托盘 `set_icon`/`set_tooltip` 须 `run_on_main_thread`。

**弹窗（独立窗口）**
- `tauri.conf.json` 加第二个窗口 `label:"notify"`（transparent/decorations:false/alwaysOnTop/skipTaskbar/visible:false）。
- 新建 `notify.html`+`src/notify.ts`（暗色卡片列表，`get_notifications` 初始 + listen `sub-notify` + 「清空」）。`vite.config.ts` 改双入口 `rollupOptions.input={main,notify}`。

**前端标注（`main.ts`）**
- `fissureSubscribed(f)` 镜像后端匹配 → `renderFissures` 命中行加 `.subscribed`（金 #FFB929 左边条+描边）+ 🔔。`saveAlerts` 后重渲裂缝列表。仅裂缝标注，周期/仲裁不标。
- 设置→订阅底部加「测试提醒效果」`btn-test-notify` → `test_notification`；hint 文案去掉过时的 toast 说法。

**闪烁形态迭代（共 6 版，配色教训见 findings 续43）**：红点 overlay→整图标明灭(透明)→纯红整格→呼吸金光晕→**logo 镂空内芯脉动**(`hole_mask` flood-fill 识别洞)→青白 bloom→**最终：内芯纯亮红 (255,45,45)、`levels=[1.0,0.0]` 亮/暗交替、500ms、bloom 外扩 2px**。用户决定红闪先用着，备选(自动弹窗/整logo变色/FlashWindowEx)未采纳。

**悬停弹窗交互迭代**：初版做成左键点击弹出（且有空内容竞态 bug）→ 用户要"悬停显示、左键唤主窗" → 改 Enter/Leave + Enter 时主动推快照修空白。已知取舍：悬停显示/移开即隐，移进弹窗会触发 Leave 收起 → 只能扫一眼，「清空」够不着（用户接受）。

**验证**：`tsc --noEmit` 通过；`cargo check --features custom-protocol` 通过（修了一次 E0521）；`npx tauri build --no-bundle` 多次成功（notify.html/notify.js 正确产出双入口）。用户暂未久等真裂缝实测，已用测试按钮验证链路。memory `project-subscription-tray-notify`。

### 51.2 代码图谱 `CODE_NOTES.md`（新建，长期维护）
- 应用户「写一份代码标注文件」要求，新建仓库根 **`CODE_NOTES.md`**：逐块标注全代码功能+联动。含全局联动总览(线程/4通道/4事件/命令/托管态/数据流图)、后端 11 模块函数级说明、前端、「订阅托盘提醒」端到端专题、资源/构建。
- **用户立规矩**：以后**每加新功能/做新旧联动都要同步更新 CODE_NOTES.md**，作为交付的一部分。memory `feedback-maintain-code-map` + `reference-code-notes-file`。

### memory 更新（本阶段）
- 新建：`feedback-dev-vs-release-binaries`、`project-subscription-tray-notify`、`reference-code-notes-file`、`feedback-maintain-code-map`。
- `MEMORY.md` 索引同步。

## 阶段 52 — 全量审查清理 + 物品库版本标注 + 母亲石印（2026-06-28/29，dev 已构建，未提交）

### 52.1 全量代码审查（安全/优化/冗余）
- **安全（总体良好）**：源码无硬编码密钥/token（grep 干净）；网络全 HTTPS + 默认 TLS 校验，无 `danger_accept_invalid_certs`；更新器固定 endpoint + pubkey 验签、`source` 仅两 URL match 无注入；`unsafe` 仅 Win32/GDI/注册表 FFI。两个低风险提示（未改）：前端 `innerHTML` 21 处不转义（数据源官方 API+翻译表可信，`notify.ts` 已转义）；`tauri.conf` `csp:null`（只加载本地内容，风险低）。
- **已修（均 clippy 零警告 + 构建通过）**：① 新增 `make_center_pulse_frames` 的 `needless_range_loop` → enumerate；② **冗余**：`resolve_hwnd` 在 lib.rs+mission_timer.rs 两份完全相同 → 合并为 `window::resolve_hwnd(keyword)`，7 调用点统一；③ **健壮性**：两个 reqwest client 无超时 → 加 `timeout`（worldstate 30s、i18n 180s）。
- **优化建议（未改，留待专门任务）**：前端 `handleUpdate` 在 `tick-update`(每秒)也触发 → renderFissures/Cycles/BountyPanel/Arbitration 每秒全量重建 innerHTML（长列表滚动每秒重置、`<select>` 被打断）。已对 Baro(`baroSig`)/订阅规则(`_lastAlertSig`)防抖，建议同法推广到裂缝表/赏金面板。#1 前端优化点，改动面大需实测。`build_payload` 每 tick 全量 clone、flash 线程 500ms 空转——开销可忽略，不值得改。

### 52.2 物品库改版本标注（替代「检查更新」按钮）
- **决策**：物品库随发行版走（WFCD 每次游戏更新自动重建、含 zh，研究见 findings 续44），manual 检查更新按钮多余 → **砍按钮**，设置→物品库改只读显示游戏版本名。
- **版本名手输不自动抓**：标注本意是"物品库覆盖到哪版"，库是发版冻结快照；自动抓 worldState `Events` 新闻拿到的是 live 最新版本 → 比库新、语义错。手输和库严格一致，用户本就每版看版本名。
- **落地**：`lib.rs` `const GAME_DATA_VERSION="更新 43《Jade 之影：众星》"`（上一版 42《绘影者》）+ `game_data_version()` 命令；前端 `itemnames-status` 显示（最终用户要求**去掉"对应游戏版本"前缀**，只剩版本名本身，靠左侧"物品库"标签说明）。**后端 `update_item_names`/`update_from_remote` 保留**（仅作发版前重生成 `baro_zh.json` 工具，无用户入口）。
- ⚠️ **发版流程多一步**：发版时改 `GAME_DATA_VERSION` = 当前游戏版本名（连带刷 baro_zh.json + 软件版本号）。

### 52.3 魔胎之境赏金「声望」→「母亲石印」
- 用户：魔胎之境(Entrati/殁世幽都)赏金实际不给声望，给**母亲石印**(Mother Token)。
- `main.ts` `renderBountyPanel` 的 `section()` 加 `standingLabel = b.syndicate==='魔胎之境' ? '母亲石印' : '声望'`，chip 用之。其它地点不变；解剖圣所(挂魔胎卡)standing=0 本就隐藏不受影响。
- ⚠️ **数字 N 待核**：仍用 `xpAmounts` 和（原"声望"那个数）。若游戏里母亲石印是固定小数量（非几千级），数字会偏大不对 → 待用户游戏内核对后定（按数对得上/固定值/不显示数字 三选一）。

### memory 更新（本阶段）
- `project-voxalic-release` 加「物品库改版本标注」段 + 研究结论。
- `CODE_NOTES.md` 同步：window.rs 加 `resolve_hwnd`、lib.rs 加 `game_data_version`、物品库按钮移除说明。

## 给接手者

**当前代码基线**: commit `a93473a` (master, tag `v1.0.4`, GitHub+Gitee 均已发布)
- `9ea819b` v1.0.1（图标+任务计时锁定+单实例）
- 历史基线 commit `edc3ed5`
- `a2b5fc7` 代码（阶段 20–23）
- `732c6a5` 文档 + `.gitignore`
- `f135a39` 规划文件同步
- `e616711` Baro 倒计时中文格式 + 文档补全
- `52cebbe` 文档：Python vs Rust OCR 对比分析 + CLAUDE.md 更新
- `0a59068` fix：OCR 管线参数对齐 Python（P0 阈值+NMS、P1 裂缝 ROI 高、P2 HSV）
- `edc3ed5` feat：节点动态里程碑 + 自定义提示词弹窗 + 设置页整理（关闭模式）
- **未 push**；`pic/`（截图）未跟踪

**已验证可用**:
- 时间 OCR（1728×1080 游戏内稳定）
- **ROI 框选校准工具**（用户实测非常成功）
- Release 构建零警告 + clippy 零警告，prod 模式

**待用户实测**:
- 提醒方式两种（toast 建议用安装包，需"已安装"身份）
- 阶段 27 节点动态分钟数 + 自定义提示词弹窗
- 设置页新布局（关闭模式 / 提醒方式整合）

**工具**: CodeGraph 0.9.7 已建索引（CLI 用法见 progress 续3）；改代码后记得 `codegraph sync`

**下一步主线**: 识别精度 — P0 修复（阈值 + NMS）预期收益最大，见 findings.md 阶段 25 详细对比

**重要提示**:
- 项目用 GNU 工具链 (`stable-x86_64-pc-windows-gnu`)，不是 MSVC
- WinLibs 需在 PATH 中（构建环境见 `findings.md`）
- Debug 构建报 ordinal 过大，始终用 `--release`
- 架构文档见 `CLAUDE.md`，已知 bug 和踩坑见 `findings.md`
