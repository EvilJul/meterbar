# settings-persistence

## Purpose

TBD — 非敏感 AppSettings 本地 JSON 持久化、加载与校验。

## Requirements

### Requirement: Persist non-sensitive AppSettings to a local JSON file

The system SHALL persist the four `AppSettings` fields (`cursorRefreshSec`, `systemRefreshSec`, `latencyTarget`, `highLatencyMs`) to a local JSON file under the application support directory for `com.usages.app`, in a file distinct from credential fallback files. The settings file MUST NOT contain API keys, session cookies, or access tokens.

#### Scenario: Settings file path is separated from credentials

- **WHEN** the application resolves the settings storage location
- **THEN** the system SHALL use a dedicated settings file (default: `~/Library/Application Support/com.usages.app/settings.json`) and MUST NOT write settings into Keychain entries or credential fallback filenames (`cursor_session_token`, `deepseek_api_key`)

#### Scenario: Successful update is durable across restart

- **WHEN** the user successfully updates settings via `update_settings` and later starts a new application process
- **THEN** `get_settings` SHALL return the previously persisted (and clamped/normalized) values

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

#### Scenario: update_settings persists after clamp

- **WHEN** `update_settings` is invoked with a patch containing out-of-range numeric values
- **THEN** the system SHALL apply existing clamp/normalize rules, persist the resulting values, and return those values from the command

#### Scenario: disk write failure does not report success with unsaved state

- **WHEN** applying a settings patch succeeds in memory preparation but writing the settings file fails
- **THEN** the command SHALL return an error and the in-memory settings MUST match the pre-update snapshot (strict rollback)

### Requirement: Validate and normalize persisted values on load

On load, the system SHALL apply the same validation rules as runtime updates: `cursorRefreshSec` ≥ 60, `systemRefreshSec` in 10–30, `highLatencyMs` ≥ 1, and `latencyTarget` trimmed/normalized (empty → default; missing scheme → `https://` prefix).

#### Scenario: Loaded out-of-range values are clamped

- **WHEN** the settings file contains `systemRefreshSec` outside 10–30
- **THEN** the in-memory value SHALL be clamped into 10–30 before use

#### Scenario: Empty latency target becomes default

- **WHEN** the settings file contains an empty or whitespace-only `latencyTarget`
- **THEN** the in-memory value SHALL be `https://cursor.com`
