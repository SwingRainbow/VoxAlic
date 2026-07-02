# API 状态快照 — DE Worldstate 依赖结构

> 本文档记录代码对 `api.warframe.com/cdn/worldState.php` 返回结构的**完整依赖**。
> DE 会不时增删字段、新增节点类型、调整数据结构。
> **目的：每次 push 前对比，一眼看出 API 是否变了。**
>
> 最后更新：2026-07-02，对应已发布版本 **v1.1.1**。

---

## 1. API 端点

| 用途 | URL | 备注 |
|------|-----|------|
| 主力 | `https://api.warframe.com/cdn/worldState.php` | `worldstate_source = "official"` |
| 镜像 | `https://oracle.browse.wf/worldState.json` | 主力失败时的 fallback |

- 请求方式：`GET`，`User-Agent: Warframe/1.0`，超时 30s。
- 返回类型：JSON（顶层 `Object`）。

---

## 2. 解析函数 → JSON 路径映射

### 2.1 `parse_fissures(data)` → `(Vec<Fissure>, Vec<Fissure>, Vec<Fissure>)`

返回三个列表：普通裂缝、钢铁裂缝、虚空风暴。

#### 2.1.1 `data["ActiveMissions"]` — Array

代码只处理 `Modifier` 以 `"VoidT"` 开头且 `is_active()` 为 true 的条目。

| JSON 路径 | 类型 | 用途 | 空值处理 |
|-----------|------|------|----------|
| `m["Modifier"]` | String | 纪元 key（`VoidT1`~`VoidT6`）；不是 VoidT→跳过 | 空字符串 |
| `m["Activation"]` | MongoDB Date | 激活时间，`is_active()` 判断 | `get_ms()` → 0 |
| `m["Expiry"]` | MongoDB Date | 过期时间，计算 `remain_ms` | `get_ms()` → 0 |
| `m["Node"]` | String | 节点 key，查 `node_lookup()` | 空字符串 |
| `m["MissionType"]` | String | 任务类型 key（`MT_SURVIVAL` 等） | 空字符串 → `mission_type("")`="未知" |
| `m["Hard"]` | Bool | 是否钢铁之路 | `false`（默认） |

#### 2.1.2 `data["VoidStorms"]` — Array

代码只处理 `is_active()` 为 true 的条目。

| JSON 路径 | 类型 | 用途 | 空值处理 |
|-----------|------|------|----------|
| `s["Activation"]` | MongoDB Date | 激活时间 | `get_ms()` → 0 |
| `s["Expiry"]` | MongoDB Date | 过期时间 | `get_ms()` → 0 |
| `s["Node"]` | String | 节点 key（九重天节点） | 空字符串 |
| `s["ActiveMissionTier"]` | String | 纪元 key（风暴用这个字段，不是 Modifier） | 空字符串 |

> ⚠️ 注意：`ActiveMissions` 用 `Modifier`，`VoidStorms` 用 `ActiveMissionTier`——两个字段名不同但含义一致。

---

### 2.2 `parse_cycles(data)` → `Vec<CycleInfo>`

返回 6 个周期：夜灵平野、魔胎之境、奥布山谷、双衍王境、扎里曼、霍瓦尼亚。

#### 2.2.1 `data["SyndicateMissions"]` — Array

按 `Tag` 字段过滤：

| Tag 值 | 用途 | 周期名 |
|--------|------|--------|
| `"HexSyndicate"` | 夜灵平野 + 魔胎之境 + 霍瓦尼亚（共享 expiry） | `build_hex_cycle()` |
| `"ZarimanSyndicate"` | 扎里曼 | `parse_zariman_cycle()` |

每个 syndicate 条目：

| JSON 路径 | 类型 | 用途 | 空值处理 |
|-----------|------|------|----------|
| `entry["Tag"]` | String | 辨识 syndicate | 不匹配→跳过 |
| `entry["Activation"]` | MongoDB Date | 激活时间 | `get_ms()` → 0 |
| `entry["Expiry"]` | MongoDB Date | 赏金过期时间 | `get_ms()` → 0 |

> **本地计算的周期**（不依赖 API）：奥布山谷（硬编码 epoch，1600s 循环）、双衍王境（硬编码 epoch，7200s/情绪，5 情绪轮换）。

---

### 2.3 `parse_void_trader(data)` → `Option<BaroInfo>`

#### 2.3.1 `data["VoidTraders"]` — Array

取第一个元素 `trader`。数组为空/不存在 → `None`。

| JSON 路径 | 类型 | 用途 | 空值处理 |
|-----------|------|------|----------|
| `trader["Activation"]` | MongoDB Date | Baro 到达时间 | `get_ms()` → 0 |
| `trader["Expiry"]` | MongoDB Date | Baro 离开时间 | `get_ms()` → 0 |
| `trader["Node"]` | String | 中继站节点 key | 空字符串 → 显示原 key |

#### 2.3.2 `trader["Manifest"]` — Array（仅 Baro 在场时填充）

| JSON 路径 | 类型 | 用途 | 空值处理 |
|-----------|------|------|----------|
| `it["ItemType"]` | String | 物品资产路径（`/Lotus/StoreItems/.../QuantaVandal`） | 空字符串 → 空名 |
| `it["PrimePrice"]` | Number | 杜卡德价格 | 0 |
| `it["RegularPrice"]` | Number | 星币价格 | 0 |

---

### 2.4 `parse_bounties(data)` → `Vec<BountyInfo>`

#### 2.4.1 `data["SyndicateMissions"]` — Array

按 6 个 Tag 过滤（`BOUNTY_SOURCES`）：

| Tag | 中文地点 | card 归属 | Jobs 来源 |
|-----|---------|-----------|-----------|
| `CetusSyndicate` | 夜灵平野 | 夜灵平野 | 实时 `Jobs` |
| `SolarisSyndicate` | 奥布山谷 | 奥布山谷 | 实时 `Jobs` |
| `ZarimanSyndicate` | 扎里曼 | 扎里曼 | **本地合成**（API 永远为空） |
| `EntratiSyndicate` | 魔胎之境 | 魔胎之境 | 实时 `Jobs` |
| `HexSyndicate` | 霍瓦尼亚 | 霍瓦尼亚 | **本地合成**（API 永远为空） |
| `EntratiLabSyndicate` | 解剖圣所 | 魔胎之境 | **本地合成**（API 永远为空） |

#### 2.4.2 `entry["Jobs"]` — Array（每个 job）

| JSON 路径 | 类型 | 用途 | 空值处理 |
|-----------|------|------|----------|
| `j["jobType"]` | String | 赏金类型的资产路径（用于标题推断） | 空字符串 |
| `j["minEnemyLevel"]` | Number | 最低敌人等级 | 0 |
| `j["maxEnemyLevel"]` | Number | 最高敌人等级 | 0 |
| `j["xpAmounts"]` | Array[Number] | 各阶段声望量（`stages = len`，`standing = sum`） | 0 |
| `j["masteryReq"]` | Number | 段位要求 | 0 |
| `j["rewards"]` | String | 奖励表资产路径（含 Table 字母和 Tier 信息） | 空字符串 |

> **`rewards` 路径解析**：`active_rotation_of()` 从路径中提取 `Table` 后字母（A/B/C）；
> `reward_tier()` 提取 `Tier` 后字母（A~E）或 `Narmer`。

---

### 2.5 `parse_circuit(data)` → `Option<CircuitInfo>`

#### 2.5.1 `data["EndlessXpSchedule"]` — Array（新位置，Jade Shadows 后）

取第一个元素。此路径优先；若不存在则 fallback 到旧路径。

| JSON 路径 | 类型 | 用途 | 空值处理 |
|-----------|------|------|----------|
| `s["CategoryChoices"]` | Array | 分类奖励选择 | 无→尝试 fallback |
| `s["Expiry"]` | MongoDB Date | 本周轮换到期时间 | `get_ms()` → 0 |

#### 2.5.2 `data["EndlessXpChoices"]` — Array（旧位置，legacy fallback）

| JSON 路径 | 类型 | 用途 | 空值处理 |
|-----------|------|------|----------|
| — | — | 结构同上但顶层 | — |

#### 2.5.3 每个 choice 条目

| JSON 路径 | 类型 | 用途 | 空值处理 |
|-----------|------|------|----------|
| `c["Category"]` | String | `"EXC_NORMAL"` 或 `"EXC_HARD"` | 不匹配→跳过 |
| `c["Choices"]` | Array[String] | 物品 token（如 `"Soma"`, `"NamiSolo"`） | 空数组 |

---

### 2.6 `parse_arbitration(now_ms)` → `Option<ArbitrationInfo>`

**不依赖 worldstate API。** 完全使用嵌入的离线数据：
- `src-tauri/resources/arbitration_seq.bin` — 44056 字节的索引序列
- `src-tauri/resources/arbitration_nodes_zh.json` — 88 节点中文信息
- `src-tauri/resources/arbitration_meta.json` — epoch 时间戳和步长

数据来源：`arbi.wf.wiki`（不是 DE 官方）。

---

## 3. MongoDB 日期格式（`get_ms()` 兼容列表）

`get_ms()` 在 `api.rs:62` 处理以下所有格式：

| 格式 | 示例 | 检测方式 |
|------|------|----------|
| 纯数字（秒） | `1719900000` | `< 10^11 → ×1000` |
| 纯数字（毫秒） | `1719900000000` | `≥ 10^11 → 原值` |
| `{"$date": number}` | `{"$date": 1719900000}` | Object 含 `$date` 数字 |
| `{"$date": {"$numberLong": "str"}}` | `{"$date": {"$numberLong": "1719900000000"}}` | 嵌套 Object + 字符串 |
| `{"$date": {"$numberLong": number}}` | `{"$date": {"$numberLong": 1719900000000}}` | 嵌套 Object + 数字 |
| `{"$date": {"$numberDouble": "str"}}` | `{"$date": {"$numberDouble": "1.72e12"}}` | 嵌套 Object + Double 字符串 |
| `{"$date": "string_number"}` | `{"$date": "1719900000"}` | Object 含 `$date` 字符串 |

---

## 4. 关键枚举值（API 原始值 → 代码映射）

### 4.1 裂缝纪元（`tier_key`）

| API 值 | 中文 | 排序 |
|--------|------|------|
| `VoidT1` | 古纪 | 1 |
| `VoidT2` | 前纪 | 2 |
| `VoidT3` | 中纪 | 3 |
| `VoidT4` | 后纪 | 4 |
| `VoidT5` | 安魂 | 5 |
| `VoidT6` | 全能 | 6 |
| *其他* | 显示原值 | 99 |

> ⚠️ DE 新增纪元（如 VoidT7）时：`tier_label()` 显示原始 key（退化），`tier_order()` 排到最后。

### 4.2 任务类型（`MissionType` key）

| API 值 | 中文 | API 值 | 中文 |
|--------|------|--------|------|
| `MT_ARENA` | 竞技场 | `MT_LANDSCAPE` | 自由探索 |
| `MT_ARTIFACT` | 中断 | `MT_MOBILE_DEFENSE` | 移动防御 |
| `MT_ASSAULT` | 强袭 | `MT_PVP` | 武形秘仪 |
| `MT_ASSASSINATION` | 刺杀 | `MT_RESCUE` | 救援 |
| `MT_CAPTURE` | 捕获 | `MT_RETRIEVAL` | 劫持 |
| `MT_CORRUPTION` | 虚空洪流 | `MT_SABOTAGE` | 破坏 |
| `MT_DEFENSE` | 防御 | `MT_SECTOR` | 黑暗地带 |
| `MT_DISRUPTION` | 中断 | `MT_SURVIVAL` | 生存 |
| `MT_EVACUATION` | 叛逃 | `MT_TERRITORY` | 拦截 |
| `MT_EXCAVATE` | 挖掘 | `MT_VOID_CASCADE` | 虚空覆涌 |
| `MT_EXTERMINATION` | 歼灭 | `MT_ASCENSION` | Ascension |
| `MT_HIVE` | 清巢 | `MT_ALCHEMY` | 元素转换 |
| `MT_INTEL` | 间谍 | `MT_ENDLESS_CAPTURE` | Legacyte Harvest |
| | | `MT_DEFAULT` | 未知 |
| *其他* | 显示原始 key | | |

> ⚠️ DE 新增任务类型时：`mission_type()` 返回原始 key（前端直接显示英文 key）。

### 4.3 Syndicate 标签（`SyndicateMissions[].Tag`）

| API 值 | 中文地点 | 用途 |
|--------|---------|------|
| `HexSyndicate` | 夜灵平野 / 魔胎之境 / 霍瓦尼亚 | 3 个周期卡 + 赏金 |
| `ZarimanSyndicate` | 扎里曼 | 周期卡 + 赏金 |
| `CetusSyndicate` | 夜灵平野 | 赏金 |
| `SolarisSyndicate` | 奥布山谷 | 赏金 |
| `EntratiSyndicate` | 魔胎之境 | 赏金 |
| `EntratiLabSyndicate` | 解剖圣所 | 赏金 |

### 4.4 无尽回廊分类（`Category`）

| API 值 | 含义 | 前端用途 |
|--------|------|----------|
| `EXC_NORMAL` | 普通回廊·战甲奖励 | `CircuitInfo.normal` |
| `EXC_HARD` | 钢铁之路·灵化之源 | `CircuitInfo.hard` |

---

## 5. 降级 / 兜底逻辑总览

| # | 场景 | 降级行为 | 代码位置 |
|---|------|----------|----------|
| 1 | 未知节点 key | `node_name = node_key`（显示原始 key），`planet = "未知"` | `node_lookup()` → 空 name |
| 2 | 未知纪元 key | `tier_label` 显示原始 key | `tier_label()` fallback |
| 3 | 未知任务类型 | 显示原始 `MissionType` key | `mission_type()` fallback |
| 4 | 周期 `remain <= 0` | Vallis/Duviri 本地重算；syndicate 周期 `roll_forward_cycle()` 滚动；其他显示 "切换中" | `refresh_cached_payload()` |
| 5 | HexSyndicate 无活跃条目 | 夜灵平野+魔胎之境+霍瓦尼亚显示 "未知"/"切换中" | `unknown_cycle()` |
| 6 | ZarimanSyndicate 无活跃条目 | 扎里曼显示 "未知"/"切换中" | `parse_zariman_cycle()` |
| 7 | Baro 不在场 | `items = []`，倒计时到到达；到达时间过期间本地翻转 `active = true` | `build_payload()` |
| 8 | Zariman Jobs 为空 | 本地合成 5 个固定赏金 | `synthesize_zariman_jobs()` |
| 9 | Hex Jobs 为空 | 本地合成 7 个固定赏金 | `synthesize_hex_jobs()` |
| 10 | EntratiLab Jobs 为空 | 本地合成 5 个固定赏金 | `synthesize_entrati_lab_jobs()` |
| 11 | 赏金 `jobType` 无匹配标题 | 关键词 fallback → "赏金任务" | `bounty_title()` |
| 12 | 赏金 rewards 路径无 Table 字母 | `active_rotation = ""` | `active_rotation_of()` |
| 13 | 赏金奖励池无匹配等级 | 钢铁之路回退到 base bracket（−100）；都无 → 空池 | `reward_rotations()` |
| 14 | Circuit 新老位置都无数据 | `None`（前端不渲染回廊面板） | `parse_circuit()` |
| 15 | Circuit 物品 token 无中文名 | 显示原始 token | `circuit_zh()` |
| 16 | 主力 API 抓取失败 | 自动尝试镜像 URL | `fetch_worldstate()` |
| 17 | API 初次抓取失败（网络） | `initialized = true`，允许本地计算数据（仲裁等）显示 | `fetch_store_emit()` Err 分支 |
| 18 | Baro 物品无 i18n 翻译 | `name_from_path()` 取路径末段 + CamelCase 拆分 | `parse_void_trader()` |
| 19 | 仲裁 epoch 之前的时间戳 | `None`（前端不渲染仲裁面板） | `parse_arbitration()` |

---

## 6. 静态嵌入资源（非 API 但影响数据展示）

这些文件编译进二进制，DE 更新游戏内容时可能需要手动更新：

| 文件 | 用途 | 更新方式 |
|------|------|----------|
| `src-tauri/resources/baro_zh.json` | ~16k 物品名（uniqueName → 简中） | 用户点"检查更新"→ 从 WFCD i18n.json 下载重建 |
| `src-tauri/resources/circuit_names.json` | Incarnon 武器 token → 简中名 | **手动维护**（无自动更新） |
| `src-tauri/resources/cetus_bounty_rewards.json` | 夜灵平野赏金奖励池 | **手动维护**（无自动更新） |
| `src-tauri/resources/solaris_bounty_rewards.json` | 奥布山谷赏金奖励池 | **手动维护** |
| `src-tauri/resources/deimos_bounty_rewards.json` | 魔胎之境赏金奖励池 | **手动维护** |
| `src-tauri/resources/zariman_bounty_rewards.json` | 扎里曼赏金奖励池 | **手动维护** |
| `src-tauri/resources/hex_bounty_rewards.json` | 霍瓦尼亚赏金奖励池 | **手动维护** |
| `src-tauri/resources/entrati_lab_bounty_rewards.json` | 解剖圣所赏金奖励池 | **手动维护** |
| `src-tauri/resources/arbitration_seq.bin` | 仲裁序列（44056 字节） | 从 arbi.wf.wiki 重新生成 |
| `src-tauri/resources/arbitration_nodes_zh.json` | 仲裁节点中文信息 | 从 arbi.wf.wiki 重新生成 |
| `src-tauri/resources/arbitration_meta.json` | 仲裁 epoch + step | 从 arbi.wf.wiki 重新生成 |
| `src-tauri/resources/digit_templates/` | OCR 数字模板（0-9.png） | 游戏 UI 改版时重新截图 |

---

## 7. API 变更检查清单（每次提交前）

当一个或多个检查项触发时，更新本文档对应章节：

- [ ] `ActiveMissions[].Modifier` 出现新的 `VoidT*` 值？→ 更新 §4.1，加 `tier_label()` 映射
- [ ] `ActiveMissions[].MissionType` 出现新 key？→ 更新 §4.2，加 `mission_type()` 映射
- [ ] 新增 Syndicate Tag？→ 更新 §4.3，考虑是否加入 `BOUNTY_SOURCES`
- [ ] 新增/重命名 JSON 字段？→ 更新 §2 对应解析函数的路径表
- [ ] MongoDB 日期格式变化？→ 更新 §3，修改 `get_ms()`
- [ ] `EndlessXpSchedule` 结构调整？→ 更新 §2.5，修改 `parse_circuit()`
- [ ] `VoidStorms` 结构调整？→ 更新 §2.1.2
- [ ] 新增节点 key（游戏中新增星球/节点）？→ 更新 `node_lookup()` 和本文档（节点表在 `api.rs:240`，约 160 条）
- [ ] Baro Manifest 物品路径格式变化？→ 更新 `name_from_path()` 和 `item_i18n::translate()`
- [ ] 赏金 `rewards` 路径格式变化？→ 更新 `active_rotation_of()` / `reward_tier()`
