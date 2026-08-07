## MODIFIED Requirements

### Requirement: Read Cursor access token from local state database
The system SHALL attempt to read `cursorAuth/accessToken` from Cursor's local SQLite database at platform default paths using read-only access. Platform defaults include macOS `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb` (and Insiders / backup variants) and Linux `~/.config/Cursor/User/globalStorage/state.vscdb` (and Insiders / backup variants). When a refresh needs both session probe metadata and the token value, the system SHALL open each candidate database at most once for that combined read (probe fields and token), rather than separate open cycles that race under WAL locks.

#### Scenario: Token present
- **WHEN** the database exists and contains a non-empty `cursorAuth/accessToken` value
- **THEN** the system SHALL obtain the token for usage fetch without requiring user-pasted Cookie

#### Scenario: Database missing or unreadable
- **WHEN** the database file is missing, locked, or the key is absent
- **THEN** the system SHALL skip local session auth without crashing and proceed to Cookie fallback

#### Scenario: Combined probe and token read
- **WHEN** refresh needs probe metadata and the access token for the same local session check
- **THEN** the system SHALL derive both from a single open per candidate path in that operation (or an equivalent single-pass API), reducing “session detected but token unreadable” races under concurrent writers

#### Scenario: Linux default path discovery
- **WHEN** the app runs on Linux and Cursor stores state under `~/.config/Cursor/User/globalStorage/state.vscdb`
- **THEN** the system SHALL include that path among candidates and MAY read the token when present
