# 任务计时器 — 设计文档

## 概述

将 Python 原版 OCR 任务计时器移植到 Tauri v2，包括屏幕捕获、模板匹配 OCR、任务计时、生命维持检测、5 分钟截点提醒。

## 架构

```
┌─────────────────────────────────────────────────┐
│  std::thread (独立线程, 不阻塞 tokio)             │
│  ┌──────────┐  ┌───────────┐  ┌──────────────┐ │
│  │ capture  │→ │ ocr       │→ │ timer_filter │ │
│  │ PrintWin │  │ normalized│  │ 连续3次验证   │ │
│  │ 截取ROI  │  │ xcorr+NMS │  │ 跳变过滤      │ │
│  └──────────┘  └───────────┘  └──────┬───────┘ │
│                                       │          │
│                                Arc<RwLock<>>     │
│                                MissionTimerState │
└───────────────────────────────────────│──────────┘
                                        ↓
                               tokio 1s tick 读取
                               → Tauri event push 前端
```

## 模块设计

### capture.rs — 屏幕捕获

- Win32 `PrintWindow` + `PW_RENDERFULLCONTENT`，通过 `windows` crate 调用
- 根据 HWND 定位 Warframe 窗口，裁剪 ROI 区域（相对坐标）
- 输出：原始像素字节（BGR 格式）+ ROI 尺寸
- GDI 句柄管理：RAII 结构体按 `DeleteDC → DeleteObject → ReleaseDC` 顺序释放
- 错误处理：捕获失败返回 `Option`，上层跳过本轮 OCR

### ocr.rs — 模板匹配

- 手写归一化互相关（约 60 行），不引入 opencv crate
- 10 张数字模板 PNG 嵌入二进制（`include_bytes!`），运行时解码为灰度矩阵
- 预处理：ROI → 灰度 → 二值化阈值 160 → resize 到与模板一致的比例
- 匹配：滑动窗口 → 各自减均值 → 点积 / (template_std × patch_std) → 阈值筛选
- NMS 合并：IoU > 0.3 去重，按 x 坐标排序，拼接数字串
- 正则验证：期望 `(\d{1,2}):(\d{2})` 格式
- 无匹配时返回 `None`

### mission_timer.rs — 计时器状态机

**状态定义：**
```
enum TimerState {
    Idle,       // 未启动
    Running,    // 计时中
    Paused,     // 用户手动暂停
    Checkpoint, // 5分钟截点，暂停+需要手动继续
}
```

**状态流转：**
```
Idle → Running  （用户点击开始）
Running → Paused（用户点击暂停）
Paused → Running（用户点击继续）
Running → Checkpoint（OCR 结果连续不变 >30s，判定为截点）
Checkpoint → Running（用户确认已回到游戏）
Running/Paused/Checkpoint → Idle（用户点击重置）
```

**同步内计时器：**
- OCR 轮询间隔 2s，但前端每秒 tick 都要更新
- 内计时器：记录启动时刻 `start_instant`，每秒 `elapsed = now - start_instant - paused_duration`
- 轮询时 OCR 结果校验通过 → 用 OCR 值校正内计时器
- 轮询时 OCR 失败或跳变 → 内计时器继续走

**结果过滤：**
- 连续 3 次有效读数且跳变范围在 (-10s, +30s) 内才接受
- 超过范围 → 丢弃本轮，保持上次有效值，内计时器继续
- 连续超过 `_MAX_REJECT` 次 → 认为游戏状态变化，重新初始化

**5 分钟截点检测：**
- OCR 连续数次读取到相同数值 + 内计时器显示过去了一段时间 → 截点
- 进入 Checkpoint 状态 → 前端弹出提醒"游戏可能进入选择界面"

### 生命维持检测

- 独立于 OCR，在单独的第二个 ROI 上运行
- 提取 ROI → 转 HSV → 统计红色像素占比（H ∈ [0,10] ∪ [160,180], S > 100, V > 80）
- 前端显示：百分比数字 + 渐变色条（绿 >50%、黄 20-50%、红 <20%）

### 配置

在 `AppConfig` 中新增：
```rust
pub mission_timer_mode: String,  // "normal" | "fissure"
pub normal_roi: ROIConfig,       // x, y, w, h (相对坐标 0.0-1.0)
pub fissure_roi: ROIConfig,      // 同上
pub life_support_roi: ROIConfig, // 生命维持 ROI
```

提供默认值（硬编码你当前使用的坐标），可通过设置 UI 调整。

## 前端

### 任务计时 Tab

- 大字号数字 `MM:SS`（或 `HH:MM:SS` if ≥1小时）
- 模式切换：radio 按钮（普通 / 裂缝）
- 开始/暂停/重置按钮
- 维生系统状态条（绿→黄→红渐变色 + 百分比）
- 状态提示（运行中/暂停/截点提醒）
- 计时变化走现有 1s `tick-update` 事件，新增 `mission_timer` 字段

### 设置扩展

- 在设置 tab 新增"任务计时"区域
- 模式默认值选择
- ROI 坐标输入（高级，折叠在 collapsible 中，默认值即开箱可用）

## 数据流

```
2s 轮询线程:
  PrintWindow Warframe → 裁剪 ROI_1 → OCR 识别 → 过滤 → 更新 timer state
                         裁剪 ROI_2 → HSV 生命维持 → 更新 life_support state

1s tokio tick:
  读取 Arc<RwLock<MissionTimerState>>
  → MissionTimerPayload { elapsed_secs, life_support_pct, life_support_level, state, mode }
  → Tauri event "tick-update" 合并推送
```

## 错误处理

- 窗口未找到 → 暂停计时，前端提示"未检测到 Warframe 窗口"
- 重复 OCR 失败 → 保持内计时器，前端提示"OCR 暂不可用"
- 连续超过 `_MAX_REJECT` 次 → 重置为 Idle
- 游戏最小化/黑屏 → OCR 无结果，内计时器继续但不更新 OCR 校正

## 测试策略

- 手动测试：实际打开 Warframe 进入任务，验证计时精度
- 开发阶段：用你原版 10 张数字模板 + 之前截的样本 ROI 做离线 OCR 测试
- 边界：0:00 跳变、5 分钟截点、窗口最小化恢复、模式切换
