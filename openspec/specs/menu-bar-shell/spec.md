# menu-bar-shell

## Purpose

TBD — macOS 菜单栏入口与弹出面板：布局、刷新与设置入口。

## Requirements

### Requirement: Menu bar entry point
The system SHALL present a macOS menu bar icon that opens a popup panel when activated.

#### Scenario: Open panel from menu bar
- **WHEN** the user clicks the menu bar icon
- **THEN** the popup panel becomes visible with the local overview layout

#### Scenario: App runs without main window
- **WHEN** the application launches on macOS
- **THEN** it SHALL keep a menu bar presence without requiring a persistent main document window

### Requirement: Panel composition
The popup panel MUST show Cursor usage, Codex local rate-limit progress (or an explicit Codex failure / needs-auth state), system metrics, latency, a status footer, and placeholder entries for remaining unsupported AI providers.

#### Scenario: Compact overview contents
- **WHEN** the panel is open and data has been loaded or attempted
- **THEN** the panel SHALL display the Cursor card, a Codex progress card bound to the Codex `UsageSnapshot` (success progress or explicit failure/needs-auth messaging), System card, Latency card, footer with refresh/update info and "Local only", and non-functional placeholders for DeepSeek and third-party API

#### Scenario: Codex card without silent fake progress
- **WHEN** Codex status is not `ok`
- **THEN** the Codex card SHALL show an explicit unavailable or needs-auth state and MUST NOT render a success-looking fabricated progress bar

### Requirement: Manual and automatic refresh
The system SHALL allow manual refresh and configurable automatic refresh of panel data, including attempting a Codex local rate-limits snapshot when refreshing usages.

#### Scenario: Manual refresh
- **WHEN** the user triggers refresh from the panel
- **THEN** the system SHALL re-fetch Cursor usage, Codex local rate limits (best-effort), system metrics, and latency according to their providers

#### Scenario: Auto refresh indication
- **WHEN** auto refresh is enabled
- **THEN** the footer SHALL indicate auto refresh status and the last successful update time when available

### Requirement: Settings entry
The system SHALL provide a settings entry to configure Cursor credentials, latency probe target, and refresh intervals.

#### Scenario: Open settings
- **WHEN** the user opens settings from the panel
- **THEN** the system SHALL allow editing Cursor session token, latency target, and refresh interval values within allowed bounds
