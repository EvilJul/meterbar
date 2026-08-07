## Why

Meterbar 已覆盖 Cursor / Codex / DeepSeek，但 SuperGrok（grok.com 付费周池）只能去网页 Settings → Usage 查看。Grok Build 开源后，本机可用同一套登录态与 `cli-chat-proxy` billing 接口读出周池已用百分比，适合作为本地托盘面板的第四家提供方。

## What Changes

- 新增 **Grok / SuperGrok** 用量 Provider：本机发现 `grok login` 会话（优先），调用 `GET …/billing?format=credits`，映射为 `UsageSnapshot`
- 主展示：**周池已用 %** + **周期结束/重置时间**；可选 prepaid / on-demand；无绝对次数时不强造 limit
- 设置：可见性 `auto|always|hidden`、排序纳入 `grok`；无会话时 `needs_auth` 与打开 `https://grok.com/?_s=usage` 的引导
- 刷新：与其它 provider 同轮 best-effort，失败不拖垮面板
- README 说明：非公开代理接口、可能变更、与 xAI 开发者 API 账单分离
- **非 BREAKING**：现有三家 provider 行为不变

## Capabilities

### New Capabilities

- `grok-local-usage`：本机 Grok/SuperGrok 会话发现、billing 拉取、UsageSnapshot 映射与错误态

### Modified Capabilities

- `provider-visibility`：模型 provider 集合增加 `grok`
- `provider-order`：默认顺序与 normalize 支持 `grok`
- `local-credentials`：可选发现/存储 Grok 会话材料（不写 settings）；不强制粘贴 token 若本机 CLI 已登录

## Impact

- `src-tauri`：`providers/grok.rs`、credentials 探测、commands 刷新、`models` 可见性/排序
- 前端：Grok 卡片、设置行、拖拽排序四家
- 依赖：HTTPS 至 `cli-chat-proxy.grok.com`（及文档中的默认基址）；本机 `grok` 登录态路径
- 隐私：仅本机；密钥不进 settings；无 Meterbar 云同步
