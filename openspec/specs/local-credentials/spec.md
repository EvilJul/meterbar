# local-credentials

## Purpose

TBD — 本机凭证：Cursor Cookie 安全存储（可选回退）、清理与防泄漏。

## Requirements

### Requirement: Store Cursor session token in Keychain
The system SHALL store the Cursor `WorkosCursorSessionToken` value in the macOS Keychain (or equivalent OS secure store) as an optional fallback when local Cursor session is unavailable. The system MUST NOT persist the raw token in plaintext project config files.

#### Scenario: Save token
- **WHEN** the user submits a non-empty session token in settings
- **THEN** the system SHALL persist it via the secure store and subsequent usage fetches MAY read from that store after local session attempts fail

### Requirement: Clear credentials
The system SHALL allow the user to clear the stored Cursor session token.

#### Scenario: Clear token
- **WHEN** the user chooses to clear Cursor credentials
- **THEN** the secure store entry SHALL be removed and Cursor usage status SHALL become `needs_auth` on the next refresh

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
