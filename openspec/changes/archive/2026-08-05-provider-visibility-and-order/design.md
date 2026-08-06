## Context

菜单栏面板（`index.html` / `src/main.ts`）当前固定渲染：

| 卡片 | id | 现状 |
|------|-----|------|
| Cursor | `card-cursor` | 始终显示；未登录呈 `needs_auth` / 空进度 |
| Codex | `card-codex` | 始终显示；未安装/未登录同样占位 |
| DeepSeek | `card-deepseek` | 已有部分隐藏：`hasDeepseekKey` 或 `status !== needs_auth` |
| System / Latency | `card-system` / `card-latency` | 始终显示；属本机指标，非模型供应商 |

配置来源差异：

- **DeepSeek**：设置页 API Key（凭证存储）
- **Cursor**：本机会话 / Cookie（`cursor-local-session`），设置页可配 Cookie
- **Codex**：无密钥设置；依赖本机 Codex app-server 登录态（`codex-local-rate-limits`）
- **System/Latency**：本机采集，不依赖供应商登录

`AppSettings` 已经 `settings-persistence` 落到 `settings.json`（四字段刷新/延迟配置）。本 change 在同一文件扩展**非密钥**偏好：显示模式与排序。

约束：不改取数协议；不装 Applications；验收用 `npm run tauri dev`。

## Goals / Non-Goals

**Goals:**

- 未配置供应商默认不出现在看板，避免假进度 / 误报卡片
- 设置中可控制每个模型供应商的显示策略（自动 / 始终显示 / 隐藏）
- 模型供应商看板顺序可自定义并跨重启保留
- System/Latency 不参与供应商排序；提供独立显示开关
- 复用现有 `get_settings` / `update_settings` + `settings.json`

**Non-Goals:**

- 不改变 Cursor / Codex / DeepSeek 的 HTTP / JSON-RPC 取数协议与字段语义
- 不把 API Key / Cookie / accessToken 写入 `settings.json`
- 不实现拖拽排序库或复杂动画（上下按钮即可）
- 不隐藏「设置页」里的供应商配置区块（设置仍可配置未显示的供应商）
- 不强制在后台停止对隐藏供应商的刷新（可选优化，非本 change 必做）

## Decisions

### 1. 显示策略：每供应商三态 `auto` | `always` | `hidden`

- **选择**：`providerVisibility: Record<ProviderId, "auto" | "always" | "hidden">`（序列化为 camelCase 对象）
- **默认**：全部为 `"auto"`
- **看板显示公式**：
  - `hidden` → **不显示**
  - `always` → **显示**（即使用户未配置；用于主动登录/配 Key）
  - `auto` → **仅当 `is_configured(provider)` 为真时显示**
- **理由**：对齐「未登录/未配置默认不显示」；同时满足「可强制显示/隐藏」。比单纯布尔更不易歧义（「关」是隐藏还是自动？）。
- **备选否决**：
  - 仅 `show: bool`：无法表达「自动隐藏未配置」与「强制显示未配置」并存
  - `show + forceShowUnconfigured` 双布尔：状态组合爆炸，UI 更绕

设置 UI 文案建议（实现可微调）：

| 值 | 控件文案 |
|----|----------|
| `auto` | 自动（已配置才显示）— **默认** |
| `always` | 始终显示 |
| `hidden` | 隐藏 |

可用 segmented control / select；不必三按钮花哨样式。

### 2. `is_configured` 判定（运行时，非持久化）

在 **前端渲染层**（或等价的纯函数，输入为 `PanelState` + 凭证标志）计算；不写入 `settings.json`。

| Provider | `is_configured === true` 当且仅当 |
|----------|-----------------------------------|
| `deepseek` | 已保存 API Key（沿用 `PanelState.hasDeepseekKey` / 等价标志） |
| `cursor` | 本机 session **或** Cookie 凭证可用（与现有 Cursor 认证入口一致：能发起受保护请求，而非「上次快照 status==ok」） |
| `codex` | 本机 Codex 可读额度且非 `needs_auth`：即最近快照 `status === "ok"`，或明确的「已登录」标志；**未安装 / `needs_auth` / 无快照且已知未登录 → 未配置** |

**Cursor 为何不用「status != needs_auth」作为唯一条件：**

- 瞬时网络错误、限流可能短暂非 ok，若据此隐藏会造成卡片闪烁。
- **选定**：以**凭证存在**为准（session 或 Cookie）。若两者皆无 → 未配置 → `auto` 下隐藏。有凭证但拉取失败时仍显示卡片并展示错误（用户可知需处理）。

**Codex 为何用快照/登录态而非「有密钥」：**

- Codex 刻意无密钥设置；「已配置」= 本机已登录且能读 rate limits。
- 冷启动尚无快照时：若 refresh 返回 `needs_auth` / 未安装错误 → 未配置；在首次结果返回前，`auto` 下可暂不显示（或短暂不渲染），避免空壳卡片。

DeepSeek 现有隐藏逻辑并入统一公式，删除「特殊分支」重复语义。

### 3. System / Latency：不算供应商排序；独立开关

- **选择**：
  - `providerOrder` **仅**含模型供应商：`cursor` | `codex` | `deepseek`
  - 新增 `showSystemSection` / `showLatencySection: bool`，默认均 `true`（旧文件仅有前者时两者同值）
  - 为真时显示 `card-system` + `card-latency`（二者作为系统区块一起显隐；不拆两个排序位）
  - 系统区块在 DOM 上固定位于**模型供应商列表之后**（或之前——**选定：模型卡片按 `providerOrder` 排列，System/Latency 固定在模型列表下方**，与现网布局一致）
- **理由**：系统指标不依赖登录；纳入排序会干扰「供应商」心智，且 Latency 依赖 General 设置而非供应商凭证。
- **备选否决**：把 `system` 塞进 `providerOrder`（用户未要求；增加校验复杂度）

### 4. 排序：`providerOrder` 持久化到 `settings.json`

- **字段**：`providerOrder: string[]`，合法 id：`cursor`、`codex`、`deepseek`
- **默认**：`["cursor", "codex", "deepseek"]`（与当前 DOM 顺序一致）
- **归一化（load / update 时）**：
  1. 过滤未知 id
  2. 去重（保留首次出现）
  3. 将缺失的已知 id **按默认顺序**追加到末尾
  4. 结果必须是三个 id 的全排列
- **UI**：设置页「看板排序」区：每个供应商一行 + 上移/下移；变更经 `update_settings` 持久化
- **渲染**：看板容器内按 `providerOrder` 重排模型卡片（`appendChild` 重排或 CSS `order`；**推荐 DOM 重排**以保持无障碍阅读顺序与视觉一致）

### 5. 设置 schema 扩展（非密钥）

在 `AppSettings` / `AppSettingsUpdate` / `settings.rs` 文件 DTO 增加：

```text
providerVisibility: { cursor, codex, deepseek } → "auto" | "always" | "hidden"
providerOrder: ["cursor","codex","deepseek"]  // 归一化后
showSystemSection: bool                       // 默认 true
```

- 缺字段：用默认补齐（兼容旧 `settings.json` 仅四字段）
- 非法枚举：回退该键为 `"auto"`
- **禁止**写入任何密钥
- `update_settings` 仍整对象保存；写失败严格回滚（沿用 settings-persistence）

### 6. 配置态信号如何到达前端

- **DeepSeek**：已有 `hasDeepseekKey`
- **Cursor**：若 `PanelState` 尚无「有凭证」布尔，**本 change 可新增**非密钥标志（如 `hasCursorCredentials: bool`），由后端在组 panel 时根据 session/Cookie 是否存在设置；**不回传 token 内容**
- **Codex**：用 `usages` 中 `provider=="codex"` 的 `status`（及可选 `hasCodexAuth` 若实现更干净）；无快照且未配置时按未配置处理

不新增独立「visibility」Tauri command；显隐纯前端 + settings。

### 7. 设置页 vs 看板

- 设置页**始终**展示各供应商的凭证/说明区块（否则用户无法从「隐藏」恢复配置）
- 每个模型供应商设置组增加「看板显示」三态控件
- 另设「看板排序」区块（可放在 General 附近或独立 group）
- 「显示系统指标」开关放在 General 或 System 相关区

### 8. 与并行 change 的关系

- 依赖 `settings-persistence` 的 load/save 路径（仓库内已有 `settings.rs` 时直接扩展字段）
- 与 `codex-local-rate-limits`：共享 `AppSettings` / 前端卡片时注意合并字段与渲染入口，避免互相覆盖
- 本 change **不**改 Codex 取数；只消费其 `UsageSnapshot.status`

## Risks / Trade-offs

- [Codex 冷启动闪烁] → `auto` 下无快照前不显示；有 `needs_auth` 保持隐藏；仅 `always` 显示登录提示
- [Cursor 有坏 Cookie 仍显示] → 有意为之：显示错误优于假装无此供应商；用户可改 `hidden`
- [用户选 `always` 又看到空卡片] → 文案标明「始终显示（含未配置）」；默认仍是 `auto`
- [并行 change 改 AppSettings 冲突] → apply 前 rebase/合并字段；归一化函数集中一处
- [隐藏后仍后台刷新] → 可接受的电量/隐私折中；后续可加「隐藏则跳过 refresh」优化
- [仅改 CSS order 导致 Tab 顺序错乱] → 采用 DOM 重排

## Migration Plan

1. 扩展 `AppSettings` 默认值与 clamp/normalize；`settings` 序列化兼容缺字段
2. 前端统一 `shouldShowProvider` + 按 `providerOrder` 重排；System 用 `showSystemSection`
3. 设置页加三态与排序控件，走现有 `update_settings`
4. 单测：归一化、非法枚举、旧文件缺字段；手动 `npm run tauri dev`：未配置隐藏、always 可见、排序持久化
5. 回滚：删除新字段或整文件 → 默认 `auto` + 默认顺序 + 显示系统区
6. **禁止** ditto 安装到 `/Applications`

## Open Questions

- Cursor「有凭证」是否已在 `PanelState` 暴露：若无，apply 时新增 `hasCursorCredentials`（待实现确认现有字段）
- Codex「未安装」与 `needs_auth` 文案是否需在 `always` 模式下区分：建议区分，但不阻塞本 change
- 是否提供「一键恢复默认排序」：非必须，可留后续
