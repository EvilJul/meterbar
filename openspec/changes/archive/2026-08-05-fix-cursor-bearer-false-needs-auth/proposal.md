## Why

本机 Cursor 已登录且能读出 accessToken 时，用量刷新仍偶发误报「会话已失效」。根因是 Bearer 回退路径里 `usage-summary` 对本机 token 常 401，却把 `saw_needs_auth` 置真并升格为 `needs_auth`，掩盖了 Dashboard 主路径的瞬时失败。

## What Changes

- 修正 Bearer 认证失败分类：仅当主路径 Dashboard `GetCurrentPeriodUsage` **自身**返回 401/unauthenticated 才产出 `needs_auth`
- Bearer 调 `usage-summary` 的 401 视为「该端点不接受本机 token」，**不**置 `saw_needs_auth`、不升格为登录失效
- Dashboard 瞬时失败（429/5xx/解析失败等）映射为 `network_error`（可带 HTTP status），不升格为 `needs_auth`
- 合并 `probe` / `read_access_token` 为一次打开 DB，降低 WAL 锁下「有会话却读不出 token」竞态
- 增加诊断日志（HTTP status + 分支名，**不含** token 明文）
- **非目标**：不重做整个 `cursor-local-session` 范围；不改 Cookie 主路径语义；不新增依赖；不改 UI 布局

## Capabilities

### New Capabilities

- （无）

### Modified Capabilities

- `cursor-usage`：Bearer 刷新链路的状态分类——主路径 Dashboard 才可判定会话失效；回退端点 401 与瞬时失败不得伪造成 `needs_auth`
- `cursor-local-session`：单次只读打开 DB 完成探测与 token 读取，减少并发锁竞态

## Impact

- Rust：`src-tauri/src/providers/cursor.rs`（尤其 `fetch_with_bearer`、相关映射与日志）
- Rust：`src-tauri/src/credentials/local_session.rs`（合并 `probe`/`read_access_token` 打开路径）
- 与 `cursor-local-session` 关系：本 change 修复其实现缺陷与未勾任务 5.3 的验收缺口，不重复 local-session 功能范围
- 前端 / Tauri command 签名：无 **BREAKING**；`UsageSnapshot.status` 取值语义更准确
- 依赖：无新第三方 crate
