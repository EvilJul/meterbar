## MODIFIED Requirements

### Requirement: Collect CPU and memory metrics
The system SHALL collect local CPU utilization percent and memory used/total bytes on supported platforms (macOS and Linux) for display in the System card.

#### Scenario: Successful CPU and memory sample
- **WHEN** the panel refreshes system metrics
- **THEN** the System card SHALL show CPU percent and memory used/total (or equivalent percent) from a local sample

#### Scenario: Linux sample succeeds without macOS APIs
- **WHEN** the app runs on Linux and refreshes system metrics
- **THEN** CPU and memory SHALL be obtained without requiring `ioreg` or other macOS-only tools

### Requirement: Optional GPU metrics
The system SHALL attempt to collect GPU utilization and/or temperature when available, and MUST display N/A or omit numeric GPU fields when unavailable. On macOS, GPU may use host-specific tools (e.g. `ioreg`). On Linux, GPU collection is best-effort; absence of a GPU reader SHALL NOT fail system sampling.

#### Scenario: GPU unavailable
- **WHEN** GPU metrics cannot be obtained on the current machine
- **THEN** the System card SHALL still render CPU and memory without failing, and GPU SHALL show an unavailable state

#### Scenario: Linux GPU optional
- **WHEN** the app runs on Linux and no Linux GPU reader is implemented or succeeds
- **THEN** GPU fields SHALL be null/unavailable and CPU/memory SHALL still refresh

### Requirement: Independent refresh
System metrics refresh SHALL operate independently from Cursor usage fetch failures.

#### Scenario: Cursor down system up
- **WHEN** Cursor usage fetch fails and system sampling succeeds
- **THEN** the System card SHALL still show fresh local metrics

## ADDED Requirements

### Requirement: Primary disk mount selection on Linux
On Linux, primary disk sampling SHALL prefer the root mount `/` (or the largest non-removable volume when `/` is unavailable), without requiring macOS APFS Data volume paths.

#### Scenario: Linux root disk
- **WHEN** system metrics sample disk usage on Linux and `/` is mounted with positive total space
- **THEN** used and available disk figures SHALL be derived from that primary selection (or documented statfs-based equivalent)
