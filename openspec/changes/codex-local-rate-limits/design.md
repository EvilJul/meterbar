## Context

菜单栏 MVP 已用 `UsageSnapshot` / `PanelState` 展示 Cursor 用量进度卡；Codex 在 `cursor-first-mvp` 中为 non-functional placeholder（`Unsupported`）。Codex ChatGPT 订阅额度无稳定官方公开 REST；与 CLI 同源的本机 `codex app-server` 提供 `account/rateLimits/read`（字段含 `usedPercent` / `windowDurationMins` / `resetsAt`），可作为本地面板数据源。

实现时须再核对最新官方文档中 Codex App Server / `account/rateLimits/read` 的 URL 与 schema（本 design 不钉死文档链接）。

## Goals / Non-Goals

**Goals:**

- 数据源：本机已登录 ChatGPT 的 Codex（与 CLI / `codex app-server` 同源）
- 读接口：优先 `account/rateLimits/read`，映射 `usedPercent`、`windowDurationMins`、`resetsAt`
- 集成：Rust 侧短生命周期拉起/连接 app-server（JSON-RPC），取一次快照写入 `PanelState.usages`
- UI：Codex 进度卡风格对齐现有 Cursor 卡（进度条 + 重置/窗口信息 + 失败态文案）
- 失败态：未安装 Codex / 未登录 / 协议变更 → `needs_auth` 或明确「本地 Codex 不可用」；禁止静默假数据
- 安全：不落盘 ChatGPT cookie；不把密钥写进 `settings.json`

**Non-Goals:**

- 不以逆向 `chatgpt.com/backend-api` 为默认路径
- 不网页抓 Dashboard
- 不把 Platform Usage/Costs 当成订阅进度条
- 不在本 change 内实现 DeepSeek / 第三方真实取数
- 不 release build / 安装到 Applications；开发验收用 `npm run tauri dev`

## Decisions

### 1. 本机 app-server，而非公开 REST

- **选择**：短生命周期连接本机 Codex app-server，JSON-RPC 调用 `account/rateLimits/read` 取快照后退出/断开。
- **理由**：与用户本机登录态同源；避免 Cookie 落盘与非官方 HTTP 逆向。
- **备选放弃**：直接打 `chatgpt.com`、抓 Dashboard、Platform Costs API——均属非目标。

### 2. 映射到现有 `UsageSnapshot`

- `provider`：`codex`；`displayName`：用户可见名（如 `Codex`）
- `percentUsed` ← `usedPercent`（主进度）
- `periodEnd` / `message` 等可承载 `resetsAt`、窗口时长（`windowDurationMins`）的展示文案；`unit` 可用 `Unknown` 或按实现时字段语义选择，不以 cents/tokens 硬套
- 成功：`status=ok`；未登录/无凭证语义：`needs_auth`；二进制缺失或进程/协议失败：明确失败状态 + 可读 `message`（「本地 Codex 不可用」等），不得填假进度
- 多窗口 rate limit（若响应含多档）：首版至少展示主窗口（实现时按文档选 primary；次要窗口可进 message 或后续扩展）

### 3. 生命周期与刷新

- 刷新时：发现/启动 app-server → JSON-RPC 一次读取 → 映射 → 关闭连接（及按需结束短生命周期子进程）
- 并入现有手动/自动刷新管线（与 Cursor 等同轮或同触发），失败隔离：不影响 System / Latency / Cursor 卡
- 超时：实现时设合理上限（建议数秒级），超时视为本地不可用，不阻塞面板其它区

### 4. UI

- 替换 Codex 占位为进度卡：主条用 `percentUsed`；展示重置时间/窗口信息（有则显示）
- `needs_auth` / 本地不可用：卡片内明确文案，不画假进度
- 不新增设置项存 ChatGPT 密钥；可选仅提示「请登录本机 Codex」

### 5. 安全与配置边界

- 不读取、不持久化 ChatGPT cookie / session 到 Usages 凭证区
- `settings.json` 仅可有非密钥开关/路径覆盖（若需要测试用 env，如 `USAGES_CODEX_*`），禁止密钥字段
- 日志脱敏：不打印 cookie、token、Authorization

## Risks / Trade-offs

- [app-server 官方可能无通知变更方法/字段] → 属尽力而为的本地面板，**非账单权威**；解析失败 → `parse_error` 或「本地 Codex 不可用」，不崩溃、不假数据
- [本机未安装 / PATH 找不到 `codex`] → 明确不可用态，引导安装或登录 CLI
- [已安装但未登录 ChatGPT] → `needs_auth`，提示在 Codex/CLI 完成登录
- [短生命周期拉起开销 / 偶发端口占用] → 超时 + 单次快照；失败隔离；后续可再优化常驻复用（首版不做）
- [与 Cursor 进度语义不完全同构（窗口分钟 vs 账单周期）] → UI 文案标明窗口/重置信息，避免暗示官方账单

## Migration Plan

1. 实现 Rust Codex provider + 接入刷新聚合
2. 前端 Codex 卡绑定真实 `UsageSnapshot`
3. `npm run tauri dev` 验收：已登录 / 未登录 / 未安装三态
4. 回滚：移除 provider 接入、恢复占位即可；无数据迁移、无密钥落盘需清理

## Open Questions

- app-server 精确启动参数、传输（stdio vs socket）与 `account/rateLimits/read` 最新 schema：实现阶段对照官方文档确认
- 多 rate-limit 窗口时首版展示优先级（primary 字段名）：实现时按文档选定并写入注释
- 是否需要 `PanelState` 增加 `has_codex_local` 类布尔（类似 `has_cursor_token`）：非必须；可用 usages 内 Codex 条目 status 表达，实现时按 UI 需要决定
