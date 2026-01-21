# PRD: Heartbeat Stall Detection Tuning

## Introduction

Ralph's heartbeat monitoring system is generating false positive stall detections that kill agents before they produce any output. The current effective timeout of 180s (45s interval × 4 threshold) is too aggressive for Claude Code's legitimate initial reasoning phases, which can take 2-4 minutes on complex prompts with large context windows, MCP server discovery, and codebase exploration.

Evidence from production runs shows 21+ failed steps in single runs, all occurring at iteration 1, all hitting exactly 180s timeout across multiple repositories (~/.ralph, ~/no_drake_in_the_house, ~/offwhite).

## Goals

- Eliminate false positive stall detections during legitimate agent reasoning
- Increase effective stall timeout from 180s to 300s (5 minutes)
- Add observability to enable data-driven tuning decisions
- Improve warning messages to be actionable and informative
- Maintain protection against genuinely hung agents

## User Stories

### US-001: Increase heartbeat interval to 60 seconds
**Description:** As a Ralph user, I want a longer heartbeat interval so that legitimate long-running agent operations aren't prematurely killed.

**Acceptance Criteria:**
- [ ] Default `heartbeat_interval` changed from 45s to 60s in TimeoutConfig
- [ ] CLI help text updated to reflect new default
- [ ] Unit tests updated to use new default where applicable
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-002: Increase missed heartbeats threshold to 5
**Description:** As a Ralph user, I want a higher missed heartbeat threshold so that the effective timeout is 300s (5 minutes) instead of 180s.

**Acceptance Criteria:**
- [ ] Default `missed_heartbeats_threshold` changed from 4 to 5 in TimeoutConfig
- [ ] CLI help text updated to reflect new default
- [ ] Unit tests updated to use new default where applicable
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-003: Improve warning message clarity
**Description:** As a Ralph user, I want clear warning messages that tell me elapsed time and what's about to happen so I understand the system state.

**Acceptance Criteria:**
- [ ] Warning message includes actual elapsed time (not just heartbeat count)
- [ ] Warning message indicates time until stall detection triggers
- [ ] Format: "Warning: No agent output for {elapsed}s ({missed} heartbeat intervals). Stall detection in {remaining}s unless activity resumes. (iteration {n})"
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-004: Improve stall detection message clarity
**Description:** As a Ralph user, I want the stall detection message to clearly explain what happened.

**Acceptance Criteria:**
- [ ] Stall detection message includes total elapsed time
- [ ] Message indicates this was due to no output activity
- [ ] Format: "Agent stall detected: no output for {elapsed}s (exceeded {threshold}s threshold). Terminating agent. (iteration {n})"
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-005: Add timing information to HeartbeatEvent
**Description:** As a developer, I need HeartbeatEvent to carry elapsed time information so warning/stall messages can display accurate timing.

**Acceptance Criteria:**
- [ ] `HeartbeatEvent::Warning` includes elapsed_seconds field
- [ ] `HeartbeatEvent::StallDetected` includes elapsed_seconds field
- [ ] Heartbeat monitor calculates and populates elapsed time
- [ ] Executor uses elapsed time in messages
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-006: Add stall detection metrics logging
**Description:** As a developer, I want stall detection events logged with structured data so I can analyze patterns and tune thresholds.

**Acceptance Criteria:**
- [ ] Log warning events with: elapsed_seconds, missed_count, iteration, run_id
- [ ] Log stall detection events with: elapsed_seconds, missed_count, iteration, run_id
- [ ] Use structured logging format (JSON or key=value pairs)
- [ ] Logs written to stderr (not stdout) to avoid mixing with agent output
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-007: Document timeout configuration in CLI help
**Description:** As a Ralph user, I want clear documentation of timeout flags so I can tune them for my use case.

**Acceptance Criteria:**
- [ ] `--heartbeat-interval` help includes recommended range (30-120s)
- [ ] `--heartbeat-threshold` help explains effective timeout calculation
- [ ] Help text includes example: "With interval=60s and threshold=5, stall detection triggers after 300s of no output"
- [ ] `ralph run --help` displays updated help text
- [ ] `cargo clippy` passes

## Functional Requirements

- FR-1: Change default `heartbeat_interval` from 45s to 60s in `src/timeout/mod.rs`
- FR-2: Change default `missed_heartbeats_threshold` from 4 to 5 in `src/timeout/mod.rs`
- FR-3: Modify `HeartbeatEvent::Warning(u32)` to `HeartbeatEvent::Warning { missed: u32, elapsed_secs: u64 }`
- FR-4: Modify `HeartbeatEvent::StallDetected(u32)` to `HeartbeatEvent::StallDetected { missed: u32, elapsed_secs: u64 }`
- FR-5: Update heartbeat monitor to calculate and include elapsed seconds in events
- FR-6: Update executor warning message format to include elapsed time and countdown
- FR-7: Update executor stall message format to include elapsed time and threshold
- FR-8: Add structured log output for stall-related events
- FR-9: Update CLI help text with new defaults and usage guidance

## Non-Goals

- No adaptive grace period based on codebase size (future work)
- No automatic threshold adjustment based on historical data
- No changes to the `startup_grace_period` default (120s is adequate)
- No changes to `agent_timeout` or `iteration_timeout`
- No Prometheus/metrics endpoint (structured logs suffice for now)

## Technical Considerations

- Changes are localized to `src/timeout/mod.rs`, `src/timeout/heartbeat.rs`, and `src/mcp/tools/executor.rs`
- HeartbeatEvent enum change requires updating all match statements
- Test configurations use explicit values, so default changes won't break tests
- Backward compatible: CLI flags override defaults, so existing scripts work

## Success Metrics

- Zero false positive stall detections on iteration 1 for standard workloads
- Stall detection still triggers within 6 minutes for genuinely hung agents
- Warning messages provide actionable information (elapsed time, countdown)
- Structured logs enable future analysis of stall patterns

## Open Questions

- Should we consider different defaults for parallel vs sequential execution?
- Should there be a "verbose" mode that logs every heartbeat pulse for debugging?
