## Context

当前 `AppSettings`（见 `src-tauri/src/models.rs`）注释已标明「内存态；完整持久化留给后续任务」。`AppState::default()` 以 `AppSettings::default()` 初始化；`get_settings` / `update_settings` 仅读写 `Mutex<AppSettings>`，进程退出后丢失。

字段与默认值 / 钳位（实现须对齐现有逻辑）：

| 字段 | 默认 | 约束 |
|------|------|------|
| `cursorRefreshSec` | 300 | ≥ 60 |
| `systemRefreshSec` | 15 | 10–30 |
| `latencyTarget` | `https://cursor.com` | trim；空则默认；无 scheme 则补 `https://` |
| `highLatencyMs` | 500 | ≥ 1 |

凭证已存于 `~/Library/Application Support/com.usages.app/`（Keychain + `cursor_session_token` / `deepseek_api_key` fallback，目录 `0700`、文件 `0600`、原子写）。设置持久化应复用该目录惯例，但**独立文件、绝不混入密钥**。

前端（`index.html` / `src/main.ts`）已通过 `get_settings` / `update_settings` 编辑上述四字段；UI 形态无需为本 change 重做。

## Goals / Non-Goals

**Goals:**

- 四字段跨重启保留；加载与保存时机明确、可测
- 存储与 credentials 分离；坏文件不阻断启动
- 保持现有 Tauri command 签名与 camelCase DTO，前端零改或仅验收

**Non-Goals:**

- 不把 Cookie / API Key / Cursor accessToken 写入设置文件
- 不持久化 `last_usages` / `last_system` / `last_latency` 快照
- 不引入云同步、iCloud、用户可选路径 UI
- 不改刷新调度算法本身（仅保证启动后间隔来自磁盘）
- 不新增第三方依赖；不做跨平台路径抽象超出当前 macOS MVP 需要
- 前端 `localStorage`（如设置分组折叠键）不在本 change 范围

## Decisions

### 1. 存储介质：JSON 文件（非 Keychain / 非 UserDefaults）

- **选择**：`settings.json`，serde 序列化与 `AppSettings` 同构（camelCase）
- **路径**：`~/Library/Application Support/com.usages.app/settings.json`
- **可选覆盖**：环境变量 `USAGES_SETTINGS_PATH`（单测/隔离）；若未设则与 credentials 共用目录根（可复用 `USAGES_CREDENTIALS_DIR` 为父目录，文件名固定 `settings.json`，或独立 `USAGES_SETTINGS_PATH` 指向完整文件路径——**推荐完整路径变量**，避免与凭证文件名耦合）
- **理由**：非敏感、可读可 diff、与现有 `serde` 一致；Keychain 不适合多字段配置
- **备选否决**：macOS UserDefaults（跨语言不直观、难单测）；SQLite（过重）

### 2. 与 credentials 分离

- 同目录不同文件；禁止在 `settings.json` 增加任何密钥字段
- 写盘模块不得调用 `credentials::set_*` / 读取 token 内容
- 文件权限：父目录 `0700`，`settings.json` `0600`；写入采用 tmp + rename（对齐 credentials fallback）

### 3. 加载时机：`AppState` 初始化（启动）

- 在构造 `AppState`（或 `AppState::load()` / `Default` 替换为显式加载）时：
  1. 若文件不存在 → 使用 `AppSettings::default()`，**不强制立刻写盘**（首次 `update_settings` 或可选「首次启动懒写入」；推荐懒写入以减少噪声）
  2. 若存在 → 读入 JSON → 反序列化 → 对每个字段应用现有 clamp / normalize
  3. 读失败 / JSON 非法 / 类型错误 → 记录 warn（无密钥内容）→ 使用默认值；可选将坏文件备份为 `settings.json.bak` 再继续
- 内存态仍是运行时权威；磁盘是冷启动来源与 `update_settings` 后的镜像

### 4. 保存时机：`update_settings` 成功更新内存后

- 钳位 / normalize 完成后写盘完整 `AppSettings`（非 partial patch 文件）
- 写盘失败：返回 `Err`（中文错误信息），**是否回滚内存**二选一：
  - **推荐**：内存已更新则保留，并返回错误提示「已应用但未能保存到磁盘」——避免用户以为没改成功又丢内存改动；或
  - **严格**：写盘失败则回滚内存到写前快照并 `Err`
  - **本设计采用严格**：写盘失败则回滚内存，保证「成功响应 ⟺ 已持久化」
- 不在每次 `get_settings` / 刷新路径写盘

### 5. Schema / 迁移

- 文件可含可选 `version: 1`（推荐）；缺省视为 v1
- 未知字段：`serde` 默认 ignore（`deny_unknown_fields` **关闭**），便于前向兼容
- 缺字段：反序列化后用 `Default` 补齐再 clamp
- 未来加字段：bump `version` 或仅靠缺省补齐；本 change 无旧格式需迁移
- 非法数值（如 `systemRefreshSec: 999`）：clamp 到合法范围后使用；下次成功保存写回钳位后的值

### 6. API 面：尽量不破

- 保持：
  - `get_settings() -> AppSettings`
  - `update_settings(patch: AppSettingsUpdate) -> AppSettings`
- 不新增前端必调命令；可选内部 helper，不暴露
- `PanelState` 中的间隔字段继续从内存 `AppSettings` 投影，无需改协议

### 7. 模块边界

- **推荐**新增 `src-tauri/src/settings.rs`（或 `settings/mod.rs`）：`load() -> AppSettings`、`save(&AppSettings) -> Result<(), String>`、`settings_path()`
- `commands::AppState` 初始化调用 `settings::load()`；`update_settings` 调用 `settings::save`
- `models::AppSettings` 钳位逻辑复用，不在 IO 层复制规则

## Risks / Trade-offs

- [磁盘满 / 权限失败导致无法保存] → 严格回滚 + 错误文案；用户可重试
- [并发 update] → 现有单 `Mutex`；save 在锁内或持克隆后锁外写（锁外写时注意最后写者胜；MVP 可锁内写，文件小）
- [误把密钥写进 settings] → code review + spec 禁止；文件 schema 仅四字段 + version
- [与 credentials 目录权限不一致] → 复用同一 `set_mode` 惯例
- [测试污染真实 Application Support] → 强制 `USAGES_SETTINGS_PATH` 指向临时文件

## Migration Plan

1. 实现 load/save + 接线 `AppState` / `update_settings`
2. 单测：缺文件、坏 JSON、越界值 clamp、保存往返、写失败回滚（可用临时路径）
3. `cargo test` + 手动：改设置 → 重启（或重新 `tauri dev`）→ 值仍在
4. 回滚：删除 `settings.json` 即恢复默认；不影响 Keychain / token 文件
5. **禁止**本 change 流程中 `ditto` 安装到 `/Applications`

## Open Questions

- 写盘失败策略：本设计选「严格回滚」；若产品更希望「内存保留 + 警告」，可在 apply 前改口
- `USAGES_SETTINGS_PATH` vs 复用 `USAGES_CREDENTIALS_DIR`：推荐独立完整路径变量；实现时可两者都支持（PATH 优先，否则 `CREDENTIALS_DIR/settings.json`，再否则默认 Application Support）
- 是否在首次成功 load 缺文件时立即写默认文件：推荐否（懒写入）
