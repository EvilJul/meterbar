## MODIFIED Requirements

### Requirement: Persist model provider board order

The system SHALL persist `providerOrder` as an ordered list of model provider ids in `AppSettings` / `settings.json`. The only valid ids are `cursor`, `codex`, `deepseek`, and `grok`. The default order MUST be `["cursor", "codex", "deepseek", "grok"]` (or an equivalent documented default that includes `grok` once). The list MUST NOT include System, Latency, or secret material.

#### Scenario: Default order when field missing

- **WHEN** `settings.json` omits `providerOrder`
- **THEN** `get_settings` SHALL return a normalized order that includes `cursor`, `codex`, `deepseek`, and `grok`

#### Scenario: Updated order survives restart

- **WHEN** the user saves a valid permutation including `grok` via `update_settings`
- **THEN** a subsequent process `get_settings` SHALL return that same normalized order

### Requirement: Normalize providerOrder on load and update

On load and on `update_settings`, the system SHALL normalize `providerOrder` by: dropping unknown ids, de-duplicating (first occurrence wins), and appending any missing known ids in default-relative order so the result is always a permutation of the four known providers (`cursor`, `codex`, `deepseek`, `grok`).

#### Scenario: Unknown ids are dropped

- **WHEN** stored `providerOrder` is `["cursor", "acme", "grok"]`
- **THEN** the normalized order SHALL include `cursor` and `grok` and append missing known ids (`codex`, `deepseek`) without retaining `acme`

#### Scenario: Legacy three-provider list gains grok

- **WHEN** stored `providerOrder` is `["cursor", "codex", "deepseek"]` from an older settings file
- **THEN** normalization SHALL append `grok` so all four known ids are present

#### Scenario: Duplicates collapse

- **WHEN** stored `providerOrder` is `["grok", "grok", "cursor"]`
- **THEN** the normalized order SHALL contain `grok` once and include the other known providers

### Requirement: Board renders model cards in providerOrder

The board SHALL render visible model provider cards in the sequence given by the normalized `providerOrder`. Providers hidden by visibility rules MUST be skipped without breaking relative order among visible providers. System and Latency cards MUST remain outside this ordered list (fixed after the model card list).

#### Scenario: Reorder includes Grok

- **WHEN** `providerOrder` places `grok` first and the Grok card is visible
- **THEN** the Grok card MUST appear before other visible model cards
