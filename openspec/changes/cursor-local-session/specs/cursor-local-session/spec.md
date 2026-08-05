## ADDED Requirements

### Requirement: Read Cursor access token from local state database
The system SHALL attempt to read `cursorAuth/accessToken` from Cursor's local SQLite database at the platform default path (macOS: `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb`) using read-only access.

#### Scenario: Token present
- **WHEN** the database exists and contains a non-empty `cursorAuth/accessToken` value
- **THEN** the system SHALL obtain the token for usage fetch without requiring user-pasted Cookie

#### Scenario: Database missing or unreadable
- **WHEN** the database file is missing, locked, or the key is absent
- **THEN** the system SHALL skip local session auth without crashing and proceed to Cookie fallback

### Requirement: No persistence or logging of local access token
The system MUST NOT write the local access token to Keychain/fallback files and MUST NOT log the token value.

#### Scenario: Local session used successfully
- **WHEN** usage is fetched via local access token
- **THEN** the token SHALL only exist in memory for the request and SHALL NOT appear in logs or UI
