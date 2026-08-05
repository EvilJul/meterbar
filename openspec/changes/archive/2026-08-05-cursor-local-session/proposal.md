## Why

用户已登录 Cursor 时，仍须手动粘贴浏览器 Cookie 才能查看用量，体验差且易过期。应优先读取本机 Cursor 登录态（`state.vscdb`），失败再退回已保存 Cookie。

## What Changes

- 新增本机 Cursor SQLite 只读读取：`cursorAuth/accessToken`
- 刷新用量顺序：本机 accessToken → Bearer 调 usage-summary / api2 内部接口 → 已保存 Cookie → `needs_auth`
- 设置页文案：说明自动读取本机会话；Cookie 为可选兜底
- 保留现有 Cookie 双写与回读校验；token 不打日志

## Capabilities

### New Capabilities

- `cursor-local-session`：从 Cursor `state.vscdb` 只读提取 accessToken，并作为 Bearer 凭证拉取用量

### Modified Capabilities

- `cursor-usage`：认证来源扩展为本机会话优先，Cookie 为 fallback；`needs_auth` 引导文案更新
- `local-credentials`：`has_cursor_token` 语义扩展为「本机会话或已保存 Cookie 任一可用」

## Impact

- `src-tauri/src/credentials/` 新增 local session 模块
- `src-tauri/src/providers/cursor.rs` 刷新链路调整
- 新增依赖 `rusqlite`（只读 SQLite）
- 设置页 HTML/TS 文案小改；不改动失焦收起与毛玻璃 UI
