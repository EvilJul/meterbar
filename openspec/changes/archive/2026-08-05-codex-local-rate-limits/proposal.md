## Why

MVP 面板中 Codex 仅为非功能占位；用户已在本机登录 ChatGPT/Codex 时，仍无法看到订阅额度进度。应通过本机 Codex app-server 读取 rate limits，做成与 Cursor 类似的进度卡，而不是抓网页或逆向非公开 REST。

## What Changes

- 新增本机 Codex 额度快照：经短生命周期连接 `codex app-server`（JSON-RPC），优先调用 `account/rateLimits/read`，映射 `usedPercent` / `windowDurationMins` / `resetsAt` 写入 `PanelState`
- 面板 Codex 卡从非功能占位升级为真实进度卡（失败态显式：`needs_auth` 或「本地 Codex 不可用」）
- 不落盘 ChatGPT cookie；密钥不写入 `settings.json`
- **非目标（首版）**：不以 `chatgpt.com/backend-api` 为默认；不网页抓 Dashboard；不把 Platform Usage/Costs 当成订阅进度条；不 release/安装到 Applications

## Capabilities

### New Capabilities

- `codex-local-rate-limits`：经本机 Codex app-server 读取账户 rate limits，映射为 `UsageSnapshot` 并参与面板刷新

### Modified Capabilities

- `menu-bar-shell`：Codex 条目从 non-functional placeholder 改为展示本地 Codex 额度进度（或明确失败态）；DeepSeek / 第三方仍可为占位

## Impact

- `src-tauri`：新增 Codex provider（短生命周期拉起/连接 app-server、JSON-RPC 调用、映射 DTO）
- `PanelState.usages` 中 Codex 条目由 `Unsupported` 占位改为真实 `UsageSnapshot`
- 前端菜单栏面板：Codex 卡绑定进度条与失败文案（对齐 Cursor 卡交互风格）
- 依赖本机已安装且可登录的 Codex（与 CLI / app-server 同源）；无官方公开 REST 保证，属尽力而为
- 开发验收：`npm run tauri dev`；不安装到 Applications
