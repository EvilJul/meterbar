## 1. 文档与探测

- [x] 1.1 核对最新 Codex App Server 文档：`account/rateLimits/read` 的传输方式（stdio/socket）、启动参数与响应字段（`usedPercent` / `windowDurationMins` / `resetsAt`），在实现注释中记录核对日期与文档入口
- [x] 1.2 确认本机 `codex` / app-server 可发现路径策略（PATH、常见安装位置）；缺失时的错误分类（未安装 vs 未登录 vs 超时）

## 2. Rust：app-server 客户端与映射

- [x] 2.1 新增 Codex provider 模块：短生命周期拉起/连接 app-server（JSON-RPC），调用优先 `account/rateLimits/read`，设超时，用毕断开/清理子进程
- [x] 2.2 将响应映射为 `UsageSnapshot`（`provider=codex`，`percentUsed`←`usedPercent`，重置/窗口信息进可展示字段或 `message`）；成功 `ok`，未登录 `needs_auth`，不可达/协议失败为明确失败态 + 文案
- [x] 2.3 禁止静默假数据：失败路径不得写入看起来像成功的假百分比
- [x] 2.4 安全：不落盘 ChatGPT cookie；不向 `settings.json` 写入密钥；日志脱敏

## 3. 接入 PanelState 刷新

- [x] 3.1 在用量刷新聚合中纳入 Codex 快照，写入 `PanelState.usages`
- [x] 3.2 失败隔离：Codex 失败不影响 Cursor / System / Latency
- [x] 3.3 手动刷新与自动刷新均会 best-effort 尝试 Codex（与 design 一致）

## 4. 前端 Codex 进度卡

- [x] 4.1 将 Codex 非功能占位替换为与 Cursor 类似的进度卡（主进度 `percentUsed`，有则显示重置/窗口信息）
- [x] 4.2 `needs_auth` / 本地不可用态：明确文案，不渲染假成功进度条
- [x] 4.3 DeepSeek / 第三方保持占位（本 change 不接真实取数）

## 5. 验收

- [ ] 5.1 `npm run tauri dev`：本机已登录 Codex → 进度卡有真实 `usedPercent`（或等价展示）
- [ ] 5.2 未登录 / 未安装 / 人为破坏协议路径 → 明确失败或 `needs_auth`，无假数据
- [x] 5.3 确认未写入 ChatGPT cookie/密钥到凭证文件或 `settings.json`
- [x] 5.4 **禁止** release build / 安装到 Applications（除非用户另授权）
