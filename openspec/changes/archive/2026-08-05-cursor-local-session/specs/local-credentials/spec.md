## MODIFIED Requirements

### Requirement: Store Cursor session token in Keychain
The system SHALL store the Cursor `WorkosCursorSessionToken` value in the macOS Keychain (or equivalent OS secure store) as an optional fallback when local Cursor session is unavailable. The system MUST NOT persist the raw token in plaintext project config files.

#### Scenario: Save token
- **WHEN** the user submits a non-empty session token in settings
- **THEN** the system SHALL persist it via the secure store and subsequent usage fetches MAY read from that store after local session attempts fail

### Requirement: Local-only indication
The panel footer MUST indicate that credential and usage operations are local-only (`Local only` or equivalent).

#### Scenario: Footer label
- **WHEN** the panel is visible
- **THEN** the footer SHALL include a Local only indicator
