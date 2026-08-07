## MODIFIED Requirements

### Requirement: Persist non-sensitive AppSettings to a local JSON file

The system SHALL persist non-sensitive `AppSettings` fields to a local JSON file under the platform application data directory for `com.usages.app` (see platform-paths), in a file distinct from credential fallback files. The settings file MUST NOT contain API keys, session cookies, or access tokens.

#### Scenario: Settings file path is separated from credentials

- **WHEN** the application resolves the settings storage location
- **THEN** the system SHALL use a dedicated settings file (default filename `settings.json` under the platform app data directory) and MUST NOT write settings into secure-store entries or credential fallback filenames (`cursor_session_token`, `deepseek_api_key`)

#### Scenario: Successful update is durable across restart

- **WHEN** the user successfully updates settings via `update_settings` and later starts a new application process
- **THEN** `get_settings` SHALL return the previously persisted (and clamped/normalized) values

#### Scenario: Linux default settings path

- **WHEN** the app runs on Linux and `USAGES_SETTINGS_PATH` / `USAGES_CREDENTIALS_DIR` are unset
- **THEN** the default settings path SHALL be under the Linux application data directory for `com.usages.app` (e.g. `~/.config/com.usages.app/settings.json`)

### Requirement: Load settings when AppState is initialized

The system SHALL load settings from disk when constructing application state at startup. Missing files MUST yield defaults without failing startup. Corrupt or unreadable files MUST yield defaults without failing startup.

#### Scenario: Missing settings file uses defaults

- **WHEN** the settings file does not exist at startup
- **THEN** the in-memory `AppSettings` SHALL equal `AppSettings` defaults and the application SHALL continue normally

#### Scenario: Corrupt settings file uses defaults

- **WHEN** the settings file exists but cannot be parsed as valid settings JSON
- **THEN** the system SHALL use default `AppSettings`, MUST NOT crash, and MUST NOT log secret material

### Requirement: Save settings on successful update_settings

The system SHALL write the full clamped/normalized `AppSettings` to disk after applying an `AppSettingsUpdate` patch. The public command signatures for `get_settings` and `update_settings` MUST remain unchanged (no breaking API change).

#### Scenario: Update persists full settings snapshot

- **WHEN** the user successfully calls `update_settings` with a valid patch
- **THEN** the full clamped `AppSettings` SHALL be written to the settings file and a subsequent load SHALL reflect those values
