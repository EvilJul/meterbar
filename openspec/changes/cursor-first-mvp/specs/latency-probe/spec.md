## ADDED Requirements

### Requirement: Configurable latency probe target
The system SHALL probe network latency against a user-configurable target and display the result in the Latency card.

#### Scenario: Successful probe
- **WHEN** the configured target responds within the probe timeout
- **THEN** the Latency card SHALL show latency in milliseconds with status `ok`

#### Scenario: Timeout or error
- **WHEN** the probe times out or fails
- **THEN** the Latency card SHALL show a non-ok status and MUST NOT crash the application

### Requirement: Default target
When the user has not configured a target, the system SHALL use a documented default probe target suitable for general connectivity checks.

#### Scenario: First launch default
- **WHEN** settings have no custom latency target
- **THEN** probes SHALL use the application default target

### Requirement: High latency visual emphasis
The system SHALL visually emphasize latency values that exceed a configurable high-latency threshold.

#### Scenario: Above threshold
- **WHEN** measured latency is greater than the high-latency threshold
- **THEN** the Latency card SHALL present the value with a warning emphasis (e.g. distinct color)
