# PRD: Runtime Evidence and Metrics Capture

## Introduction/Overview
Ralph currently lacks runtime evidence and execution logs for validating provisioning
outcomes and PRD success criteria. This feature adds structured evidence capture,
metrics, and export hooks so runs can be evaluated, retried, and post-analyzed.

## Goals
- Provide a consistent runtime evidence schema for all Ralph runs
- Capture metrics needed to evaluate PRD success criteria
- Enable downstream processing for retries, failure analysis, and incomplete work
- Preserve enough context to generate after-action reports per run

## User Stories

### US-001: Record run lifecycle evidence
**Description:** As an operator, I want each run to emit structured lifecycle events so I can audit what happened.

**Acceptance Criteria:**
- [ ] A run emits start, step, and completion events with timestamps
- [ ] Each event includes a run ID and step ID for correlation
- [ ] Failed steps include error type and message fields
- [ ] Evidence is written even on partial or failed runs

### US-002: Capture PRD success metrics
**Description:** As a developer, I want per-run metrics captured so success criteria can be evaluated automatically.

**Acceptance Criteria:**
- [ ] Metrics include at least: steps attempted, steps completed, failures, retries
- [ ] Metrics include timing: run duration and per-step duration
- [ ] Metrics include completeness: percent of steps with evidence
- [ ] Metrics are stored with the run ID and timestamp

### US-003: Persist evidence with retention rules
**Description:** As an operator, I want evidence stored with retention controls to support audits without unbounded growth.

**Acceptance Criteria:**
- [ ] Evidence records are stored in a durable local or configured backend
- [ ] Retention period is configurable via a single setting
- [ ] A run can be deleted without orphaned evidence

### US-004: Provide export hooks for downstream processing
**Description:** As an integrator, I want a stable export format so other systems can analyze runs and trigger retries.

**Acceptance Criteria:**
- [ ] Evidence can be exported as a single JSON document per run
- [ ] Export includes summary metrics and full event list
- [ ] Export exposes run status: success, failed, incomplete

### US-005: Detect and label incomplete work
**Description:** As a reviewer, I want the system to flag incomplete runs so they are easy to triage.

**Acceptance Criteria:**
- [ ] Incomplete runs are labeled when expected steps are missing
- [ ] Incomplete status is derived from evidence, not manual tags
- [ ] Incomplete runs are included in exports and summaries

## Functional Requirements
- FR-1: The system must generate a unique run ID for each execution
- FR-2: The system must emit structured evidence events for run lifecycle
- FR-3: Evidence events must include timestamps and correlation fields
- FR-4: The system must calculate and store run-level metrics
- FR-5: The system must expose a single export entry point per run
- FR-6: The system must support configurable retention and deletion
- FR-7: The system must detect incomplete runs from evidence coverage
- FR-8: The system must record retries with attempt counts
- FR-9: The system must version the evidence schema and record storage location
- FR-10: The system must map PRD success criteria to evidence by story ID
- FR-11: The system must expose a CLI entry point to export or review evidence

## Non-Goals (Out of Scope)
- Real-time dashboards or visualization UI
- Distributed tracing across external services
- External analytics pipelines beyond the export hook

## Design Considerations (Optional)
- Evidence schema should be versioned for backward compatibility
- Keep event payloads small; store large artifacts separately if needed
- Use consistent naming for statuses: success, failed, incomplete
- Include explicit storage location metadata in exports
- Keep export format stable for downstream systems
- Align evidence model with standard observability signals (run trace, step spans, derived metrics)
- Support redaction of sensitive prompt or response content while keeping metadata

## Technical Considerations (Optional)
- Provide an interface for evidence writers used by all execution paths
- Ensure evidence write failures do not crash a run, but are surfaced
- Consider a local file backend first, with pluggable storage later
- Metrics should be derived from events to avoid double reporting
- Link PRD success criteria to evidence using story IDs from prd.json
- Provide a CLI subcommand or flag for exporting and reviewing evidence
- Ensure evidence export can be mapped to OTel-like fields for future OTLP support

## Success Metrics
- 95%+ of runs have complete evidence coverage (no missing steps)
- 100% of runs produce an exportable run summary
- <5% of runs have evidence write errors
- Time to generate after-action report reduced by 50%

## Open Questions
- What is the authoritative list of steps per run for completeness checks?
- Where should evidence be stored by default (repo, tmp, or config path)?
- Should evidence be emitted synchronously or buffered asynchronously?
- What is the expected retention period and size budget per run?
- How should PRD success criteria be represented in prd.json for evaluation?
