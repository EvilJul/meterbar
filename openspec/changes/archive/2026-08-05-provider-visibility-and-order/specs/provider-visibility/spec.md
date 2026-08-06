## ADDED Requirements

### Requirement: Persist per-provider board visibility mode

The system SHALL persist a non-secret visibility mode for each model provider (`cursor`, `codex`, `deepseek`) in `AppSettings` / `settings.json` as one of `auto`, `always`, or `hidden`. The default mode for every model provider MUST be `auto`. Settings persistence MUST NOT store API keys, cookies, or access tokens in these fields.

#### Scenario: Defaults when settings file omits visibility

- **WHEN** `settings.json` has no `providerVisibility` field (or is newly created)
- **THEN** `get_settings` SHALL return `auto` for `cursor`, `codex`, and `deepseek`

#### Scenario: User updates visibility mode

- **WHEN** the user sets a provider’s board visibility to `hidden` via `update_settings`
- **THEN** a subsequent `get_settings` in a new process SHALL return `hidden` for that provider

#### Scenario: Invalid mode falls back to auto

- **WHEN** the settings file contains an unrecognized visibility string for a provider
- **THEN** the system SHALL treat that provider’s mode as `auto` after load/normalize

### Requirement: Board shows a model provider only according to mode and configuration

The board SHALL show a model provider card if and only if:
- mode is `always`, or
- mode is `auto` AND the provider is configured (`is_configured`).
The board MUST NOT show the card when mode is `hidden`, and MUST NOT show the card when mode is `auto` and the provider is not configured.

#### Scenario: Auto hides unconfigured provider

- **WHEN** a provider’s mode is `auto` and `is_configured` is false
- **THEN** the corresponding board card MUST be hidden

#### Scenario: Auto shows configured provider

- **WHEN** a provider’s mode is `auto` and `is_configured` is true
- **THEN** the corresponding board card MUST be visible

#### Scenario: Always shows even when unconfigured

- **WHEN** a provider’s mode is `always` and `is_configured` is false
- **THEN** the corresponding board card MUST be visible (so the user can see auth/setup guidance)

#### Scenario: Hidden overrides configuration

- **WHEN** a provider’s mode is `hidden` and `is_configured` is true
- **THEN** the corresponding board card MUST be hidden

### Requirement: Define is_configured per model provider

The system SHALL evaluate `is_configured` at runtime (not persisted) as follows:
- `deepseek`: true only when a DeepSeek API key is saved
- `cursor`: true only when a local Cursor session or Cookie credential is available (presence of credentials; not solely last snapshot `ok`)
- `codex`: true only when the local Codex integration can read rate limits as authenticated; uninstalled or `needs_auth` MUST count as not configured

#### Scenario: DeepSeek without API key is not configured

- **WHEN** no DeepSeek API key is stored
- **THEN** `is_configured("deepseek")` MUST be false

#### Scenario: Cursor without session and cookie is not configured

- **WHEN** neither a usable local Cursor session nor a Cookie credential is available
- **THEN** `is_configured("cursor")` MUST be false

#### Scenario: Codex needs_auth is not configured

- **WHEN** the Codex usage snapshot status is `needs_auth` (or Codex is not installed / cannot authenticate)
- **THEN** `is_configured("codex")` MUST be false

### Requirement: Settings UI exposes visibility control without hiding setup

The settings view SHALL offer a board-visibility control (`auto` / `always` / `hidden`) for each model provider. The settings view MUST continue to show each provider’s credential or setup section even when that provider’s board card is hidden.

#### Scenario: Hidden provider remains configurable

- **WHEN** the user sets Cursor board visibility to `hidden`
- **THEN** the settings page MUST still show the Cursor setup/credential section so the user can configure or change the mode later

### Requirement: System metrics use a separate visibility switch

The system SHALL persist `showSystemSection` and `showLatencySection` (booleans, default `true`) in `AppSettings` / `settings.json`. Each flag controls only its card. When loading a legacy file that has only `showSystemSection`, both fields MUST be set to that value. System/Latency MUST NOT be treated as entries in model-provider ordering.

#### Scenario: Default shows system section

- **WHEN** settings use defaults
- **THEN** System and Latency cards MUST be visible on the board

#### Scenario: User hides system section

- **WHEN** `showSystemSection` is false and `showLatencySection` is true
- **THEN** System MUST be hidden and Latency MUST remain visible

#### Scenario: User hides latency section

- **WHEN** `showLatencySection` is false and `showSystemSection` is true
- **THEN** Latency MUST be hidden and System MUST remain visible
