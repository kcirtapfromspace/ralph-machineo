# PRD: Agent Execution UX Observability

## Introduction

Ralph users currently have zero visibility into what Claude is doing during story execution. The terminal shows only a spinner and iteration count (`⠙ Iteration [1/10]`), leaving users anxious about whether the agent is working, stuck, or needs input. This creates a "black box" experience that erodes trust and wastes time when users could quickly resolve blocking issues.

The streaming infrastructure exists (toggle state, keyboard bindings, display callbacks) but isn't connected to actual agent output. This PRD addresses wiring that infrastructure together and adding intelligent status detection.

## Goals

- Provide real-time visibility into agent activity during execution
- Surface when Claude needs user input instead of timing out
- Show elapsed time and activity health indicators
- Enable power users to see full verbose output when desired
- Reduce false-positive stall detections caused by legitimate input-wait states

## User Stories

### US-001: Wire streaming output to terminal display
**Description:** As a Ralph user, I want to see Claude's activity in real-time so that I know the agent is making progress.

**Acceptance Criteria:**
- [ ] Executor passes a display callback that receives each line of agent output
- [ ] When streaming is enabled (`[s]` toggle or `--verbose`), output lines are printed below the iteration indicator
- [ ] Output is prefixed with a subtle indicator (e.g., `  │ `) to visually separate from Ralph's UI
- [ ] Streaming respects the existing `ToggleState::should_show_streaming()` check
- [ ] Output does not interfere with the spinner/progress bar (use proper terminal control)
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-002: Add display callback infrastructure to executor
**Description:** As a developer, I need a callback mechanism in the executor so that display components can receive real-time output.

**Acceptance Criteria:**
- [ ] Add `DisplayCallback` trait with `on_agent_output(&self, line: &str)` method
- [ ] Add optional `display_callback: Option<Arc<dyn DisplayCallback>>` field to executor config
- [ ] Call `display_callback.on_agent_output()` when stdout/stderr lines are received
- [ ] Callback is called synchronously to maintain output order
- [ ] Callback errors are logged but do not fail execution
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-003: Implement DisplayCallback for TuiRunnerDisplay
**Description:** As a developer, I need TuiRunnerDisplay to implement the callback so streaming output reaches the terminal.

**Acceptance Criteria:**
- [ ] `TuiRunnerDisplay` implements `DisplayCallback` trait
- [ ] Output is printed only when `should_show_streaming()` returns true
- [ ] Output is formatted with timestamp if verbose mode is enabled
- [ ] Terminal cursor is managed correctly (clear line, print, restore spinner)
- [ ] Thread-safe: callback may be called from async context
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-004: Add last activity indicator to iteration display
**Description:** As a Ralph user, I want to see what Claude last did so that I have context even when streaming is off.

**Acceptance Criteria:**
- [ ] Iteration display shows last action summary below the progress bar
- [ ] Format: `└── {icon} {action} ({time} ago)`
- [ ] Icons: `📖` Reading, `✏️` Writing, `🔧` Running, `🧠` Thinking
- [ ] Action text is truncated to fit terminal width
- [ ] Updates on each new output line (debounced to max 2 updates/second)
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-005: Parse agent output for activity classification
**Description:** As a developer, I need to classify agent output lines into activity types for the status indicator.

**Acceptance Criteria:**
- [ ] Create `ActivityType` enum: `Reading`, `Writing`, `Running`, `Thinking`, `Unknown`
- [ ] Create `parse_activity(line: &str) -> Option<(ActivityType, String)>` function
- [ ] Detect file read patterns: `Reading file:`, `Read `, file paths ending in common extensions
- [ ] Detect file write patterns: `Writing file:`, `Write `, `Edit `, `Created `
- [ ] Detect command patterns: `Running:`, `Executing:`, `$ `, shell commands
- [ ] Extract relevant details (filename, command) for display
- [ ] `cargo test` passes with pattern matching tests
- [ ] `cargo clippy` passes

### US-006: Add elapsed time to iteration display
**Description:** As a Ralph user, I want to see how long the current iteration has been running so that I can gauge if something is wrong.

**Acceptance Criteria:**
- [ ] Iteration display shows elapsed time: `⠙ Iteration [1/10] █░░░░░░░░ 45s`
- [ ] Time format: seconds for <60s, `Xm Ys` for >=60s
- [ ] Time updates every second (or on each animation frame)
- [ ] Timer resets when iteration changes
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-007: Add activity health indicator
**Description:** As a Ralph user, I want to see if activity has stopped so that I'm warned before a timeout occurs.

**Acceptance Criteria:**
- [ ] Show time since last activity when >30 seconds: `Last activity: 45s ago`
- [ ] Show warning state when approaching threshold: `⚠️ No activity for 2m (timeout at 5m)`
- [ ] Health indicator uses color: green (<30s), yellow (30s-2m), red (>2m)
- [ ] Integrates with existing heartbeat monitor timing information
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-008: Detect agent input-needed state from output
**Description:** As a developer, I need to detect when Claude is waiting for user input so we don't timeout on legitimate waits.

**Acceptance Criteria:**
- [ ] Create `InputNeededDetector` that scans output for input-request patterns
- [ ] Detect `AskUserQuestion` tool calls in stream-json output
- [ ] Detect permission request patterns
- [ ] Detect clarification request patterns (`please confirm`, `which option`, etc.)
- [ ] Return structured `InputRequest` with question text and options if available
- [ ] `cargo test` passes with detection tests
- [ ] `cargo clippy` passes

### US-009: Pause heartbeat when input is needed
**Description:** As a Ralph user, I don't want the agent killed when it's legitimately waiting for my input.

**Acceptance Criteria:**
- [ ] When input-needed is detected, pause heartbeat monitoring
- [ ] Emit `HeartbeatEvent::InputNeeded` instead of continuing timeout countdown
- [ ] Heartbeat resumes when user responds or after configurable max wait (default: 10 minutes)
- [ ] State is logged for debugging: `Heartbeat paused: waiting for user input`
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-010: Display input-needed notification to user
**Description:** As a Ralph user, I want to be clearly notified when Claude needs my input so I can respond promptly.

**Acceptance Criteria:**
- [ ] Display prominent notification box when input-needed is detected
- [ ] Show the question Claude is asking (extracted from output)
- [ ] Show available options if multiple-choice
- [ ] Show keyboard hints: `[r] respond | [s] skip | [c] cancel`
- [ ] Notification replaces/overlays the normal iteration display
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-011: Handle user response to input request
**Description:** As a Ralph user, I want to respond to Claude's questions inline so execution can continue.

**Acceptance Criteria:**
- [ ] `[r]` key opens text input for free-form response
- [ ] `[1-9]` keys select numbered options directly
- [ ] `[s]` skips the question (sends empty/default response)
- [ ] `[c]` cancels the current story execution
- [ ] Response is written to agent's stdin
- [ ] Heartbeat monitoring resumes after response
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-012: Add --verbose flag for full output streaming
**Description:** As a power user, I want a command-line flag to see all agent output so I can debug issues.

**Acceptance Criteria:**
- [ ] Add `--verbose` / `-v` flag to `ralph run` command
- [ ] When enabled, all agent stdout/stderr is printed with timestamps
- [ ] Format: `[HH:MM:SS] {line}`
- [ ] Verbose mode implies streaming enabled
- [ ] Works with both sequential and parallel execution
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

### US-013: Add verbose output to parallel display
**Description:** As a Ralph user running parallel stories, I want to see activity for each concurrent story.

**Acceptance Criteria:**
- [ ] Each story's progress bar can show last activity summary
- [ ] Verbose mode shows interleaved output with story ID prefix: `[US-001] Reading...`
- [ ] Output is color-coded by story for visual separation
- [ ] Does not break MultiProgress layout
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes

## Functional Requirements

- FR-1: Add `DisplayCallback` trait to `src/ui/mod.rs` with `on_agent_output(&self, line: &str)` method
- FR-2: Add `display_callback` field to `ExecutorConfig` in `src/mcp/tools/executor.rs`
- FR-3: Call display callback on each stdout/stderr line in executor's async read loop
- FR-4: Implement `DisplayCallback` for `TuiRunnerDisplay` in `src/ui/tui_runner.rs`
- FR-5: Add `ActivityType` enum and `parse_activity()` function to `src/ui/activity.rs`
- FR-6: Add `last_activity` field to `TuiRunnerDisplay` updated on each output line
- FR-7: Render last activity in `IterationWidget::render_string()`
- FR-8: Add `elapsed_time` to iteration display, reset on iteration change
- FR-9: Add `InputNeededDetector` to `src/ui/input_detection.rs`
- FR-10: Add `HeartbeatEvent::InputNeeded` variant to heartbeat module
- FR-11: Add input notification rendering to `TuiRunnerDisplay`
- FR-12: Add stdin writer to executor for sending user responses
- FR-13: Add `--verbose` flag to CLI in `src/main.rs`
- FR-14: Pass verbose setting through to display options and callback behavior

## Non-Goals

- Real-time log file output (can be added later)
- Web UI or dashboard (CLI only for now)
- Automatic response generation for Claude's questions
- Recording/replay of agent sessions
- Integration with external monitoring systems (Prometheus, etc.)

## Technical Considerations

- **Thread safety**: Display callback will be called from async executor context; must be `Send + Sync`
- **Terminal control**: Must coordinate with indicatif/ratatui to avoid corrupting spinner display
- **Performance**: Debounce activity indicator updates to avoid excessive redraws
- **Stdin handling**: Agent process stdin must be kept open for potential user responses
- **Buffering**: May need line buffering on agent stdout to get timely updates

## Design Considerations

### Activity Indicator Layout
```
⠙ Iteration [1/10] █░░░░░░░░░░░░░░ 1m 23s
  └── 📖 Reading src/timeout/mod.rs (3s ago)
```

### Input-Needed Notification Layout
```
╔══════════════════════════════════════════════════════════════╗
║  ⚠️  AGENT NEEDS INPUT                                       ║
╠══════════════════════════════════════════════════════════════╣
║                                                              ║
║  Claude is asking:                                           ║
║  "Which database should I use for the new feature?"          ║
║                                                              ║
║    [1] PostgreSQL (recommended)                              ║
║    [2] SQLite                                                ║
║    [3] MySQL                                                 ║
║                                                              ║
║  Press [1-3] to select, [r] to respond, [s] to skip          ║
╚══════════════════════════════════════════════════════════════╝
```

### Verbose Output Format
```
⠙ Iteration [1/10] █░░░░░░░░░░░░░░ 45s
──────────────────────────────────────────────────────────────
[06:15:32] Reading src/timeout/mod.rs
[06:15:33] Found TimeoutConfig struct at line 14
[06:15:34] Analyzing heartbeat_interval field
[06:15:35] Planning edit: change default 45s → 60s
[06:15:36] Executing Edit tool on src/timeout/mod.rs
──────────────────────────────────────────────────────────────
```

## Success Metrics

- Users can see agent activity within 1 second of it occurring
- Input-needed states are detected and surfaced within 2 seconds
- Zero false-positive stall timeouts due to legitimate input waits
- User satisfaction: "I know what Ralph is doing" (qualitative)
- Time-to-resolution reduced when issues occur (users can see problems immediately)

## Open Questions

1. Should streaming be on by default, or require explicit `--verbose`?
2. What's the right debounce interval for activity indicator updates?
3. Should we show a preview of the last N lines even when streaming is off?
4. How do we handle very long lines in the activity summary (truncate vs wrap)?
5. Should input-needed detection work with all agent types or just Claude?
