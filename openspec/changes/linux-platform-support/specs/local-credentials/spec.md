## MODIFIED Requirements

### Requirement: Store Cursor session token in Keychain
The system SHALL store the Cursor `WorkosCursorSessionToken` value in the host OS secure store when available (macOS Keychain; Linux Secret Service / equivalent via the keyring backend) as an optional fallback when local Cursor session is unavailable. The system MUST dual-write a mode-`0600` local fallback file under the platform application data directory when possible. The system MUST NOT persist the raw token in plaintext project config files.

#### Scenario: Save token
- **WHEN** the user submits a non-empty session token in settings
- **THEN** the system SHALL persist it via the secure store and/or local fallback and subsequent usage fetches MAY read from that store after local session attempts fail

#### Scenario: Secure store unavailable on Linux
- **WHEN** the OS secure store is unavailable on Linux but the local fallback write succeeds
- **THEN** the system SHALL still consider save successful if the fallback can be read back with the same value

### Requirement: Clear credentials
The system SHALL allow the user to clear the stored Cursor session token.

#### Scenario: Clear token
- **WHEN** the user chooses to clear Cursor credentials
- **THEN** the secure store entry and local fallback file SHALL be removed when present and Cursor usage status SHALL become `needs_auth` on the next refresh when no local session remains

### Requirement: No secret leakage in logs or UI copy
The system MUST NOT log or display the full session token in ordinary UI text or application logs.

#### Scenario: Error without token echo
- **WHEN** authentication or network errors occur
- **THEN** user-visible messages and logs SHALL omit the full token value

### Requirement: Local-only indication
The panel footer MUST indicate that credential and usage operations are local-only (`Local only` or equivalent).

#### Scenario: Footer label
- **WHEN** the panel is visible
- **THEN** the footer SHALL include a Local only indicator

## ADDED Requirements

### Requirement: Credential fallback path is platform-relative
Credential fallback files (`cursor_session_token`, `deepseek_api_key`) SHALL live under the platform application data directory defined by platform-paths (not hard-coded solely to macOS Application Support), unless `USAGES_CREDENTIALS_DIR` overrides the directory.

#### Scenario: Linux fallback location
- **WHEN** a Cookie is saved on Linux without `USAGES_CREDENTIALS_DIR`
- **THEN** the fallback file SHALL be created under the Linux default application data directory for `com.usages.app`
