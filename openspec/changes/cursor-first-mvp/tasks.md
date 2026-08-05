## 1. Scaffold

- [x] 1.1 初始化 Tauri 2 项目（Rust + Vite/TS Web UI），保留现有 `openspec/` 与 `.cursor/`
- [x] 1.2 配置 macOS 菜单栏 / tray 应用形态（无常驻主窗口，点击图标弹出面板）
- [x] 1.3 建立目录骨架：`commands/`、`providers/`、`system/`、`network/`、`credentials/` 与共享 DTO

## 2. Local credentials

- [x] 2.1 实现 Keychain 存取 `WorkosCursorSessionToken`（set / get / clear）
- [x] 2.2 暴露 Tauri commands：`set_cursor_session_token`、`clear_cursor_session_token`
- [x] 2.3 确保日志与 UI 错误信息不输出完整 token；面板 footer 显示 Local only

## 3. Cursor usage provider

- [x] 3.1 实现 `GET https://cursor.com/api/usage-summary` 请求（Cookie 头，后端发起）
- [x] 3.2 按 Pro+ 实测映射 `UsageSnapshot`（`unit=cents`，`percentUsed=totalPercentUsed`，onDemand 按 enabled 显示）
- [x] 3.3 覆盖错误态：`needs_auth` / `parse_error` / `network_error`，且不拖垮其它卡片
- [x] 3.4 实现 `refresh_cursor` 与自动刷新（默认 300s，最小 60s）

## 4. System metrics

- [x] 4.1 采集并返回 CPU 利用率与内存 used/total
- [x] 4.2 尽力采集 GPU；不可用时返回 null / N/A
- [x] 4.3 系统指标独立刷新（10–30s），与 Cursor 失败解耦

## 5. Latency probe

- [x] 5.1 实现可配置目标的延迟探测（默认目标写入设置）
- [x] 5.2 返回 `latencyMs` 与 `ok|timeout|error`；超时/失败不崩溃
- [x] 5.3 超过高延迟阈值时 UI 标红（阈值可配置）

## 6. Panel UI

- [x] 6.1 实现弹出面板布局：标题区、Cursor 卡、System 卡、Latency 卡、footer、设置入口
- [x] 6.2 Cursor 卡：主进度用 `totalPercentUsed`；展示剩余 %、金额（cents→$）、周期结束时间
- [x] 6.3 System / Latency 卡绑定实时数据；Codex / DeepSeek / 第三方为占位
- [x] 6.4 设置页：粘贴/清除 Cookie、延迟目标、刷新间隔；`needs_auth` 时引导配置

## 7. Integration & verification

- [x] 7.1 打通 `get_panel_state` / `refresh_all` 端到端数据流
- [x] 7.2 用真实 Cookie 验收：用量与官网大致一致；会话失效提示正确
- [x] 7.3 `cargo tauri build`（或至少 `tauri dev` 手工验收清单）通过，记录剩余风险
