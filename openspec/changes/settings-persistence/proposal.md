## Why

`AppSettings`（刷新间隔、延迟目标、高延迟阈值）目前仅存在于进程内存；重启后回退默认值，设置页改动无法保留。凭证已走 Keychain/本地 fallback，用户期望非敏感设置同样跨会话生效，且与凭证存储严格分离。

## What Changes

- 将 `AppSettings` 四个字段持久化到本机 JSON 配置文件（与 credentials 目录同属 `com.usages.app`，但独立文件）
- 应用启动 / `AppState` 初始化时加载；`update_settings` 成功钳位后写盘
- 缺失、损坏或非法文件时回退默认值并安全覆盖，不阻断启动
- 前端 `get_settings` / `update_settings` 调用面保持不变（无 **BREAKING**）
- **非目标**：不把 API Key / Cookie / accessToken 写入设置文件；不持久化快照缓存；不改 UI 交互形态（除重启后值应保留）

## Capabilities

### New Capabilities

- `settings-persistence`：非敏感应用设置的本机加载、保存、校验与坏文件恢复

### Modified Capabilities

- （无；已归档 main specs 中暂无独立「设置持久化」能力；本 change 以新 capability 覆盖行为）

## Impact

- Rust：`AppState` 初始化、`update_settings` 写盘；可能新增小型 settings 模块（路径/读写）
- 存储：`~/Library/Application Support/com.usages.app/settings.json`（或等价，见 design）；权限对齐现有 credentials 目录惯例（目录 `0700`，文件 `0600`）
- 前端 / Tauri command 签名：保持 `get_settings` / `update_settings` + `AppSettings` / `AppSettingsUpdate` camelCase 不变
- 依赖：无新第三方 crate 需求（标准库 `fs` + 现有 `serde`/`serde_json`）
- 与 `cursor-first-mvp` / `local-credentials` 边界：凭证仍只走 Keychain + token fallback 文件；设置文件禁止写入任何密钥字段
