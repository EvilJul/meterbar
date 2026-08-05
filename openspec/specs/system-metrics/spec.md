# system-metrics

## Purpose

TBD — 本机 CPU/内存（及可选 GPU）指标采集与独立刷新。

## Requirements

### Requirement: Collect CPU and memory metrics
The system SHALL collect local CPU utilization percent and memory used/total bytes on macOS for display in the System card.

#### Scenario: Successful CPU and memory sample
- **WHEN** the panel refreshes system metrics
- **THEN** the System card SHALL show CPU percent and memory used/total (or equivalent percent) from a local sample

### Requirement: Optional GPU metrics
The system SHALL attempt to collect GPU utilization and/or temperature when available, and MUST display N/A or omit numeric GPU fields when unavailable.

#### Scenario: GPU unavailable
- **WHEN** GPU metrics cannot be obtained on the current machine
- **THEN** the System card SHALL still render CPU and memory without failing, and GPU SHALL show an unavailable state

### Requirement: Independent refresh
System metrics refresh SHALL operate independently from Cursor usage fetch failures.

#### Scenario: Cursor down system up
- **WHEN** Cursor usage fetch fails and system sampling succeeds
- **THEN** the System card SHALL still show fresh local metrics
