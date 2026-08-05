## Why

需要在 macOS 菜单栏常驻查看 AI 用量与本机状态，避免反复打开各官网。个人 Cursor 账号无官方用量 API，需用本地会话拉取 dashboard 摘要；同时一并展示 CPU/内存与网络延迟，形成第一期可用的桌面监控面板。

## What Changes

- 新建 Tauri 2 macOS 应用：菜单栏图标 + 弹出面板
- 接入 Cursor 个人用量（Cookie → `GET /api/usage-summary`），归一化展示已用/限额/进度/周期
- 采集本机 CPU、内存；GPU 尽力而为（不可得则 N/A）
- 对可配置目标做延迟探测，高延迟可标红
- 会话凭证写入本机 Keychain；面板标注 Local only
- Codex / DeepSeek / 第三方用量仅 UI 占位，本 change 不接真实取数
- Token 用量统计（周期合计 / 按模型等）不在本 change，后续单独 change（事件接口聚合）
- 自动刷新：用量默认约 5 分钟；系统/延迟更短间隔

## Capabilities

### New Capabilities

- `menu-bar-shell`：菜单栏图标、弹出面板生命周期、刷新入口与设置入口
- `cursor-usage`：Cursor 个人会话认证、usage-summary 拉取与 UsageSnapshot 映射/错误态
- `system-metrics`：CPU / 内存 / GPU（可选）本机指标采集
- `latency-probe`：可配置目标延迟探测与状态
- `local-credentials`：Cursor 会话 Cookie 的本机安全存取与清除

### Modified Capabilities

- （无；仓库尚无已归档 specs）

## Impact

- 代码库从空项目脚手架为 Tauri 2（Rust + Web UI）
- 新增对 `cursor.com` 非官方 dashboard 接口的依赖（易碎，需 Provider 隔离）
- 使用 macOS Keychain 存储会话；无云端同步
- 后续 Win/Linux 与其它 AI Provider 可复用同一面板与 Provider 接口，但不在本 change 范围
