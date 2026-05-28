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
