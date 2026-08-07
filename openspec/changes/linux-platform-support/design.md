## Context

See proposal.md — Why. Today:

- Paths hardcode `~/Library/Application Support/com.usages.app` and macOS Cursor DB only.
- Shell: AppKit panel positioning, Accessory, `hudWindow`, template tray icon; non-macOS already has `position_panel_near_tray_tao` + simplified visibility.
- GPU: `ioreg` only; non-macOS returns `None`.
- Disk/home: partial `cfg` already for `/` and Linux `statfs` block size.

Constraints: no Windows; keep macOS UX; keep Tauri command surface; env overrides remain.

## Goals / Non-Goals

**Goals:**

- Single codepath with `cfg`/platform helpers; Linux builds and runs tray panel
- Correct default paths + Cursor session discovery on Linux
- Readable panel without macOS private effects
- Keyring + file dual-write portable

**Non-Goals:**

- Windows; Flatpak/AppImage polish; full Wayland edge-case matrix
- True NSPanel rewrite; Linux GPU parity with Apple Silicon
- Changing provider APIs or settings schema fields

## Decisions

### 1. Centralize paths in one helper

- Add small module (e.g. `platform_paths` or fold into `local_session` + shared `app_data_dir()`)
- `app_data_dir()`: macOS Application Support; Linux XDG config; env override first
- `cursor_state_db_candidates(home)`: per-OS list; keep Insiders + `.backup`
- Wire `credentials::fallback_dir`, `settings::settings_path`, `candidate_state_db_paths` through it
- **Alt rejected**: duplicate path strings in each file

### 2. Passwd home on all Unix

- Enable `getpwuid` home on Linux (same as macOS), not `None`
- Order unchanged: passwd → `home_dir()` → `$HOME`

### 3. Shell: cfg-only macOS extras; Linux uses existing tao path

- Keep AppKit / Accessory / ScreenSaver level / `set_effects(HudWindow)` under `cfg(macos)`
- Linux: `icon_as_template(false)`; ensure tray PNG has alpha + contrast
- `macOSPrivateApi` / tauri `macos-private-api`: remain macOS-only (feature gate if Linux build complains)
- Blur-to-hide: keep Focused(false); accept desktop variance; tray toggle remains primary

### 4. UI fallback without hud

- Prefer CSS: body/panel background solid or high-opacity when effects absent
- Prefer runtime detection or `document` class set from a thin `get_platform` command / `navigator.userAgent` only if needed — **prefer CSS that works on both** (macOS glass stays optional enhancement)

### 5. Credentials / keyring

- Keep dual-write; service id `com.usages.app`
- Linux: default keyring secret-service; if fail → fallback file still OK (already specified)
- No new cloud stores

### 6. System metrics

- CPU/mem: sysinfo (already)
- Disk: existing non-macOS `/` preference
- GPU: leave `None` on Linux in v1 unless cheap path later (`nvidia-smi` optional follow-up)
- VPN: existing tun/wg/ppp; optional later `tailscale0`

### 7. Docs & build

- README: Linux deps (webkit2gtk, libayatana-appindicator or distro equivalent), dual platform table
- Remove “Linux 非目标” as absolute; note known tray DE limits

## Risks / Trade-offs

| Risk | Mitigation |
|------|------------|
| GNOME tray missing without extension/indicator | Document; prefer AppIndicator-compatible Tauri tray |
| Wayland position / focus flaky | Tao fallback + tray toggle; manual QA on one DE first |
| Transparent window unreadable | CSS solid fallback |
| Secret Service absent (SSH/CI) | File fallback + `USAGES_CREDENTIALS_DIR` |
| Cursor path variants | Multiple candidates; env override |

## Migration Plan

1. Implement path helper + wire credentials/settings/cursor session
2. Tray/icon/cfg shell polish + CSS readability
3. Linux compile fix (features/deps)
4. Unit tests with temp dirs / env overrides
5. Manual smoke on Linux + regression on macOS
6. README

No data migration: Linux users start with empty config; macOS keeps old paths.

## Open Questions

- First QA target DE: GNOME vs KDE (default assume GNOME + AppIndicator package).
- Whether v1 ships any Linux GPU reader (default: no).
