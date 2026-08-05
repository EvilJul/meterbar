## 1. Settings 模型扩展

- [x] 1.1 在 `AppSettings` / `AppSettingsUpdate` 增加 `providerVisibility`、`providerOrder`、`showSystemSection`（camelCase），并定义默认值（三态全 `auto`；顺序 `cursor,codex,deepseek`；系统区 `true`）
- [x] 1.2 实现 normalize：非法 visibility → `auto`；`providerOrder` 过滤/去重/补齐为三供应商全排列
- [x] 1.3 扩展 `settings.rs` 序列化与缺字段兼容；确认不写入任何密钥；必要时补单测（缺字段、非法枚举、乱序归一化）

## 2. 配置态信号

- [x] 2.1 确认或新增前端可用的非密钥标志：`hasDeepseekKey`（已有）、`hasCursorCredentials`（若缺失则在 `PanelState` 暴露）、Codex 用 snapshot `status` / 等价已登录判定
- [x] 2.2 实现统一的 `is_configured(provider)`（按 design：DeepSeek=有 Key；Cursor=session 或 Cookie；Codex=可读额度且非 needs_auth/未安装）

## 3. 看板显隐与排序

- [x] 3.1 实现 `shouldShowProvider(mode, configured)`：`hidden` 永不显示；`always` 显示；`auto` 仅 configured 显示
- [x] 3.2 用统一规则替换 DeepSeek 特殊隐藏逻辑；Cursor/Codex 在 `auto` 且未配置时默认隐藏
- [x] 3.3 按 `providerOrder` DOM 重排模型卡片；隐藏项跳过且保持相对顺序；System/Latency 固定在模型列表下方
- [x] 3.4 按 `showSystemSection` 同时显隐 System 与 Latency 卡片

## 4. 设置页 UI

- [x] 4.1 为 Cursor / Codex / DeepSeek 设置组增加看板显示三态控件（自动 / 始终显示 / 隐藏），保存走 `update_settings`
- [x] 4.2 增加看板排序区（上移/下移），变更持久化到 `providerOrder`
- [x] 4.3 增加「显示系统指标」开关绑定 `showSystemSection`；设置页供应商配置区在卡片隐藏时仍可见

## 5. 验收

- [x] 5.1 `cargo test`（settings 归一化相关）通过
- [ ] 5.2 `npm run tauri dev` 手动验收：未配置默认不显示；`always` 可强制显示；`hidden` 强制隐藏；排序与系统开关跨重启保留
- [x] 5.3 确认未安装到 `/Applications`；未改取数协议
