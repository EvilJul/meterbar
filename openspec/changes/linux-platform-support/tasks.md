## 1. Platform paths

- [x] 1.1 Add `app_data_dir()` / shared path helper (macOS Application Support, Linux XDG config, `USAGES_CREDENTIALS_DIR` override)
- [x] 1.2 Enable Unix `getpwuid` home candidates on Linux (same priority as macOS)
- [x] 1.3 Expand Cursor `state.vscdb` candidates for Linux (`~/.config/Cursor` + Insiders + backup); keep macOS list
- [x] 1.4 Wire `credentials::fallback_dir` and `settings::settings_path` to the helper
- [x] 1.5 Unit tests: path resolution under env overrides and Linux/macOS cfg expectations

## 2. Credentials & settings behavior

- [x] 2.1 Confirm dual-write works when keyring fails (Linux Secret Service missing) via file fallback only
- [x] 2.2 Confirm DeepSeek key paths use the same app data dir
- [x] 2.3 Settings load/save still use `settings.json` under platform dir; no schema change

## 3. Tray shell (Linux)

- [x] 3.1 Set `icon_as_template(true)` only on macOS; Linux false with readable tray PNG
- [x] 3.2 Verify Linux uses `position_panel_near_tray_tao` / show-hide toggle without AppKit
- [x] 3.3 Gate `macos-private-api` / private effects so Linux builds cleanly
- [x] 3.4 CSS (or minimal platform class) so panel is readable without `hudWindow`

## 4. System metrics

- [x] 4.1 Keep GPU `None` on Linux for v1; ensure sample does not call `ioreg`
- [x] 4.2 Confirm Linux primary disk prefers `/` and CPU/memory via sysinfo
- [x] 4.3 Run existing system unit tests on Linux host

## 5. Docs & validation

- [x] 5.1 Update README: dual platform, Linux build deps, paths, remove absolute “Linux 非目标”
- [x] 5.2 Manual smoke: Linux tray open/close, settings persist, Cursor local session if present, Cookie fallback
- [x] 5.3 macOS regression checklist: menu bar, paths unchanged, template icon, effects
- [x] 5.4 `openspec validate linux-platform-support` (or project equivalent) passes
