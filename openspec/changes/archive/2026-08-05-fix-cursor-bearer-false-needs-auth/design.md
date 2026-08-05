## Context

`cursor-local-session` 已实现：从 `state.vscdb` 读 `cursorAuth/accessToken`，Bearer 拉用量，失败再 Cookie。实测主路径应为 Dashboard `GetCurrentPeriodUsage`（本机 token 可用），`usage-summary` 对本机 token **常 401**。

当前 `fetch_with_bearer` 在 Dashboard 非成功后仍回退 Bearer `usage-summary`；该 401 会置 `saw_needs_auth=true`，最终误报「会话已失效」。Dashboard 瞬时 429/5xx/解析失败也被同一逻辑掩盖。另：`probe` 与 `read_access_token` 分别打开 DB，WAL 锁下可出现「探测有会话、读 token 失败」竞态。

与 `cursor-local-session` 关系：修实现缺陷与验收缺口（尤其未勾 5.3），不扩大 local-session 功能范围。

## Goals / Non-Goals

**Goals:**

- Bearer 路径仅在 Dashboard **自身** 401/unauthenticated 时产出 `needs_auth`
- Bearer `usage-summary` 401 → 视为端点不支持该 token，不置 `saw_needs_auth`
- Dashboard 瞬时失败 → `network_error`（可带 HTTP status），不升格登录失效
- 一次打开 DB 完成探测与 token 读取
- 诊断日志含各端点 HTTP status + 分支名，不含 token 明文
- 无 Cookie、Cursor 已登录时连续刷新不再出现假 `needs_auth`

**Non-Goals:**

- 重做 Cookie 路径或设置页 UI
- 改变凭证存储（Keychain / fallback）
- 新增第三方依赖或新 Tauri command
- 归档/替换整个 `cursor-local-session` change
- release 安装到 Applications（本 change 规划与实现阶段均禁止擅自安装）

## Decisions

### 1. Bearer 失败分类以 Dashboard 为主判据

```text
POST GetCurrentPeriodUsage (Bearer)
  → Ok 可映射        → return ok
  → 401 / unauth     → NeedsAuth（可立即返回，不必再靠 usage-summary 确认）
  → 429 / 5xx / 解析失败 / 非成功且非 auth
                     → 记录诊断；可尝试 usage-summary 仅作「若 Ok 则用」；
                       若回退也失败 → NetworkError（带 status），绝不因回退 401 变 NeedsAuth
GET usage-summary (Bearer, 可选回退)
  → Ok               → return ok
  → 401              → 忽略（不置 saw_needs_auth），继续下一 URL 或收尾
  → 其他可映射错误   → 按现有非 auth 状态返回或继续
```

**替代方案**：完全移除 usage-summary Bearer 回退。  
**未采用原因**：保留「若某环境 token 对 usage-summary 有效」的成功路径；只修正错误分类即可。

### 2. `saw_needs_auth` 仅绑定 Dashboard

删除（或不再使用）「任一端点 NeedsAuth 即累计」的语义。usage-summary Bearer 的 NeedsAuth **不得**驱动最终「会话已失效」。

**替代方案**：usage-summary 401 时直接跳过 Cookie。  
**未采用原因**：假阳性时仍应尝试已保存 Cookie；真会话失效时 Dashboard 已会 NeedsAuth。

### 3. 一次打开 DB：`probe_and_read`（或等价）

将「路径解析 → 只读打开 → token 长度/内容 → probe 字段」合并为单次连接（每候选路径至多打开一次）。`refresh` 侧避免先 `probe()` 再 `read_access_token()` 双重打开。

公开 API 可保留 `probe` / `read_access_token` 薄封装（内部共享合并实现），以降低调用方改动面。

**替代方案**：仅加重试。  
**未采用原因**：治标不治本；一次打开同时降低锁竞争与 IO。

### 4. 诊断日志

在 Bearer 各分支记录：`endpoint`（或短名）、`http_status`、`branch`（如 `dashboard_ok` / `dashboard_needs_auth` / `usage_summary_unsupported_token` / `dashboard_transient`）。禁止记录 Authorization 头或 token 子串。

### 5. 与 Cookie 路径边界

Cookie `fetch_with_cookie` 的 401 → `needs_auth` 语义保持不变。本 change 只约束 Bearer/`fetch_with_bearer` 及 local session 打开路径。

## Risks / Trade-offs

- [Dashboard 真 401 但 usage-summary 偶发 Ok] → 若先判 Dashboard NeedsAuth 并立即返回，可能少一次成功机会 → 可接受：本机 token 对 Dashboard 才是主路径；真失效应引导重新登录/Cookie
- [去掉 usage-summary 401 升格后，全部失败且无 Cookie 时可能落到 `network_error` 而非 `needs_auth`] → 符合「勿伪造成登录失效」；无凭证场景仍由 refresh 外层（无 token）给出 `needs_auth`
- [合并打开改变 probe 行为] → 单测需覆盖临时 db / 缺失文件；保持失败码语义（`cursor_db_not_found` 等）
- [Cursor 再改 API] → 诊断日志便于区分；仍优雅降级

## Migration Plan

- 纯行为修复，无数据迁移、无配置格式变更
- 回滚：恢复 `fetch_with_bearer` 累计 `saw_needs_auth` 与双次打开即可（不推荐）

## Open Questions

- Dashboard 返回 NeedsAuth 时是否仍尝试 Cookie fallback（当前 refresh 外层在 Bearer NeedsAuth 时的行为）：建议 **保留** Cookie 尝试，与「本机 token 失效但 Cookie 仍可用」一致——实现时对照现有 `refresh` 分支，不扩大为新产品决策
- 诊断日志级别（`debug` vs `info`）：默认 `debug`/`tracing` 已有惯例优先；若项目无统一 tracing，用现有 `eprintln!`/`log` 风格对齐，避免刷屏
