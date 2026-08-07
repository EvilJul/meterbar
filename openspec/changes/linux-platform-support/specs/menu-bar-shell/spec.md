## ADDED Requirements

### Requirement: Linux system tray entry point
On Linux, the system SHALL present a system tray (status notifier / app indicator) icon that toggles the popup panel on left-click activation, without requiring a persistent main document window in the taskbar when platform APIs allow tray-only presence.

#### Scenario: Open panel from tray on Linux
- **WHEN** the user left-clicks the tray icon on Linux and the panel is not effectively shown
- **THEN** the popup panel becomes visible with the local overview layout

#### Scenario: Toggle hide when panel is shown
- **WHEN** the user left-clicks the tray icon while the panel is effectively shown
- **THEN** the panel SHALL hide

### Requirement: Platform-appropriate tray icon presentation
On macOS, the tray icon MAY use template-image semantics for menu bar coloring. On Linux, the tray icon MUST NOT rely on macOS template-only rendering; the icon SHALL remain visually recognizable on common light and dark tray backgrounds.

#### Scenario: Linux tray icon visible
- **WHEN** the application starts on Linux
- **THEN** the tray icon SHALL be visible without depending on macOS template icon monochrome inversion

### Requirement: Linux panel visual fallback
When OS window effects such as macOS `hudWindow` are unavailable, the panel UI SHALL remain readable (non-transparent-to-desktop-only) via solid or semi-opaque styling rather than failing to launch.

#### Scenario: Panel readable without macOS private effects
- **WHEN** the panel opens on Linux without macOS visual effect APIs
- **THEN** content and chrome SHALL remain legible against the desktop background

## MODIFIED Requirements

### Requirement: Menu bar entry point
The system SHALL present a host-platform status entry that opens a popup panel when activated: on macOS a menu bar icon; on Linux a system tray icon.

#### Scenario: Open panel from menu bar
- **WHEN** the user clicks the menu bar icon on macOS
- **THEN** the popup panel becomes visible with the local overview layout

#### Scenario: App runs without main window
- **WHEN** the application launches on macOS
- **THEN** it SHALL keep a menu bar presence without requiring a persistent main document window

#### Scenario: Open panel from system tray on Linux
- **WHEN** the user clicks the system tray icon on Linux
- **THEN** the popup panel becomes visible with the local overview layout
