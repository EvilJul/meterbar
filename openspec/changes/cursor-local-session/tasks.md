## 1. OpenSpec & deps

- [x] 1.1 确认 change artifacts 就绪（proposal / design / specs / tasks）
- [x] 1.2 添加 `rusqlite` 依赖（只读 SQLite）

## 2. Local session reader

- [x] 2.1 实现 `credentials/local_session.rs`：只读打开 `state.vscdb`，读取 `cursorAuth/accessToken`
- [x] 2.2 支持 `USAGES_CURSOR_STATE_DB` 覆盖路径；失败返回 `None`，不 panic
- [x] 2.3 单测：mock/临时 db 或缺失文件降级

## 3. Cursor provider refresh chain

- [x] 3.1 Bearer 调 `cursor.com/api/usage-summary`，复用现有 `map_ok`
- [x] 3.2 失败则 Bearer 调 `api2.cursor.sh/api/usage/summary`
- [x] 3.3 可选：Bearer 调 `GetCurrentPeriodUsage` 并映射 cents（字段不足则跳过）
- [x] 3.4 本地全失败 → 现有 Cookie usage-summary；两者皆失败 → `needs_auth`
- [x] 3.5 确保日志/错误信息不含 token

## 4. Panel & settings

- [x] 4.1 `hasCursorToken` 扩展为本机会话或 Cookie 任一可用
- [x] 4.2 更新设置页与 `needs_auth` 文案（本机登录优先，Cookie 兜底）
- [x] 4.3 不破坏失焦收起 / 毛玻璃 UI

## 5. Verification

- [x] 5.1 `cargo test` / `cargo check` / `npm run build`
- [x] 5.2 尽量 `tauri build` 并 ditto 到 `/Applications/Usages.app`
- [ ] 5.3 本机已登录 Cursor 时无 Cookie 也能出用量（或说明如何测）
