# PRD: Parallel Execution Terminal UI

## Introduction

Add rich terminal UI features to Ralph's parallel execution mode to match the existing sequential run experience. Currently, parallel execution uses only `eprintln!()` for output, while sequential mode has a polished TUI with spinners, progress bars, and status icons. This creates an inconsistent user experience between the two modes.

The solution uses `indicatif::MultiProgress` for thread-safe concurrent progress management combined with existing widget rendering for consistent styling.

## Goals

- Provide real-time visual feedback for all in-flight stories during parallel execution
- Show iteration progress with spinners and progress bars for each story
- Display pending stories with dependency/conflict blocking information
- Track completed stories with timing and commit information
- Support keyboard toggles (streaming, expand, quit) consistent with sequential mode
- Maintain visual consistency with the existing sequential execution UI

## User Stories

### US-001: Create parallel UI event system
**Description:** As a developer, I need an event-based communication system between the scheduler and display so that UI updates are decoupled from execution logic.

**Acceptance Criteria:**
- [ ] Create `src/ui/parallel_events.rs` with `ParallelUIEvent` enum
- [ ] Define events: StoryStarted, IterationUpdate, GateUpdate, StoryCompleted, StoryFailed
- [ ] Define events: ConflictDeferred, ReconciliationStatus, SequentialRetryStarted
- [ ] Include `StoryDisplayInfo` struct and `StoryStatus` enum for tracking
- [ ] Add unit tests for event types
- [ ] Typecheck passes (`cargo check`)

### US-002: Create parallel display controller
**Description:** As a developer, I need a display controller that manages multiple concurrent progress indicators so that all in-flight stories show real-time progress.

**Acceptance Criteria:**
- [ ] Create `src/ui/parallel_display.rs` with `ParallelRunnerDisplay` struct
- [ ] Use `indicatif::MultiProgress` for thread-safe progress management
- [ ] Store story progress bars in `HashMap<String, ProgressBar>`
- [ ] Implement `new()` with theme and display options support
- [ ] Implement `init_stories()` to set up progress bars for all stories
- [ ] Integrate with existing `Theme` from `colors.rs`
- [ ] Typecheck passes (`cargo check`)

### US-003: Implement story lifecycle display methods
**Description:** As a user, I want to see when stories start, progress, and complete so that I can monitor parallel execution in real-time.

**Acceptance Criteria:**
- [ ] Implement `story_started()` - creates/shows spinner for story with braille animation
- [ ] Implement `update_iteration()` - updates progress bar position and percentage
- [ ] Implement `story_completed()` - marks complete with green checkmark and optional commit hash
- [ ] Implement `story_failed()` - marks failed with red X and error message
- [ ] Use consistent colors from Theme (success=green, error=red, in_progress=blue)
- [ ] Typecheck passes (`cargo check`)

### US-004: Implement header and status sections
**Description:** As a user, I want to see an overall summary header and organized status sections so that I can quickly understand execution progress.

**Acceptance Criteria:**
- [ ] Implement `render_header()` showing PRD name, agent, worker count
- [ ] Show overall progress bar: "Stories: [████░░░░] 4/10 (40%)"
- [ ] Display counts: running, completed, pending, failed
- [ ] Implement "In Flight" section with active spinners
- [ ] Implement "Pending" section showing blocked stories with blocking reason
- [ ] Implement "Completed" section (collapsible)
- [ ] Typecheck passes (`cargo check`)

### US-005: Add event channel to scheduler
**Description:** As a developer, I need the scheduler to send UI events through a channel so that the display can update independently of execution.

**Acceptance Criteria:**
- [ ] Add `ui_tx: Option<mpsc::Sender<ParallelUIEvent>>` field to `ParallelRunner`
- [ ] Create channel in `run()` method: `let (ui_tx, ui_rx) = mpsc::channel(100)`
- [ ] Initialize `ParallelRunnerDisplay` in `run()` when UI is enabled
- [ ] Spawn tokio task for event handling loop
- [ ] Typecheck passes (`cargo check`)

### US-006: Replace eprintln with UI events in scheduler
**Description:** As a developer, I need to replace debug print statements with proper UI events so that the display shows formatted output.

**Acceptance Criteria:**
- [ ] Replace conflict `eprintln!` (lines 339-344) with `ConflictDeferred` event
- [ ] Replace reconciliation `eprintln!` (line 503) with `ReconciliationStatus` event
- [ ] Replace sequential retry `eprintln!` (line 552) with `SequentialRetryStarted` event
- [ ] Send `StoryStarted` event when story begins execution
- [ ] Send `StoryCompleted`/`StoryFailed` events when story finishes
- [ ] Typecheck passes (`cargo check`)

### US-007: Implement iteration progress callbacks
**Description:** As a user, I want to see real-time iteration progress for each story so that I know how far along each story is.

**Acceptance Criteria:**
- [ ] Update executor callback from `|_iter, _max| {}` to send `IterationUpdate` events
- [ ] Progress bar updates in real-time as iterations complete
- [ ] Show format: `[2/10] ████░░` for each in-flight story
- [ ] Handle rapid updates without flickering
- [ ] Typecheck passes (`cargo check`)

### US-008: Integrate keyboard listener
**Description:** As a user, I want keyboard controls during parallel execution so that I can toggle options and quit gracefully.

**Acceptance Criteria:**
- [ ] Integrate `KeyboardListener` from `keyboard.rs`
- [ ] Create shared `ToggleState` for streaming and expand toggles
- [ ] Support 's' key to toggle streaming output visibility
- [ ] Support 'e' key to expand/collapse completed section
- [ ] Support 'q' key for graceful quit (finish current stories, then exit)
- [ ] Support Ctrl+C for immediate interrupt
- [ ] Show toggle hint bar: `[s] stream: off | [e] expand: off | [q] quit`
- [ ] Typecheck passes (`cargo check`)

### US-009: Update module exports
**Description:** As a developer, I need the new modules properly exported so they can be used by the scheduler and other components.

**Acceptance Criteria:**
- [ ] Add `pub mod parallel_display;` to `src/ui/mod.rs`
- [ ] Add `pub mod parallel_events;` to `src/ui/mod.rs`
- [ ] Add public re-exports for key types (ParallelRunnerDisplay, ParallelUIEvent)
- [ ] Update `src/parallel/mod.rs` to re-export UI events for external use
- [ ] Typecheck passes (`cargo check`)

### US-010: Support quiet mode and display options
**Description:** As a user, I want parallel mode to respect the `-q` quiet flag and display options so that output can be controlled.

**Acceptance Criteria:**
- [ ] Check `display_options.ui_mode` before initializing display
- [ ] Skip UI rendering when `UiMode::Quiet` is set
- [ ] Respect `--max-concurrency` flag for worker count display
- [ ] Fall back to simple output when terminal doesn't support colors
- [ ] Typecheck passes (`cargo check`)

## Functional Requirements

- FR-1: The parallel display must use `indicatif::MultiProgress` for thread-safe concurrent progress
- FR-2: Each in-flight story must show a spinner with braille animation pattern
- FR-3: Progress bars must update in real-time without screen flicker
- FR-4: The display must show story ID, priority, title, and progress for each story
- FR-5: Completed stories must show green checkmark with optional commit hash
- FR-6: Failed stories must show red X with error message
- FR-7: Pending stories must show blocking reason (dependency or conflict)
- FR-8: The header must show overall progress percentage and counts
- FR-9: Keyboard toggles must work identically to sequential mode
- FR-10: The display must gracefully handle terminal resize
- FR-11: Events must be sent via async channel without blocking execution
- FR-12: Display must use the same color theme as sequential mode

## Non-Goals

- No full-screen ratatui TUI (too complex for this iteration)
- No per-story streaming output panel (would require significant refactoring)
- No mouse support
- No history scrollback for completed stories
- No persistent log file of UI output

## Technical Considerations

### Components to Reuse
| Component | File | Usage |
|-----------|------|-------|
| `ProgressManager` | `src/ui/spinner.rs:431` | MultiProgress wrapper |
| `RalphSpinner` | `src/ui/spinner.rs:139` | Per-story spinner |
| `IterationProgress` | `src/ui/spinner.rs:257` | Progress bar rendering |
| `ToggleState` | `src/ui/keyboard.rs` | Thread-safe toggle state |
| `KeyboardListener` | `src/ui/keyboard.rs` | Keyboard event handling |
| `Theme` | `src/ui/colors.rs` | Consistent color scheme |

### Architecture
- Use `tokio::sync::mpsc` channel for event communication (capacity 100)
- Display controller runs in separate tokio task
- Scheduler sends events, display receives and renders
- Shared `Arc<ToggleState>` between keyboard listener and display

### Display Layout
```
================== RALPH PARALLEL EXECUTION ==================

PRD: prd.json  |  Agent: claude  |  Workers: 3/3

Stories: [████████░░░░░░░░░░] 4/10 (40%)  | 3 running | 4 done

----- IN FLIGHT -----
  * US-003 [P1] Add user authentication       [2/10] ====--
  * US-005 [P2] Implement search              [4/10] ========
  * US-007 [P2] Add pagination                [1/10] ==

----- PENDING (3) -----
  o US-008 [P3] User profile       <- blocked by US-003
  o US-009 [P3] Settings           <- blocked by US-003

----- COMPLETED (4) -----
  v US-001 Setup project
  v US-002 Database models

[s] stream: off | [e] expand: off | [q] quit
```

## Success Metrics

- Parallel mode UI provides equivalent information to sequential mode
- All in-flight stories visible with real-time progress
- No visual regressions from existing sequential mode
- Keyboard toggles respond within 100ms
- No performance degradation from UI overhead

## Open Questions

- Should completed section auto-collapse after N items?
- Should we show estimated time remaining based on iteration progress?
- Should gate status (build/test/lint) be shown per story?
