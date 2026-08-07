## Purpose

Reads SuperGrok / Grok consumer weekly usage pool from the local Grok login session via the same cli-chat-proxy billing path used by Grok Build, and exposes it as a board UsageSnapshot.

## ADDED Requirements

### Requirement: Discover local Grok authentication material
The system SHALL attempt to obtain Grok consumer authentication material from the host machine without requiring Meterbar cloud login. Discovery SHOULD prefer credentials written by the official Grok Build / `grok login` flow when present. When no usable material is found, the Grok usage snapshot status MUST be `needs_auth` (or equivalent) without crashing.

#### Scenario: Local session present
- **WHEN** a usable Grok login token and user id (or equivalent material required by the billing request) are available on the machine
- **THEN** the system SHALL treat Grok as configured for visibility `auto` and MAY call the billing endpoint

#### Scenario: Local session missing
- **WHEN** no usable Grok auth material is found
- **THEN** refresh MUST NOT invent usage numbers and MUST surface an explicit needs-auth or unavailable state

### Requirement: Fetch weekly credits usage from cli-chat-proxy billing
When authenticated, the system SHALL request Grok credits configuration using an HTTPS GET to the cli-chat-proxy billing path with `format=credits` (default base `https://cli-chat-proxy.grok.com/v1`, overridable for tests). The request MUST include the authenticated bearer (or equivalent) and user identity headers required by that proxy. Tokens MUST NOT be written to `settings.json` or application logs.

#### Scenario: Successful credits response
- **WHEN** the billing endpoint returns a successful JSON body with credit usage fields
- **THEN** the system SHALL map them into a Grok `UsageSnapshot` with status `ok`

#### Scenario: HTTP auth failure
- **WHEN** the billing endpoint returns 401 or 403
- **THEN** the Grok snapshot status MUST be `needs_auth` (or equivalent) and MUST NOT show a fabricated progress bar

#### Scenario: Network or parse failure
- **WHEN** the request fails to complete or JSON cannot be interpreted for usage percent
- **THEN** the snapshot MUST use an explicit error status (`network_error` or `parse_error`) without crashing the panel

### Requirement: Map credits config into UsageSnapshot
The system SHALL prefer `creditUsagePercent` (clamped 0–100) as the primary used percentage on the Grok card. Period end / reset time SHALL prefer `currentPeriod.end` when present, falling back to legacy period end fields when needed. Absolute unit counts MAY be omitted when the API only exposes percentage pools. Prepaid or on-demand fields MAY be shown when present and MUST be omitted when absent.

#### Scenario: Percent-primary card
- **WHEN** `creditUsagePercent` is 42.5 and a weekly period end is present
- **THEN** the Grok card SHALL show approximately 42.5% used and the reset/end time, without requiring a non-null absolute limit

#### Scenario: Legacy fallback
- **WHEN** percent is absent but legacy `used` and `monthlyLimit` cents are present with limit greater than zero
- **THEN** the system MAY derive percent as used/limit and still produce status `ok` when the derivation succeeds

### Requirement: Participate in panel refresh best-effort
Grok refresh SHALL participate in provider refresh rounds (manual and auto) on a best-effort basis. Failure of Grok MUST NOT prevent Cursor, Codex, DeepSeek, system, or latency refresh from updating the panel.

#### Scenario: Grok down others up
- **WHEN** Grok billing fails and Cursor refresh succeeds
- **THEN** the Cursor card SHALL still show fresh data and the Grok card SHALL show its explicit failure state

### Requirement: Optional open official Usage UI
The system MAY offer a user-visible path to open the official grok.com Usage settings surface (e.g. `https://grok.com/?_s=usage`) for management. Opening the browser MUST NOT be required for a successful snapshot when local auth and billing succeed.

#### Scenario: Manage without local success
- **WHEN** the user is in needs-auth and chooses the manage/open usage action if offered
- **THEN** the system SHALL open the official Usage URL (or equivalent) rather than inventing local numbers
