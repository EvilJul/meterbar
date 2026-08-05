## Why

看板当前固定展示 Cursor / Codex 等卡片（DeepSeek 仅有部分隐藏逻辑），未登录或未配置时仍会出现空进度、needs_auth 或误报态，容易让用户误判为「有额度 / 已连通」。需要在设置中让用户控制各供应商是否显示，并支持看板自定义排序，默认隐藏一切未登录/未配置的供应商。

## What Changes

- 看板默认**不显示**未配置（未登录 / 无密钥 / 不可读额度）的模型供应商卡片，避免假进度与误报
- 设置页为每个模型供应商增加「是否显示」控制（含自动隐藏与用户强制显示/隐藏的策略，见 design）
- 看板支持模型供应商**自定义排序**；顺序持久化到 `settings.json`（非密钥），设置页可上下调整，看板按序渲染
- System / Latency 等系统指标**不纳入**供应商排序；可有独立显示开关（见 design）
- 扩展现有 `AppSettings` / `get_settings` / `update_settings` 持久化路径（依赖 `settings-persistence` 已落地的 `settings.json`）
- **不改**各供应商取数协议本身；验收以 `npm run tauri dev` 为准；不安装到 Applications

## Capabilities

### New Capabilities

- `provider-visibility`: 定义「已配置」判定、看板显示规则、设置中的显示偏好（自动 / 始终显示 / 隐藏）及 System 指标是否单独开关
- `provider-order`: 定义模型供应商排序列表的默认值、校验/归一化、设置页调整与看板按序渲染

### Modified Capabilities

- （无）主规格 `openspec/specs/` 中尚无 settings / menu-bar 的已归档能力需改写；本 change 通过新能力扩展 `AppSettings` 行为。实现时复用 `settings-persistence` 的加载/保存约定，不另起凭证存储。

## Impact

- **后端**：`src-tauri/src/models.rs`（`AppSettings` / `AppSettingsUpdate`）、`settings.rs`（序列化/校验）、可选在 `commands` / `PanelState` 暴露「是否已配置」或由前端用已有 status 判定
- **前端**：`index.html`、`src/main.ts`、`src/styles.css` — 卡片显隐、DOM 重排或 order、设置页开关与排序控件
- **持久化**：`~/Library/Application Support/com.usages.app/settings.json` 新增非密钥字段（如 `providerVisibility`、`providerOrder`、`showSystem`）
- **依赖**：假定 `settings-persistence` 的 load/save 已可用；与仍 active 的 `codex-local-rate-limits` 并行时注意 `AppSettings` 字段冲突需合并
- **非目标**：不改 Cursor/Codex/DeepSeek 取数协议；不引入云同步；不装 Applications
