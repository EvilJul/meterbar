## Context

空仓库起步，产品目标是 macOS 菜单栏常驻的 AI 用量与本机状态面板。第一期只打通 Cursor 个人账号（Pro+ 已实测 `GET https://cursor.com/api/usage-summary`），系统指标与延迟一并实采。个人账号无官方用量 API；凭证为浏览器 Cookie `WorkosCursorSessionToken`。架构需预留跨平台与多 Provider，但本 change 只交付 macOS + Cursor。

背景方案亦见 Obsidian：`项目/usages/技术方案/2026-08-05-Cursor-First技术方案`。

## Goals / Non-Goals

**Goals:**

- Tauri 2 菜单栏图标 + 弹出面板，可刷新与配置
- Cursor 用量：Cookie → usage-summary → 归一化 `UsageSnapshot`；主进度用 `totalPercentUsed`；金额按 cents
- 本机 CPU / 内存实采；GPU 可选
- 可配置目标延迟探测
- Keychain 存取会话；失效时 `needs_auth`
- Provider 接口可扩展，其它源 UI 占位

**Non-Goals:**

- Codex / DeepSeek / 第三方真实取数
- Win/Linux 托盘
- 读取 Cursor 本地 `state.vscdb`（P1）
- 用量事件明细列表与 **token 用量统计**（依赖 `get-filtered-usage-events` 聚合；**另开 change，不在本 MVP**）
- 官方 Team Admin API

## Decisions

### 1. 桌面框架：Tauri 2

- **选择**：Tauri 2 + Rust 后端 + Web 面板 UI
- **理由**：体积与常驻内存优于 Electron；可跨平台；与既有 PaseBoard 经验一致
- **备选**：纯 SwiftUI（绑死 macOS）、Electron（偏重）

### 2. Cursor 取数：Cookie + usage-summary（P0）

- **选择**：用户粘贴 `WorkosCursorSessionToken`；Rust 侧 `GET https://cursor.com/api/usage-summary`
- **理由**：已用 Pro+ 真实响应验证；实现路径最短
- **备选**：本地 `state.vscdb` + `api2.cursor.sh`（体验更好，schema 易变，留 P1）

### 3. 进度与单位语义（按 Pro+ 实测）

- **选择**：
  - `unit = cents`；金额 UI：`cents / 100` → `$x.xx`
  - 主进度条：`percentUsed = totalPercentUsed`
  - 「剩余 %」：`100 - totalPercentUsed`
  - 不单独用 `plan.remaining` 当主剩余（实测可为 0 而 total 仍约 52%）
  - `onDemand.enabled == false` 时不展示 on-demand
- **备选**：仅用 `used/limit` 算百分比（与官网文案不一致，否决）

### 4. 模块边界

```text
src-tauri/
  commands/       # invoke 面
  providers/      # Provider trait + cursor 适配器
  system/         # CPU/内存/GPU
  network/        # 延迟探测
  credentials/    # Keychain
src/              # 弹出面板 UI
```

- 前端不直连 Cursor；全部经 Tauri commands
- 统一 DTO：`UsageSnapshot` / `SystemSnapshot` / `LatencySnapshot` / `PanelState`

### 5. 凭证存储：macOS Keychain

- **选择**：系统钥匙串（如 `keyring` crate 或等价）
- **理由**：符合 Local only；避免明文配置文件
- **备选**：加密本地文件（跨平台迁移时再评估）

### 6. 刷新策略

- Cursor 用量：默认 300s，最小 ≥ 60s
- 系统 / 延迟：10–30s
- 手动刷新始终可用

### 7. 前端栈

- **选择**：轻量 Web（Vite + TypeScript；框架保持简单，优先原生或轻量组件）
- **理由**：面板信息密度高、交互少，避免重型 UI 库
- **备选**：React（若脚手架默认更顺可接受，但不强制复杂状态方案）

## Risks / Trade-offs

- [非官方接口改版] → Provider 隔离 + `parse_error`；保留适配器版本字段；不拖垮系统/延迟卡片
- [Cookie 过期] → `needs_auth` 明确提示；提供清除/重贴入口
- [单位/字段因套餐而异] → 先按 Pro+ 映射；解析失败不崩溃；后续可加探测
- [GPU/温度在 Apple Silicon 上受限] → 允许 null / N/A
- [过度轮询被限流] → 用量默认 5 分钟
- [Cookie 泄露风险] → 禁止日志打印完整 token；仅 HTTPS；Keychain 存储

## Migration Plan

- 新项目：脚手架 → 实现 → 本地 `cargo tauri dev` 验收
- 无旧数据迁移
- 回滚：卸载应用并清除 Keychain 中本应用条目即可

## Open Questions

- 面板前端最终用纯 TS 还是 React：实现时按 Tauri 模板便利性二选一，不阻塞规格
- 延迟探测默认目标（如 `https://cursor.com` vs `1.1.1.1`）：实现时可先默认 HTTPS 到 `cursor.com`，设置可改
- GPU 采集库选型：实现阶段按 macOS 可得性选型，规格只要求「可得则显示，否则 N/A」

## Deferred（已确认另开 change）

- Token 用量统计：用户确认走后续独立 change，不并入 `cursor-first-mvp`（2026-08-05）
