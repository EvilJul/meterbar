## Why

Meterbar 目前仅面向 macOS 菜单栏；默认路径、托盘壳与部分系统采集写死 macOS。用户需要在 Linux 上同样使用本机 AI 用量面板，同时不破坏现有 macOS 行为。

## What Changes

- 正式支持 **macOS + Linux** 双目标（Windows 仍非目标）
- 平台路径：设置 / 凭证 / Cursor `state.vscdb` 在 Linux 走 XDG / Cursor 默认配置目录
- 托盘壳：Linux 用标准系统托盘 + tao 定位；非 template 图标；毛玻璃/Accessory 仅 macOS
- 凭证：Linux 用 Secret Service（可用时）+ 既有 `0600` 文件 fallback
- 系统指标：Linux 上 GPU 可降级为 N/A；磁盘/VPN 用现有跨平台逻辑并小幅补全
- README / 环境要求：补充 Linux 构建依赖与路径说明
- **非 BREAKING**：macOS 路径、Keychain、菜单栏行为保持兼容

## Capabilities

### New Capabilities

- `platform-paths`：各 OS 下 App 数据目录、凭证目录、Cursor DB 候选路径的解析约定

### Modified Capabilities

- `menu-bar-shell`：从「仅 macOS 菜单栏」扩展为「macOS 菜单栏 + Linux 系统托盘」面板壳
- `cursor-local-session`：增加 Linux Cursor `state.vscdb` 候选路径
- `local-credentials`：默认存储位置平台化；密钥环后端按平台选择
- `settings-persistence`：默认 `settings.json` 路径平台化
- `system-metrics`：明确 Linux 上 GPU 可选/N/A；主盘挂载 Linux 默认 `/`

## Impact

- `src-tauri`：`credentials/*`、`settings.rs`、`system/mod.rs`、`lib.rs` 托盘/窗体 cfg
- `Cargo.toml` / `tauri.conf.json`：macOS-only features 隔离；Linux 构建依赖文档
- 前端：必要时 Linux 下面板背景不透明降级（CSS / 无 hud 时）
- 依赖：`keyring` Linux secret-service；Tauri 需 WebKitGTK + tray indicator
- 不改 Provider API 契约；`USAGES_*` 环境变量覆盖仍有效
