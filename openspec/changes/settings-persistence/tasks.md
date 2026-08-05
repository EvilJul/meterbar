## 1. Settings IO 模块

- [x] 1.1 新增 `src-tauri/src/settings.rs`（或 `settings/mod.rs`）：路径解析（默认 Application Support、`USAGES_SETTINGS_PATH` 优先，可选 `USAGES_CREDENTIALS_DIR/settings.json`）
- [x] 1.2 实现 `load() -> AppSettings`：缺文件/坏 JSON → 默认值；成功则反序列化后 clamp/normalize
- [x] 1.3 实现 `save(&AppSettings) -> Result<(), String>`：目录 `0700`、文件 `0600`、tmp+rename 原子写；schema 仅 version（可选）+ 四字段，禁止密钥字段
- [x] 1.4 在 `lib.rs`（或等价）注册 `mod settings`

## 2. AppState 与命令接线

- [x] 2.1 将 `AppState` 初始化改为调用 `settings::load()`，替代纯 `AppSettings::default()`
- [x] 2.2 修改 `update_settings`：钳位后 `save`；写盘失败则严格回滚内存并返回中文 `Err`；成功返回持久化后的 `AppSettings`
- [x] 2.3 确认 `get_settings` / `update_settings` 签名与 camelCase DTO 不变；前端无需改 invoke 契约

## 3. 测试与验收

- [x] 3.1 单测（临时路径）：缺文件、坏 JSON、越界 clamp、save/load 往返、写失败回滚
- [x] 3.2 运行 `cargo test`（settings 相关 + 既有不回归）
- [ ] 3.3 手动验收：`npm run tauri dev` 改四字段 → 重启/重开面板 → 值保留；确认未写入 credentials 文件、未安装到 Applications
