## MODIFIED Requirements

### Requirement: Fetch Cursor usage summary with session cookie
The system SHALL fetch Cursor personal usage via authenticated HTTP from the Rust/backend side. Authentication SHALL be attempted in order: (1) local Cursor access token as Bearer, preferring `POST` Dashboard `GetCurrentPeriodUsage` and optionally falling back to Bearer `usage-summary` endpoints when they return mappable success, (2) stored `WorkosCursorSessionToken` cookie via `GET https://cursor.com/api/usage-summary`.

For Bearer flows, `needs_auth` SHALL be produced only when the primary Dashboard endpoint itself indicates authentication failure (HTTP 401 or unauthenticated body). A Bearer `usage-summary` HTTP 401 SHALL NOT alone cause `needs_auth`. Transient Dashboard failures (including HTTP 429, 5xx, or unparseable success-path bodies) SHALL surface as `network_error` (message MAY include HTTP status) rather than `needs_auth`.

#### Scenario: Successful fetch via local session
- **WHEN** a valid local access token is available and Dashboard (or a successful Bearer usage-summary fallback) returns a mappable plan usage body
- **THEN** the system SHALL produce a `UsageSnapshot` with `provider=cursor`, `status=ok`, and populated period and usage fields

#### Scenario: Successful fetch via cookie fallback
- **WHEN** local session is unavailable or Bearer path does not yield `ok`, but a valid stored session cookie exists
- **THEN** the system SHALL fetch via cookie and produce `status=ok` when the response is valid

#### Scenario: Missing credentials
- **WHEN** neither local session nor stored cookie is available
- **THEN** the Cursor usage status SHALL be `needs_auth` and the panel SHALL prompt the user that signing into Cursor locally is preferred, with optional Cookie paste

#### Scenario: Dashboard authentication failure
- **WHEN** Bearer Dashboard `GetCurrentPeriodUsage` responds with HTTP 401 or an unauthenticated body, and Cookie fallback is unavailable or also unauthorized
- **THEN** the Cursor usage status SHALL be `needs_auth`

#### Scenario: Bearer usage-summary rejects local token
- **WHEN** Dashboard does not return authentication failure, but Bearer `usage-summary` returns HTTP 401
- **THEN** the system SHALL NOT treat that 401 as session invalidation (`needs_auth`) solely because of the usage-summary response

#### Scenario: Dashboard transient failure without cookie
- **WHEN** local session token is present, Dashboard fails with a transient error (HTTP 429, 5xx, or parse failure), Bearer usage-summary does not yield `ok`, and no stored cookie is available
- **THEN** the Cursor usage status SHALL be `network_error` (not `needs_auth`) and the message MAY include the HTTP status

## ADDED Requirements

### Requirement: Bearer path diagnostic logging without secrets
The system SHALL emit diagnostic logs for Bearer usage fetch branches that include endpoint identity (or short name), HTTP status when known, and branch name. Logs MUST NOT include access token values, Authorization header contents, or Cookie secrets.

#### Scenario: Bearer attempt records branch
- **WHEN** the system attempts Dashboard or Bearer usage-summary during a refresh
- **THEN** a diagnostic log entry SHALL identify the branch outcome (for example success, needs_auth, unsupported token, or transient error) without logging the token
