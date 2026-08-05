## MODIFIED Requirements

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
