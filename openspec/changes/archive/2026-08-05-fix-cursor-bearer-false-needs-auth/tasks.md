## 1. Bearer 状态分类修正

- [x] 1.1 修改 `fetch_with_bearer`：仅当 Dashboard `GetCurrentPeriodUsage` **自身** 401/unauthenticated 时产出 `NeedsAuth`；移除「任一端点 NeedsAuth 即累计 `saw_needs_auth`」语义
- [x] 1.2 Bearer `usage-summary`（cursor.com / api2）返回 401 时视为「端点不支持该 token」，**不要**置 `saw_needs_auth`，可继续尝试下一 URL 或收尾
- [x] 1.3 Dashboard 瞬时失败（429/5xx/解析失败等）映射为 `network_error`（消息可带 HTTP status），不得升格为「会话已失效」/`needs_auth`
- [x] 1.4 确认 `refresh` 外层：Bearer 真 `NeedsAuth` 时仍可尝试 Cookie；无 Cookie 且仅瞬时/回退失败时返回 `network_error` 或既有 local-session API 失败文案，而非假 `needs_auth`

## 2. 一次打开 DB

- [x] 2.1 在 `local_session.rs` 合并 `probe` / `read_access_token`：每候选路径单次只读打开，同时得到 probe 字段与 token（可新增 `probe_and_read`，薄封装保留旧 API）
- [x] 2.2 更新 `refresh`：避免先 `probe()` 再 `read_access_token()` 双重打开；保留 `cursor_db_not_found` / `cursor_db_not_openable` / `cursor_token_missing` 等失败码语义
- [x] 2.3 补充/调整 `local_session` 单测：临时 db、缺失文件、单次打开路径行为

## 3. 诊断日志

- [x] 3.1 为 Dashboard / usage-summary Bearer 各分支记录诊断日志：HTTP status + 分支名（如 `dashboard_ok` / `dashboard_needs_auth` / `usage_summary_unsupported_token` / `dashboard_transient`）
- [x] 3.2 确认日志与错误消息不含 token / Authorization / Cookie 明文

## 4. 验证

- [x] 4.1 `cargo test`（含 `local_session` / cursor 相关单测）与 `cargo check` 通过
- [ ] 4.2 手动验收：无 Cookie、Cursor 本机已登录时连续刷新，不应再出现假 `needs_auth`；瞬时/网络失败应表现为 `network_error`（可衔接 `cursor-local-session` 未勾任务 5.3）— **待用户** `npm run tauri dev` 验收
- [x] 4.3 **禁止**本任务擅自 `tauri build` release 或 ditto 安装到 `/Applications/Usages.app`（仅 `tauri dev` 或用户明确授权后再安装）
