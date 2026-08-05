## Context

MVP 仅用 Cookie 调 `cursor.com/api/usage-summary`。社区实现（ClearMeasureLabs/Cursor-Usage-Status）从 `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb` 读取 `cursorAuth/accessToken`，以 Bearer 调 `api2.cursor.sh` 与 `cursor.com` 用量接口。

## Goals / Non-Goals

**Goals:**

- macOS 优先：只读打开 `state.vscdb`，提取非空 `cursorAuth/accessToken`
- Bearer 顺序：`GET cursor.com/api/usage-summary` → `GET api2.cursor.sh/api/usage/summary` → `POST api2.../GetCurrentPeriodUsage`（能映射则映射，否则跳过）
- 本地失败 → 现有 Cookie + usage-summary 路径不变
- 解析/SQLite/网络失败均优雅降级，不 panic、不日志 token

**Non-Goals:**

- 写回或修改 Cursor 数据库
- Win/Linux 路径（可留 stub）
- Token 事件统计
- 自动 refresh token

## Decisions

### 1. SQLite 只读 + 文件锁

- `rusqlite` + `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`
- 路径不可读/被锁/无 key → 返回 `None`，走 Cookie fallback
- 测试可设 `USAGES_CURSOR_STATE_DB` 覆盖路径

### 2. 刷新顺序

```text
read_access_token()
  → fetch_usage_summary_bearer (cursor.com, then api2)
  → fetch_get_current_period_usage_bearer (映射 cents/percent 若字段齐全)
get_token() cookie
  → fetch_usage_summary_cookie (现有逻辑)
needs_auth
```

### 3. 映射

- usage-summary 响应仍走现有 `map_ok`（auto/api/total percent、cents）
- GetCurrentPeriodUsage 的 `planUsage` 仅作兜底：填 used/limit/percentUsed，auto/api 缺则留空

### 4. UI / PanelState

- `hasCursorToken` 改名为语义「有可用凭证」：本机会话 **或** 已保存 Cookie
- 设置文案：优先「已登录 Cursor 即可」；Cookie 可选

## Risks

- Cursor 改 key/schema → 降级 Cookie，不崩溃
- GetCurrentPeriodUsage 无 auto/api 百分比 → 主卡可能只显示部分条
- 本机未登录 Cursor → 行为与 MVP 一致（需 Cookie）
