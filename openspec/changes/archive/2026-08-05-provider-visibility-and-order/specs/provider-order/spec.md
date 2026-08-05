## ADDED Requirements

### Requirement: Persist model provider board order

The system SHALL persist `providerOrder` as an ordered list of model provider ids in `AppSettings` / `settings.json`. The only valid ids are `cursor`, `codex`, and `deepseek`. The default order MUST be `["cursor", "codex", "deepseek"]`. The list MUST NOT include System, Latency, or secret material.

#### Scenario: Default order when field missing

- **WHEN** `settings.json` omits `providerOrder`
- **THEN** `get_settings` SHALL return `["cursor", "codex", "deepseek"]`

#### Scenario: Updated order survives restart

- **WHEN** the user saves `providerOrder` as `["deepseek", "cursor", "codex"]` via `update_settings`
- **THEN** a subsequent process `get_settings` SHALL return that same normalized order

### Requirement: Normalize providerOrder on load and update

On load and on `update_settings`, the system SHALL normalize `providerOrder` by: dropping unknown ids, de-duplicating (first occurrence wins), and appending any missing known ids in default-relative order so the result is always a permutation of the three known providers.

#### Scenario: Unknown ids are dropped

- **WHEN** stored `providerOrder` is `["cursor", "acme", "deepseek"]`
- **THEN** the normalized order SHALL be `["cursor", "deepseek", "codex"]` (missing `codex` appended)

#### Scenario: Duplicates collapse

- **WHEN** stored `providerOrder` is `["codex", "codex", "cursor"]`
- **THEN** the normalized order SHALL be `["codex", "cursor", "deepseek"]`

#### Scenario: Partial list completed

- **WHEN** stored `providerOrder` is `["deepseek"]`
- **THEN** the normalized order SHALL be `["deepseek", "cursor", "codex"]`

### Requirement: Board renders model cards in providerOrder

The board SHALL render visible model provider cards in the sequence given by the normalized `providerOrder`. Providers hidden by visibility rules MUST be skipped without breaking relative order among visible providers. System and Latency cards MUST remain outside this ordered list (fixed after the model card list).

#### Scenario: Reorder changes board sequence

- **WHEN** `providerOrder` is `["deepseek", "codex", "cursor"]` and all three cards are visible
- **THEN** the board DOM order of those cards MUST be DeepSeek, then Codex, then Cursor

#### Scenario: Hidden provider is skipped in sequence

- **WHEN** `providerOrder` is `["cursor", "codex", "deepseek"]` and Codex is hidden by visibility rules while Cursor and DeepSeek are visible
- **THEN** the board MUST show Cursor before DeepSeek, with no Codex card between them

### Requirement: Settings UI can move providers up and down

The settings view SHALL provide controls to move each model provider up or down in `providerOrder`. Each successful change MUST be persisted through `update_settings` (or the same settings save path used by other AppSettings fields).

#### Scenario: Move up persists new order

- **WHEN** the user moves DeepSeek above Cursor in the settings order list and the save succeeds
- **THEN** `providerOrder` SHALL place `deepseek` before `cursor`, and the board SHALL reflect that order on the next render
