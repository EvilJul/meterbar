## Purpose

Defines platform-specific resolution of app data, credentials, settings, and Cursor local-session database paths on supported desktop OSes.

## ADDED Requirements

### Requirement: Resolve application data directory per platform
The system SHALL resolve a default application data directory for `com.usages.app` using the host platform conventions: on macOS `~/Library/Application Support/com.usages.app`; on Linux `$XDG_CONFIG_HOME/com.usages.app` when `XDG_CONFIG_HOME` is set and non-empty, otherwise `~/.config/com.usages.app`. Environment variable `USAGES_CREDENTIALS_DIR` SHALL override this directory when set and non-empty.

#### Scenario: Linux default without XDG override
- **WHEN** the app runs on Linux and `USAGES_CREDENTIALS_DIR` is unset and `XDG_CONFIG_HOME` is unset
- **THEN** the default application data directory SHALL be `$HOME/.config/com.usages.app`

#### Scenario: Explicit credentials directory override
- **WHEN** `USAGES_CREDENTIALS_DIR` is set to a non-empty path
- **THEN** that path SHALL be used as the application data directory for credentials and default settings placement

### Requirement: Resolve Cursor state database candidates per platform
The system SHALL discover Cursor `state.vscdb` (and backup if applicable) under platform default Cursor install locations for each candidate home directory. On macOS candidates include `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb` and Insiders equivalent. On Linux candidates include `~/.config/Cursor/User/globalStorage/state.vscdb` and Insiders equivalent. `USAGES_CURSOR_STATE_DB` SHALL override discovery when set to an existing path.

#### Scenario: Linux Cursor database present
- **WHEN** the app runs on Linux and `~/.config/Cursor/User/globalStorage/state.vscdb` exists and contains a token
- **THEN** local session read SHALL find that database without requiring macOS Application Support paths

#### Scenario: Unsupported or missing paths do not crash
- **WHEN** no platform candidate database exists
- **THEN** discovery SHALL return no paths and the app SHALL continue with Cookie or needs-auth fallback

### Requirement: Home directory resolution on Unix
On both macOS and Linux, the system SHALL prefer the passwd home directory for the current user when available, then fall back to the process home / `HOME` environment variable, without inventing non-existent paths.

#### Scenario: Linux uses passwd home when available
- **WHEN** the app resolves home directories on Linux and passwd provides a non-empty home
- **THEN** that home SHALL appear among candidates before or instead of a wrong launcher-injected `HOME` alone when both differ
