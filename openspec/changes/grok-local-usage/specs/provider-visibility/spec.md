## MODIFIED Requirements

### Requirement: Persist per-provider board visibility mode

The system SHALL persist a non-secret visibility mode for each model provider (`cursor`, `codex`, `deepseek`, `grok`) in `AppSettings` / `settings.json` as one of `auto`, `always`, or `hidden`. The default mode for every model provider MUST be `auto`. Settings persistence MUST NOT store API keys, cookies, or access tokens in these fields.

#### Scenario: Defaults when settings file omits visibility

- **WHEN** `settings.json` has no `providerVisibility` field (or is newly created)
- **THEN** `get_settings` SHALL return `auto` for `cursor`, `codex`, `deepseek`, and `grok`

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

#### Scenario: Auto shows configured Grok

- **WHEN** Grok mode is `auto` and local Grok auth material is present
- **THEN** the Grok board card MUST be eligible to show (subject to successful layout inclusion)

#### Scenario: Hidden Grok never shows

- **WHEN** Grok mode is `hidden`
- **THEN** the Grok card MUST NOT appear even if local auth is present
