# PRD: Reliability and Observability Hardening

## Introduction

This PRD addresses four critical reliability concerns identified during code review, hardening Ralph for broader adoption. The improvements focus on: eliminating panic-prone code paths, adding timeout protection to blocking operations, providing actionable failure diagnostics, and implementing circuit breaker patterns to prevent API cost burn during cascading failures.

These are preventive hardening measures to ensure system stability before scaling customer adoption.

## Goals

- Eliminate all production `expect()` calls that can crash the application
- Add 60-second timeout protection to git operations to prevent hung parallel batches
- Replace unstructured failure strings with typed objects containing file, line, error category, and fix suggestions
- Implement cross-story circuit breaker that pauses after 5 consecutive failures
- Maintain backward compatibility with existing checkpoint/evidence formats
- Zero regression in execution performance

## User Stories

### US-001: Convert ErrorPattern constructor to return Result
**Description:** As a developer, I want regex compilation errors to be handled gracefully so that invalid patterns don't crash the entire application.

**Acceptance Criteria:**
- [ ] `ErrorPattern::new()` returns `Result<Self, String>` instead of `Self`
- [ ] Regex compilation errors are propagated with descriptive messages including the invalid pattern
- [ ] `ErrorDetector::default_patterns()` returns `Result<Vec<ErrorPattern>, String>`
- [ ] `ErrorDetector::new()` handles pattern initialization errors gracefully
- [ ] All 24+ hardcoded patterns in `default_patterns()` are validated at construction
- [ ] `cargo test` passes with no panics
- [ ] `cargo clippy` passes

**Target Files:**
- `src/error/detector.rs` (lines 45-51, 113-118, 126-303)

---

### US-002: Replace expect() with defensive pattern matching in classify_error
**Description:** As a developer, I want error classification to handle edge cases gracefully so that unexpected regex behavior doesn't crash production.

**Acceptance Criteria:**
- [ ] Line 336 `expect()` replaced with `if let Some(m) = pattern.find(text)` pattern
- [ ] Function returns `None` gracefully when find() unexpectedly fails after matches()
- [ ] Add debug logging when this edge case occurs (should never happen, but log if it does)
- [ ] Existing tests continue to pass
- [ ] `cargo clippy` passes

**Target Files:**
- `src/error/detector.rs` (lines 330-349)

---

### US-003: Add git_timeout to TimeoutConfig
**Description:** As a developer, I want git operation timeouts to be configurable so that different environments can tune for their needs.

**Acceptance Criteria:**
- [ ] Add `git_timeout: Duration` field to `TimeoutConfig` struct (default: 60 seconds)
- [ ] Add `with_git_timeout()` builder method following existing pattern
- [ ] Add unit tests for the new field and builder
- [ ] Document the field with rustdoc comment explaining its purpose
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

**Target Files:**
- `src/timeout/mod.rs` (lines 18-46)

---

### US-004: Implement git operation timeout wrapper
**Description:** As a user, I want git operations to timeout after 60 seconds so that a hung git command doesn't block my entire parallel batch indefinitely.

**Acceptance Criteria:**
- [ ] `create_commit()` wraps git operations with `tokio::time::timeout()`
- [ ] Timeout duration sourced from `ExecutorConfig.timeout_config.git_timeout`
- [ ] Mutex acquisition (`git_mutex.lock().await`) also wrapped with timeout
- [ ] New `ExecutorError::GitTimeout(String)` variant for timeout-specific errors
- [ ] Timeout errors are classified as `ErrorCategory::Timeout(TimeoutReason::ProcessTimeout)`
- [ ] Checkpoint saved before returning timeout error
- [ ] Add integration test that simulates slow git operation
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

**Target Files:**
- `src/mcp/tools/executor.rs` (lines 1114-1160)
- `src/mcp/tools/executor.rs` (ExecutorError enum, lines 55-73)

---

### US-005: Add GateFailureDetail structured type
**Description:** As a developer, I need a structured type for quality gate failures so that downstream code can programmatically access failure details.

**Acceptance Criteria:**
- [ ] New `GateFailureDetail` struct with fields:
  - `file: Option<String>` - source file path
  - `line: Option<u32>` - line number
  - `column: Option<u32>` - column number
  - `error_code: Option<String>` - e.g., "E0001" for Rust, "no-unused-vars" for ESLint
  - `category: FailureCategory` - enum: `Lint`, `TypeCheck`, `Test`, `Format`, `Security`, `Coverage`
  - `message: String` - the error message
  - `suggestion: Option<String>` - fix suggestion if available
  - `doc_url: Option<String>` - link to relevant documentation
- [ ] Struct derives `Debug, Clone, Serialize, Deserialize`
- [ ] Add `FailureCategory` enum with variants listed above
- [ ] Unit tests for serialization roundtrip
- [ ] `cargo clippy` passes

**Target Files:**
- `src/quality/gates.rs` (new types, near line 109)

---

### US-006: Extend GateResult with structured failures
**Description:** As a developer, I want GateResult to contain structured failure details so that the UI and evidence system can display actionable information.

**Acceptance Criteria:**
- [ ] Add `failures: Vec<GateFailureDetail>` field to `GateResult` struct
- [ ] Existing `details: Option<String>` field retained for backward compatibility
- [ ] `GateResult::fail()` constructor accepts optional `Vec<GateFailureDetail>`
- [ ] `GateResult::pass()` and `GateResult::skipped()` set empty failures vec
- [ ] Serialization format remains backward compatible (failures field is additive)
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

**Target Files:**
- `src/quality/gates.rs` (lines 109-155)

---

### US-007: Parse clippy output into structured failures
**Description:** As a user, I want lint failures to show me exactly which file and line has issues so I can fix them quickly.

**Acceptance Criteria:**
- [ ] `extract_clippy_errors()` returns `Vec<GateFailureDetail>` instead of `String`
- [ ] Parses clippy JSON output format (`--message-format=json`) for structured data
- [ ] Extracts: file path, line number, column, error code, message, suggestion (if present)
- [ ] Falls back to text parsing if JSON parsing fails
- [ ] Limits to first 20 failures to prevent oversized results
- [ ] Each failure has `category: FailureCategory::Lint`
- [ ] `check_lint()` passes structured failures to `GateResult::fail()`
- [ ] Add unit test with sample clippy JSON output
- [ ] `cargo test` passes

**Target Files:**
- `src/quality/gates.rs` (lines 439-489)

---

### US-008: Parse cargo test output into structured failures
**Description:** As a user, I want test failures to show me the specific test name and assertion that failed so I can debug efficiently.

**Acceptance Criteria:**
- [ ] New `extract_test_failures()` function that parses `cargo test` output
- [ ] Extracts: test name, file path (if available), failure message, expected vs actual (if assertion)
- [ ] Handles both `cargo test` text output and `--format=json` if available
- [ ] Each failure has `category: FailureCategory::Test`
- [ ] Integrates with test gate in `check_tests()` method
- [ ] Limits to first 20 failures
- [ ] Add unit test with sample test failure output
- [ ] `cargo test` passes

**Target Files:**
- `src/quality/gates.rs` (new function, integrate with test checking)

---

### US-009: Parse format check output into structured failures
**Description:** As a user, I want format failures to show me which files need formatting so I can run the formatter on them.

**Acceptance Criteria:**
- [ ] `extract_format_errors()` returns `Vec<GateFailureDetail>` instead of `String`
- [ ] Extracts: file path from "Diff in" lines
- [ ] Each failure has `category: FailureCategory::Format`
- [ ] Suggestion field set to "Run `cargo fmt` to fix"
- [ ] `check_format()` passes structured failures to `GateResult::fail()`
- [ ] Add unit test with sample rustfmt output
- [ ] `cargo test` passes

**Target Files:**
- `src/quality/gates.rs` (lines 530-565)

---

### US-010: Parse security audit output into structured failures
**Description:** As a user, I want security vulnerabilities to show me the crate, version, and RUSTSEC ID so I can assess and remediate.

**Acceptance Criteria:**
- [ ] `extract_audit_vulnerabilities()` returns `Vec<GateFailureDetail>` instead of `String`
- [ ] Parses `cargo audit --json` output for structured vulnerability data
- [ ] Extracts: crate name, version, RUSTSEC ID, severity, title, advisory URL
- [ ] Each failure has `category: FailureCategory::Security`
- [ ] `doc_url` field populated with RUSTSEC advisory link
- [ ] Falls back to text parsing if JSON unavailable
- [ ] `check_security_audit()` passes structured failures to `GateResult::fail()`
- [ ] Add unit test with sample audit JSON output
- [ ] `cargo test` passes

**Target Files:**
- `src/quality/gates.rs` (lines 738-784)

---

### US-011: Add CircuitBreakerTriggered pause reason
**Description:** As a developer, I need a checkpoint pause reason for circuit breaker events so that resume can distinguish this from other pause types.

**Acceptance Criteria:**
- [ ] Add `CircuitBreakerTriggered { consecutive_failures: u32, threshold: u32 }` variant to `PauseReason` enum
- [ ] Variant serializes/deserializes correctly (test roundtrip)
- [ ] Add display formatting that shows "Circuit breaker triggered after N consecutive failures"
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

**Target Files:**
- `src/checkpoint/mod.rs` (lines 14-29)

---

### US-012: Add circuit_breaker_threshold to RunnerConfig
**Description:** As a developer, I want circuit breaker threshold to be configurable so users can tune sensitivity for their workloads.

**Acceptance Criteria:**
- [ ] Add `circuit_breaker_threshold: u32` field to `RunnerConfig` (default: 5)
- [ ] Add corresponding field to `ParallelRunnerConfig`
- [ ] Add `--circuit-breaker-threshold` CLI flag in main.rs
- [ ] Document the configuration option
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

**Target Files:**
- `src/runner.rs` (RunnerConfig struct, lines 32-60)
- `src/parallel/scheduler.rs` (ParallelRunnerConfig struct, lines 57-81)
- `src/main.rs` (CLI argument parsing)

---

### US-013: Implement circuit breaker in sequential runner
**Description:** As a user, I want execution to pause after 5 consecutive story failures so that I don't burn API costs on a broken environment.

**Acceptance Criteria:**
- [ ] Add `consecutive_failures: u32` counter in `run_sequential()` main loop
- [ ] Increment counter when story execution returns non-transient error
- [ ] Reset counter to 0 when a story succeeds
- [ ] When counter >= `circuit_breaker_threshold`:
  - Save checkpoint with `PauseReason::CircuitBreakerTriggered`
  - Log clear message: "Circuit breaker triggered: {n} consecutive failures"
  - Return from run with appropriate error
- [ ] Counter resets on run resume (fresh start per run attempt)
- [ ] Add unit test simulating consecutive failures
- [ ] `cargo test` passes

**Target Files:**
- `src/runner.rs` (lines 280-627, main sequential loop)

---

### US-014: Implement circuit breaker in parallel runner
**Description:** As a user running parallel execution, I want the circuit breaker to trigger when too many stories fail concurrently so that batch failures don't waste resources.

**Acceptance Criteria:**
- [ ] Track failures across parallel batch in `ParallelRunner::run()`
- [ ] Count non-transient failures from batch results
- [ ] When cumulative failures across batches >= `circuit_breaker_threshold`:
  - Save checkpoint with `PauseReason::CircuitBreakerTriggered`
  - Cancel any in-flight stories gracefully
  - Log clear message about circuit breaker trigger
  - Return from run with appropriate error
- [ ] Add integration test with parallel failure scenario
- [ ] `cargo test` passes

**Target Files:**
- `src/parallel/scheduler.rs` (lines 563-1085, main parallel loop)

---

### US-015: Display circuit breaker status in terminal UI
**Description:** As a user, I want to see when circuit breaker is approaching threshold so I can intervene before automatic pause.

**Acceptance Criteria:**
- [ ] Display "Failures: N/5" counter in terminal UI during execution
- [ ] Counter turns yellow at 60% threshold (3/5)
- [ ] Counter turns red at 80% threshold (4/5)
- [ ] Clear notification when circuit breaker triggers
- [ ] Counter resets visually when story succeeds
- [ ] Works in both sequential and parallel UI modes
- [ ] `cargo test` passes

**Target Files:**
- `src/ui/display.rs` or relevant UI module
- `src/ui/parallel_status.rs`

---

## Functional Requirements

- **FR-1:** `ErrorPattern::new()` must return `Result<Self, String>` and never panic on invalid regex
- **FR-2:** `classify_error()` must handle unexpected regex match failures without crashing
- **FR-3:** Git operations must timeout after configurable duration (default: 60 seconds)
- **FR-4:** Git timeout must save checkpoint before returning error
- **FR-5:** `GateResult` must include `Vec<GateFailureDetail>` with structured failure information
- **FR-6:** Each gate type (lint, test, format, security) must parse tool output into structured failures
- **FR-7:** Structured failures must include file path, line number, error code, and category at minimum
- **FR-8:** Sequential runner must track consecutive non-transient failures
- **FR-9:** Parallel runner must track cumulative failures across batches
- **FR-10:** Circuit breaker must trigger at configurable threshold (default: 5 consecutive failures)
- **FR-11:** Circuit breaker trigger must save checkpoint with distinct pause reason
- **FR-12:** All new configuration must be exposed via CLI flags

## Non-Goals

- No retry logic for git timeout (just fail and checkpoint)
- No automatic git repository repair after timeout
- No per-story circuit breaker (only cross-story)
- No circuit breaker for transient errors (only non-transient)
- No web UI for circuit breaker status (terminal only)
- No structured parsing for non-Rust toolchains (future work)
- No machine learning for failure pattern detection

## Technical Considerations

- **Backward Compatibility:** `GateResult` serialization must remain compatible with existing evidence files. The `failures` field is additive; old code ignoring it will continue to work.
- **Tokio Integration:** Git timeout uses `tokio::time::timeout()` which is already a dependency via full tokio features.
- **Regex Performance:** Converting to Result adds minimal overhead; regex compilation happens once at startup.
- **Error Propagation:** Changing `ErrorPattern::new()` signature is a breaking change for any external code using it directly. Internal usage will be updated.
- **JSON Parsing:** Clippy and cargo-audit JSON formats are stable but may change between major Rust versions. Include fallback to text parsing.

## Success Metrics

- Zero panics in production from eliminated `expect()` calls
- Git operations never block longer than configured timeout (default 60s)
- 100% of gate failures include at least file path and error message
- Circuit breaker triggers correctly after threshold consecutive failures
- No performance regression: execution time within 5% of baseline
- Checkpoint/resume works correctly after circuit breaker trigger

## Open Questions

1. Should git timeout trigger automatic retry once before failing?
2. Should circuit breaker count transient errors toward threshold (currently: no)?
3. Should structured gate failures be stored in separate evidence file to avoid bloating main evidence?
4. Should there be a "warning" phase before circuit breaker triggers (e.g., notification at 3/5)?
