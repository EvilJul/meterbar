# codex-local-rate-limits

## Purpose

TBD — 经本机 Codex app-server 读取 ChatGPT 订阅 rate-limit 进度。

## Requirements

### Requirement: Read Codex rate limits via local app-server
The system SHALL obtain Codex ChatGPT subscription rate-limit progress by connecting to a short-lived local Codex app-server (JSON-RPC), preferring the `account/rateLimits/read` method. The system MUST NOT use scraping of the ChatGPT Dashboard or Platform Usage/Costs APIs as the default path for this progress bar.

#### Scenario: Successful rate-limits snapshot
- **WHEN** a local Codex app-server is reachable with a logged-in ChatGPT session and `account/rateLimits/read` returns mappable `usedPercent` (and optionally `windowDurationMins` / `resetsAt`)
- **THEN** the system SHALL produce a `UsageSnapshot` with `provider=codex`, `status=ok`, and `percentUsed` derived from `usedPercent`

#### Scenario: Codex not installed or app-server unreachable
- **WHEN** the Codex binary/app-server cannot be started or connected within the configured timeout
- **THEN** the Codex usage status SHALL indicate local unavailability with an explicit user-visible message (e.g. local Codex unavailable) and MUST NOT invent progress percentages

#### Scenario: Not logged in
- **WHEN** app-server is reachable but the account is not authenticated for rate limits
- **THEN** the Codex usage status SHALL be `needs_auth` with guidance to sign in via local Codex / CLI

#### Scenario: Protocol or schema change
- **WHEN** the RPC succeeds but the response cannot be mapped to the expected rate-limit fields
- **THEN** the Codex usage status SHALL be a non-ok failure state (`parse_error` or equivalent explicit failure) with a clear message and MUST NOT show fabricated usage bars

### Requirement: Map rate-limit fields into UsageSnapshot
The system MUST map primary rate-limit progress into the existing `UsageSnapshot` shape for panel consumption.

#### Scenario: Map used percent
- **WHEN** `usedPercent` is present on the primary rate-limit window
- **THEN** `UsageSnapshot.percentUsed` SHALL equal that value for the Codex card primary progress

#### Scenario: Surface reset or window metadata
- **WHEN** `resetsAt` and/or `windowDurationMins` are present
- **THEN** the snapshot SHALL expose them for UI (via dedicated fields and/or `message` / period fields) so the card can show reset or window context

### Requirement: No silent fake data
Codex progress display MUST reflect a real successful snapshot or an explicit failure; silent zeros or placeholder percentages presented as live data are forbidden.

#### Scenario: Failure does not look like empty quota success
- **WHEN** Codex fetch fails for any reason covered by this capability
- **THEN** the UI/status MUST communicate failure or needs-auth and MUST NOT present a success-looking full-remaining bar from fabricated numbers

### Requirement: Credential and settings safety
The system MUST NOT persist ChatGPT cookies or secrets for Codex into Usages credential stores or `settings.json`.

#### Scenario: No cookie disk write
- **WHEN** Codex rate limits are fetched via local app-server
- **THEN** the system SHALL NOT write ChatGPT cookies or session secrets to disk for this feature

#### Scenario: Settings file stays secret-free
- **WHEN** application settings are saved
- **THEN** `settings.json` MUST NOT contain ChatGPT/Codex cookies, tokens, or API keys introduced by this change

### Requirement: Failure isolation
Codex provider failures MUST NOT prevent other panel sections from updating.

#### Scenario: Codex failure leaves other cards usable
- **WHEN** Codex rate-limit fetch fails
- **THEN** Cursor usage, system metrics, and latency sections SHALL still be refreshable and renderable independently
