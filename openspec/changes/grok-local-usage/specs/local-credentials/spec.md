## ADDED Requirements

### Requirement: Grok auth material stays off settings and logs
Grok / SuperGrok authentication material used for billing (bearer token, user id, or equivalent) MUST NOT be stored in `settings.json` and MUST NOT be logged in full. When Meterbar optionally caches such material, it MUST use the secure store and/or mode-`0600` local fallback under the platform application data directory, consistent with other secrets.

#### Scenario: Settings file has no grok token fields
- **WHEN** settings are saved after Grok is used
- **THEN** `settings.json` MUST NOT contain Grok bearer tokens or cookies

#### Scenario: Error paths scrub secrets
- **WHEN** Grok billing fails
- **THEN** user-visible messages and logs MUST omit the full bearer token

### Requirement: Local Grok session discovery without mandatory paste
The system SHALL treat a discovered local Grok Build / `grok login` session as sufficient configuration when present. A user-pasted Grok credential MAY be supported as optional fallback but MUST NOT be required when local discovery succeeds.

#### Scenario: Discovery-only configuration
- **WHEN** local Grok auth is discovered and no Meterbar-pasted Grok secret exists
- **THEN** Grok MAY still be considered configured for refresh attempts
