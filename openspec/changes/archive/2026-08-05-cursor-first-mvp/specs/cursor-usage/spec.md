## ADDED Requirements

### Requirement: Fetch Cursor usage summary with session cookie
The system SHALL fetch Cursor personal usage via `GET https://cursor.com/api/usage-summary` using a stored `WorkosCursorSessionToken` cookie, performed from the Rust/backend side (not from the webview directly).

#### Scenario: Successful fetch
- **WHEN** a valid session token is stored and the endpoint returns a successful JSON body containing `individualUsage.plan`
- **THEN** the system SHALL produce a `UsageSnapshot` with `provider=cursor`, `status=ok`, and populated period and usage fields

#### Scenario: Missing credentials
- **WHEN** no Cursor session token is stored
- **THEN** the Cursor usage status SHALL be `needs_auth` and the panel SHALL prompt the user to provide credentials

#### Scenario: Unauthorized response
- **WHEN** the endpoint responds with authentication failure (e.g. HTTP 401 or not_authenticated)
- **THEN** the Cursor usage status SHALL be `needs_auth`

### Requirement: Normalize Pro+ style usage fields
For responses matching the verified Pro+/user shape, the system MUST map usage with cents semantics and use `totalPercentUsed` as the primary progress metric.

#### Scenario: Map cents and primary percent
- **WHEN** a successful summary includes `individualUsage.plan.used`, `limit`, and `totalPercentUsed`
- **THEN** `UsageSnapshot.unit` SHALL be `cents`, `used`/`limit` SHALL come from plan fields, and `percentUsed` SHALL equal `totalPercentUsed`

#### Scenario: Remaining percent display
- **WHEN** `totalPercentUsed` is available
- **THEN** the UI SHALL derive remaining percent as `100 - totalPercentUsed` for the primary "left" display and MUST NOT rely solely on `plan.remaining` as the primary remaining indicator

#### Scenario: On-demand hidden when disabled
- **WHEN** `individualUsage.onDemand.enabled` is false
- **THEN** the Cursor card SHALL omit on-demand usage display

### Requirement: Resilient parse and network errors
The system SHALL isolate Cursor provider failures so other panel sections remain usable.

#### Scenario: Parse failure
- **WHEN** the response body cannot be mapped to the expected usage shape
- **THEN** Cursor status SHALL be `parse_error` and system metrics / latency cards SHALL still render independently

#### Scenario: Network failure
- **WHEN** the request fails due to network error or timeout
- **THEN** Cursor status SHALL be `network_error` without crashing the application

### Requirement: Refresh cadence bounds
Cursor usage automatic refresh MUST default to 300 seconds and MUST NOT poll more frequently than every 60 seconds when auto refresh is enabled.

#### Scenario: Default interval
- **WHEN** the user has not customized the Cursor refresh interval
- **THEN** automatic Cursor refresh SHALL use 300 seconds

#### Scenario: Minimum interval enforcement
- **WHEN** the user sets a Cursor refresh interval below 60 seconds
- **THEN** the system SHALL enforce a minimum of 60 seconds
