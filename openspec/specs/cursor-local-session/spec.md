# cursor-local-session

## Purpose

TBD — Cursor 本机会话：从本地 state 数据库读取 access token，供用量刷新鉴权。

## Requirements

### Requirement: Read Cursor access token from local state database
The system SHALL attempt to read `cursorAuth/accessToken` from Cursor's local SQLite database at the platform default path (macOS: `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb`) using read-only access. When a refresh needs both session probe metadata and the token value, the system SHALL open each candidate database at most once for that combined read (probe fields and token), rather than separate open cycles that race under WAL locks.

#### Scenario: Token present
- **WHEN** the database exists and contains a non-empty `cursorAuth/accessToken` value
- **THEN** the system SHALL obtain the token for usage fetch without requiring user-pasted Cookie

#### Scenario: Database missing or unreadable
- **WHEN** the database file is missing, locked, or the key is absent
- **THEN** the system SHALL skip local session auth without crashing and proceed to Cookie fallback

#### Scenario: Combined probe and token read
- **WHEN** refresh needs probe metadata and the access token for the same local session check
- **THEN** the system SHALL derive both from a single open per candidate path in that operation (or an equivalent single-pass API), reducing “session detected but token unreadable” races under concurrent writers

### Requirement: Preserve probe failure codes with single-pass read
The combined probe/token read SHALL preserve existing failure identifiers used by UI messaging (`cursor_db_not_found`, `cursor_db_not_openable`, `cursor_token_missing` or current equivalents) when the corresponding conditions hold.

#### Scenario: Missing database still reports not found
- **WHEN** no candidate `state.vscdb` path exists
- **THEN** the probe result SHALL indicate database-not-found (or equivalent) and token read SHALL yield no token

### Requirement: No persistence or logging of local access token
The system MUST NOT write the local access token to Keychain/fallback files and MUST NOT log the token value.

#### Scenario: Local session used successfully
- **WHEN** usage is fetched via local access token
- **THEN** the token SHALL only exist in memory for the request and SHALL NOT appear in logs or UI
