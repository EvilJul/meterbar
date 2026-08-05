## MODIFIED Requirements

### Requirement: Fetch Cursor usage summary with session cookie
The system SHALL fetch Cursor personal usage via `GET https://cursor.com/api/usage-summary`. Authentication SHALL be attempted in order: (1) local Cursor access token as Bearer on usage-summary and compatible internal endpoints, (2) stored `WorkosCursorSessionToken` cookie. Requests SHALL be performed from the Rust/backend side.

#### Scenario: Successful fetch via local session
- **WHEN** a valid local access token is available and a usage endpoint returns a successful JSON body containing mappable plan usage
- **THEN** the system SHALL produce a `UsageSnapshot` with `provider=cursor`, `status=ok`, and populated period and usage fields

#### Scenario: Successful fetch via cookie fallback
- **WHEN** local session is unavailable or rejected but a valid stored session cookie exists
- **THEN** the system SHALL fetch via cookie and produce `status=ok` when the response is valid

#### Scenario: Missing credentials
- **WHEN** neither local session nor stored cookie is available
- **THEN** the Cursor usage status SHALL be `needs_auth` and the panel SHALL prompt the user that signing into Cursor locally is preferred, with optional Cookie paste

#### Scenario: Unauthorized response
- **WHEN** all attempted endpoints respond with authentication failure
- **THEN** the Cursor usage status SHALL be `needs_auth`
