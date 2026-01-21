//! Parallel execution scheduler

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch, Mutex, RwLock, Semaphore};

use crate::checkpoint::{Checkpoint, CheckpointManager, PauseReason, StoryCheckpoint};
use crate::error::classification::ErrorCategory;
use crate::evidence::{error_category_label, generate_run_id, EvidenceWriter};
use crate::mcp::tools::executor::{detect_agent, ExecutorConfig, StoryExecutor};
use crate::mcp::tools::load_prd::{validate_prd, PrdFile};
use crate::metrics::{RunMetricsCollector, RunMetricsStore};
use crate::parallel::dependency::{DependencyGraph, StoryNode};
use crate::parallel::reconcile::{ReconciliationEngine, ReconciliationIssue, ReconciliationResult};
use crate::runner::{RunResult, RunnerConfig};
use crate::timeout::TimeoutConfig;
use crate::ui::parallel_display::ParallelRunnerDisplay;
use crate::ui::parallel_events::{ParallelUIEvent, StoryDisplayInfo};

/// Strategy for detecting conflicts between parallel story executions.
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ConflictStrategy {
    /// Detect conflicts based on file paths (target_files).
    /// Stories modifying the same files cannot run concurrently.
    #[default]
    FileBased,
    /// Detect conflicts based on entity references.
    /// Stories referencing the same entities cannot run concurrently.
    EntityBased,
    /// No conflict detection. All ready stories can run concurrently.
    None,
}

/// How to handle backpressure when the parallel queue is full.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueuePolicy {
    /// Wait for queue capacity to free up.
    Block,
    /// Reject new stories when the queue is full.
    Reject,
    /// Drop the oldest queued story to make room.
    DropOldest,
}

impl QueuePolicy {
    fn as_label(&self) -> &'static str {
        match self {
            QueuePolicy::Block => "block",
            QueuePolicy::Reject => "reject",
            QueuePolicy::DropOldest => "drop_oldest",
        }
    }
}

/// Configuration options for parallel story execution.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct ParallelRunnerConfig {
    /// Maximum number of stories to execute concurrently.
    pub max_concurrency: u32,
    /// Maximum number of stories allowed in the pending queue.
    pub queue_capacity: usize,
    /// Backpressure policy when the queue is full.
    pub queue_policy: QueuePolicy,
    /// Wait duration between queue capacity checks when blocking.
    pub queue_wait: Duration,
    /// Whether to automatically infer dependencies from file patterns.
    pub infer_dependencies: bool,
    /// Whether to fall back to sequential execution on errors.
    pub fallback_to_sequential: bool,
    /// Strategy for detecting conflicts between parallel stories.
    pub conflict_strategy: ConflictStrategy,
    /// Timeout configuration for execution limits.
    pub timeout_config: TimeoutConfig,
    /// Timeout for an entire batch of parallel executions.
    /// If a batch takes longer than this, remaining tasks are cancelled.
    /// Default: 30 minutes.
    pub batch_timeout: Duration,
    /// Number of consecutive failures before circuit breaker triggers.
    /// Default: 5.
    pub circuit_breaker_threshold: u32,
}

impl Default for ParallelRunnerConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 3,
            queue_capacity: 32,
            queue_policy: QueuePolicy::Block,
            queue_wait: Duration::from_millis(200),
            infer_dependencies: true,
            fallback_to_sequential: true,
            conflict_strategy: ConflictStrategy::default(),
            timeout_config: TimeoutConfig::default(),
            batch_timeout: Duration::from_secs(1800), // 30 minutes
            circuit_breaker_threshold: 5,
        }
    }
}

/// Tracks execution state across parallel story executions.
///
/// This struct maintains the runtime state for the parallel scheduler,
/// including which stories are currently executing, which have completed,
/// which have failed, and which files are locked.
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct ParallelExecutionState {
    /// Stories currently being executed, mapped by story ID.
    pub in_flight: HashSet<String>,
    /// Stories that have completed successfully, mapped by story ID.
    pub completed: HashSet<String>,
    /// Stories that have failed, mapped by story ID to error message.
    pub failed: HashMap<String, String>,
    /// Files currently locked by stories, mapped from file path to story ID.
    pub locked_files: HashMap<PathBuf, String>,
}

impl ParallelExecutionState {
    /// Attempts to acquire locks on the given file patterns for a story.
    ///
    /// Returns `true` if all locks were acquired successfully, `false` if any
    /// file is already locked by another story. If acquisition fails, no locks
    /// are added (atomic behavior).
    ///
    /// # Arguments
    ///
    /// * `story_id` - The ID of the story requesting the locks
    /// * `target_files` - List of file patterns that the story will modify
    pub fn acquire_locks(&mut self, story_id: &str, target_files: &[String]) -> bool {
        // First, check if any file is already locked by another story
        for file_pattern in target_files {
            let path = PathBuf::from(file_pattern);
            if let Some(locking_story) = self.locked_files.get(&path) {
                if locking_story != story_id {
                    // File is locked by another story
                    return false;
                }
            }
        }

        // All files are available, acquire all locks
        for file_pattern in target_files {
            let path = PathBuf::from(file_pattern);
            self.locked_files.insert(path, story_id.to_string());
        }

        true
    }

    /// Releases all file locks held by a story.
    ///
    /// This should be called when a story completes (success or failure).
    ///
    /// # Arguments
    ///
    /// * `story_id` - The ID of the story releasing its locks
    pub fn release_locks(&mut self, story_id: &str) {
        self.locked_files
            .retain(|_path, locking_story| locking_story != story_id);
    }
}

/// Detects pre-execution conflicts between ready stories based on overlapping target files.
///
/// Returns a list of story ID pairs that conflict (have overlapping target_files).
/// The first element of each pair is always the lower-priority story (higher priority number).
fn detect_preexecution_conflicts(stories: &[StoryNode]) -> Vec<(String, String)> {
    let mut conflicts = Vec::new();

    for i in 0..stories.len() {
        for j in (i + 1)..stories.len() {
            let story_a = &stories[i];
            let story_b = &stories[j];

            // Check for overlapping target_files
            let files_a: HashSet<&String> = story_a.target_files.iter().collect();
            let files_b: HashSet<&String> = story_b.target_files.iter().collect();

            if !files_a.is_disjoint(&files_b) {
                // There is an overlap - determine which is lower priority
                // Lower priority number = higher priority
                if story_a.priority > story_b.priority {
                    // story_a has lower priority (higher number), story_b runs first
                    conflicts.push((story_a.id.clone(), story_b.id.clone()));
                } else if story_b.priority > story_a.priority {
                    // story_b has lower priority (higher number), story_a runs first
                    conflicts.push((story_b.id.clone(), story_a.id.clone()));
                } else {
                    // Same priority - use lexicographic order as tiebreaker
                    // Earlier ID runs first
                    if story_a.id > story_b.id {
                        conflicts.push((story_a.id.clone(), story_b.id.clone()));
                    } else {
                        conflicts.push((story_b.id.clone(), story_a.id.clone()));
                    }
                }
            }
        }
    }

    conflicts
}

/// Filters ready stories to avoid pre-execution conflicts.
///
/// When two stories have overlapping target_files, only the higher-priority story
/// (lower priority number) is included in this batch. The lower-priority story
/// is deferred to a subsequent batch.
///
/// Returns a tuple of (stories to run this batch, deferred story IDs for logging).
fn filter_conflicting_stories(stories: Vec<StoryNode>) -> (Vec<StoryNode>, Vec<(String, String)>) {
    if stories.is_empty() {
        return (stories, Vec::new());
    }

    let conflicts = detect_preexecution_conflicts(&stories);

    if conflicts.is_empty() {
        return (stories, Vec::new());
    }

    // Collect IDs of stories that should be deferred (lower-priority stories in conflicts)
    let deferred_ids: HashSet<String> = conflicts
        .iter()
        .map(|(deferred, _)| deferred.clone())
        .collect();

    // Filter out deferred stories
    let filtered: Vec<StoryNode> = stories
        .into_iter()
        .filter(|s| !deferred_ids.contains(&s.id))
        .collect();

    (filtered, conflicts)
}

/// The main parallel runner that executes multiple stories concurrently.
///
/// This struct manages parallel story execution with concurrency limiting
/// via a semaphore and shared execution state protected by a read-write lock.
#[allow(dead_code)]
pub struct ParallelRunner {
    /// Configuration for parallel execution settings.
    config: ParallelRunnerConfig,
    /// Base runner configuration (paths, limits, etc.).
    base_config: RunnerConfig,
    /// Semaphore for limiting concurrent story executions.
    semaphore: Arc<Semaphore>,
    /// Shared execution state tracking in-flight, completed, and failed stories.
    execution_state: Arc<RwLock<ParallelExecutionState>>,
    /// Mutex to serialize git operations across parallel stories.
    git_mutex: Arc<Mutex<()>>,
    /// Optional channel sender for UI events during parallel execution.
    ui_tx: Option<mpsc::Sender<ParallelUIEvent>>,
    /// Optional checkpoint manager for circuit breaker persistence.
    checkpoint_manager: Option<CheckpointManager>,
}

#[allow(dead_code)]
impl ParallelRunner {
    /// Create a new parallel runner with the given configurations.
    ///
    /// # Arguments
    /// * `config` - Parallel execution settings (concurrency, inference, fallback)
    /// * `base_config` - Base runner configuration (paths, iteration limits)
    pub fn new(config: ParallelRunnerConfig, base_config: RunnerConfig) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrency as usize));
        let execution_state = Arc::new(RwLock::new(ParallelExecutionState::default()));
        let git_mutex = Arc::new(Mutex::new(()));

        // Initialize checkpoint manager if checkpointing is enabled
        let checkpoint_manager = if base_config.no_checkpoint {
            None
        } else {
            match CheckpointManager::new(&base_config.working_dir) {
                Ok(manager) => Some(manager),
                Err(e) => {
                    eprintln!("Warning: Failed to initialize checkpoint manager: {}", e);
                    None
                }
            }
        };

        Self {
            config,
            base_config,
            semaphore,
            execution_state,
            git_mutex,
            ui_tx: None,
            checkpoint_manager,
        }
    }

    /// Run all stories in parallel until all pass or an error occurs.
    ///
    /// This method implements the main parallel execution loop:
    /// 1. Loads the PRD and builds a DependencyGraph
    /// 2. Optionally infers dependencies from target files
    /// 3. Spawns concurrent tasks for ready stories (limited by semaphore)
    /// 4. Waits for any task to complete and updates state
    /// 5. Repeats until all stories pass or cannot make progress
    pub async fn run(&self) -> RunResult {
        let run_id = generate_run_id();
        let run_metrics = RunMetricsCollector::new(run_id.clone(), 0);
        let metrics_store = match RunMetricsStore::new(&self.base_config.working_dir) {
            Ok(store) => Some(store),
            Err(err) => {
                eprintln!("Warning: Failed to initialize run metrics store: {}", err);
                None
            }
        };
        let save_metrics = |collector: &RunMetricsCollector| {
            if let Some(store) = metrics_store.as_ref() {
                let metrics = collector.finish();
                if let Err(err) = store.save(&metrics) {
                    eprintln!("Warning: Failed to save run metrics: {}", err);
                }
            }
        };

        let evidence = match EvidenceWriter::try_new(&self.base_config.working_dir, run_id.clone())
        {
            Ok(mut writer) => {
                writer.emit_run_start();
                Some(Arc::new(Mutex::new(writer)))
            }
            Err(err) => {
                eprintln!("Warning: Failed to initialize evidence writer: {}", err);
                None
            }
        };

        // Load and validate PRD
        let prd = match self.load_prd() {
            Ok(prd) => prd,
            Err(e) => {
                emit_run_complete(
                    &evidence,
                    "failed",
                    Some("fatal".to_string()),
                    Some(format!("Failed to load PRD: {}", e)),
                )
                .await;
                save_metrics(&run_metrics);
                return RunResult {
                    all_passed: false,
                    stories_passed: 0,
                    total_stories: 0,
                    total_iterations: 0,
                    error: Some(format!("Failed to load PRD: {}", e)),
                };
            }
        };

        let total_stories = prd.user_stories.len();

        // Build dependency graph
        let mut graph = DependencyGraph::from_stories(&prd.user_stories);

        // Optionally infer dependencies from file patterns
        if self.config.infer_dependencies {
            graph.infer_dependencies();
        }

        // Validate graph for cycles
        if let Err(e) = graph.validate() {
            emit_run_complete(
                &evidence,
                "failed",
                Some("fatal".to_string()),
                Some(format!("Invalid dependency graph: {}", e)),
            )
            .await;
            save_metrics(&run_metrics);
            return RunResult {
                all_passed: false,
                stories_passed: 0,
                total_stories,
                total_iterations: 0,
                error: Some(format!("Invalid dependency graph: {}", e)),
            };
        }

        // Count already passing stories
        let initially_passing: HashSet<String> = prd
            .user_stories
            .iter()
            .filter(|s| s.passes)
            .map(|s| s.id.clone())
            .collect();
        let expected_steps = total_stories.saturating_sub(initially_passing.len());
        run_metrics.set_expected_steps(expected_steps);

        // Initialize completed set with already passing stories
        {
            let mut state = self.execution_state.write().await;
            state.completed = initially_passing.clone();
        }

        // Check if all stories already pass - no agent needed in this case
        if initially_passing.len() == total_stories {
            // Show completion message for parallel mode
            if !self.base_config.display_options.quiet {
                use crate::ui::parallel_display::ParallelRunnerDisplay;
                let display = ParallelRunnerDisplay::with_display_options(
                    self.base_config.display_options.clone(),
                );
                display.display_completion(total_stories, total_stories, 0);
            }
            emit_run_complete(&evidence, "success", None, None).await;
            save_metrics(&run_metrics);
            return RunResult {
                all_passed: true,
                stories_passed: total_stories,
                total_stories,
                total_iterations: 0,
                error: None,
            };
        }

        // Detect agent (only needed if there are failing stories)
        let agent = match self.base_config.agent_command.clone().or_else(detect_agent) {
            Some(a) => a,
            None => {
                emit_run_complete(
                    &evidence,
                    "failed",
                    Some("fatal".to_string()),
                    Some(
                        "No agent found. Install Claude Code CLI, Codex CLI, or Amp CLI."
                            .to_string(),
                    ),
                )
                .await;
                save_metrics(&run_metrics);
                return RunResult {
                    all_passed: false,
                    stories_passed: initially_passing.len(),
                    total_stories,
                    total_iterations: 0,
                    error: Some(
                        "No agent found. Install Claude Code CLI, Codex CLI, or Amp CLI."
                            .to_string(),
                    ),
                };
            }
        };

        let mut total_iterations: u32 = 0;

        // Check if UI should be enabled based on display options
        // Skip UI rendering when quiet mode is set or UI mode is disabled
        let should_enable_ui = !self.base_config.display_options.quiet
            && self.base_config.display_options.should_enable_rich_ui();

        // Create UI channel and spawn event handler if UI is enabled
        let (ui_tx, ui_rx) = mpsc::channel::<ParallelUIEvent>(100);
        let _ui_handle = if should_enable_ui {
            let mut display = ParallelRunnerDisplay::with_display_options(
                self.base_config.display_options.clone(),
            );
            // Set the max workers for display
            display.set_max_workers(self.config.max_concurrency);
            display.set_queue_info(
                self.config.queue_capacity,
                self.config.queue_policy.as_label(),
            );

            // Initialize display with story information
            let story_infos: Vec<_> = prd
                .user_stories
                .iter()
                .map(|s| {
                    crate::ui::parallel_events::StoryDisplayInfo::new(&s.id, &s.title, s.priority)
                })
                .collect();
            display.init_stories(&story_infos);

            // Spawn event handling task
            Some(tokio::spawn(async move {
                let mut rx = ui_rx;
                while let Some(event) = rx.recv().await {
                    match &event {
                        ParallelUIEvent::StoryStarted {
                            story,
                            iteration,
                            concurrent_count: _,
                        } => {
                            display.story_started(
                                &story.id,
                                &story.title,
                                *iteration,
                                5, // Default max iterations
                            );
                        }
                        ParallelUIEvent::IterationUpdate {
                            story_id,
                            iteration,
                            max_iterations,
                            message: _,
                        } => {
                            // We need story title - use story_id as fallback
                            display.update_iteration(
                                story_id,
                                story_id,
                                *iteration,
                                *max_iterations,
                            );
                        }
                        ParallelUIEvent::StoryCompleted {
                            story_id,
                            iterations_used,
                            duration_ms: _,
                        } => {
                            display.story_completed(story_id, story_id, *iterations_used, None);
                        }
                        ParallelUIEvent::StoryFailed {
                            story_id,
                            error,
                            iteration: _,
                        } => {
                            display.story_failed(story_id, story_id, error);
                        }
                        ParallelUIEvent::QueueStatus {
                            queued,
                            capacity,
                            policy,
                        } => {
                            display.display_queue_status(*queued, *capacity, policy);
                        }
                        ParallelUIEvent::ConflictDeferred {
                            story_id,
                            blocking_story_id,
                            conflicting_files: _,
                        } => {
                            display.story_deferred(story_id, story_id, blocking_story_id);
                        }
                        ParallelUIEvent::SequentialRetryStarted { story_id, reason } => {
                            display.story_sequential_retry(story_id, story_id, reason);
                        }
                        ParallelUIEvent::GateUpdate { .. }
                        | ParallelUIEvent::ReconciliationStatus { .. } => {
                            // These events don't have direct display methods yet
                        }
                        ParallelUIEvent::KeyboardToggle { .. }
                        | ParallelUIEvent::GracefulQuitRequested
                        | ParallelUIEvent::ImmediateInterrupt => {
                            // Keyboard events are handled separately by the keyboard listener
                        }
                    }
                }
            }))
        } else {
            // Drop the receiver if UI is not enabled
            drop(ui_rx);
            None
        };

        // Store sender for use in spawned tasks (only if UI enabled)
        let ui_sender: Option<mpsc::Sender<ParallelUIEvent>> = if should_enable_ui {
            Some(ui_tx)
        } else {
            drop(ui_tx);
            None
        };

        // Build story info lookup for event creation
        let story_info_map: HashMap<String, StoryDisplayInfo> = prd
            .user_stories
            .iter()
            .map(|s| {
                (
                    s.id.clone(),
                    StoryDisplayInfo::new(&s.id, &s.title, s.priority),
                )
            })
            .collect();

        // Circuit breaker: track cumulative failures across batches
        let mut cumulative_failures: u32 = 0;
        let circuit_breaker_threshold = self.config.circuit_breaker_threshold;

        // Shared cancel channel for graceful shutdown when circuit breaker triggers
        let (cancel_tx, _cancel_rx) = watch::channel(false);
        let cancel_tx = Arc::new(cancel_tx);

        // Main execution loop
        let mut pending_queue: VecDeque<StoryNode> = VecDeque::new();
        let mut queued_ids: HashSet<String> = HashSet::new();
        let mut last_queue_size: Option<usize> = None;
        loop {
            // Get current state snapshot
            let state = self.execution_state.read().await;
            let completed = state.completed.clone();
            let in_flight = state.in_flight.clone();
            drop(state);

            // Get stories ready to execute (dependencies satisfied, not completed, not in flight)
            // Keep the full StoryNode so we have access to target_files for locking
            let ready_stories: Vec<_> = graph
                .get_ready_stories(&completed)
                .into_iter()
                .filter(|s| !in_flight.contains(&s.id) && !queued_ids.contains(&s.id))
                .cloned()
                .collect();

            // Pre-execution conflict detection: filter out lower-priority stories
            // that have overlapping target_files with higher-priority stories
            let (ready_stories, conflicts) = filter_conflicting_stories(ready_stories);
            let ready_empty = ready_stories.is_empty();

            // Send ConflictDeferred events when stories are deferred due to conflicts
            for (deferred_id, higher_priority_id) in &conflicts {
                if let Some(ref sender) = ui_sender {
                    // Find conflicting files between the two stories
                    let deferred_story = graph.get_story(deferred_id);
                    let blocking_story = graph.get_story(higher_priority_id);
                    let conflicting_files: Vec<PathBuf> = if let (Some(deferred), Some(blocking)) =
                        (deferred_story, blocking_story)
                    {
                        let deferred_files: HashSet<&String> =
                            deferred.target_files.iter().collect();
                        blocking
                            .target_files
                            .iter()
                            .filter(|f| deferred_files.contains(f))
                            .map(PathBuf::from)
                            .collect()
                    } else {
                        Vec::new()
                    };

                    let event = ParallelUIEvent::ConflictDeferred {
                        story_id: deferred_id.clone(),
                        blocking_story_id: higher_priority_id.clone(),
                        conflicting_files,
                    };
                    let _ = sender.try_send(event);
                }
            }

            // Enqueue ready stories with backpressure handling
            let mut blocked_on_queue = false;
            for story in ready_stories {
                if pending_queue.len() >= self.config.queue_capacity {
                    match self.config.queue_policy {
                        QueuePolicy::Block => {
                            blocked_on_queue = true;
                            break;
                        }
                        QueuePolicy::Reject => {
                            let mut state = self.execution_state.write().await;
                            state.failed.insert(
                                story.id.clone(),
                                "Queue full - rejected by backpressure policy".to_string(),
                            );
                            run_metrics.start_step(&story.id);
                            run_metrics.complete_step(
                                &story.id,
                                false,
                                1,
                                Duration::ZERO,
                                Some("Queue full - rejected by backpressure policy".to_string()),
                            );
                            emit_step_event(
                                &evidence,
                                &run_metrics,
                                &story.id,
                                "failed",
                                Some("queue_full".to_string()),
                                Some("Queue full - rejected by backpressure policy".to_string()),
                            )
                            .await;
                            if let Some(ref sender) = ui_sender {
                                let event = ParallelUIEvent::StoryFailed {
                                    story_id: story.id.clone(),
                                    error: "Queue full - rejected".to_string(),
                                    iteration: 0,
                                };
                                let _ = sender.try_send(event);
                            }
                            continue;
                        }
                        QueuePolicy::DropOldest => {
                            if let Some(dropped) = pending_queue.pop_front() {
                                queued_ids.remove(&dropped.id);
                                let mut state = self.execution_state.write().await;
                                state.failed.insert(
                                    dropped.id.clone(),
                                    "Queue full - dropped oldest".to_string(),
                                );
                                run_metrics.start_step(&dropped.id);
                                run_metrics.complete_step(
                                    &dropped.id,
                                    false,
                                    1,
                                    Duration::ZERO,
                                    Some("Queue full - dropped oldest".to_string()),
                                );
                                emit_step_event(
                                    &evidence,
                                    &run_metrics,
                                    &dropped.id,
                                    "failed",
                                    Some("queue_full".to_string()),
                                    Some("Queue full - dropped oldest".to_string()),
                                )
                                .await;
                                if let Some(ref sender) = ui_sender {
                                    let event = ParallelUIEvent::StoryFailed {
                                        story_id: dropped.id.clone(),
                                        error: "Queue full - dropped oldest".to_string(),
                                        iteration: 0,
                                    };
                                    let _ = sender.try_send(event);
                                }
                            }
                        }
                    }
                }

                queued_ids.insert(story.id.clone());
                pending_queue.push_back(story);
            }

            if let Some(ref sender) = ui_sender {
                let queue_size = pending_queue.len();
                if last_queue_size != Some(queue_size) {
                    last_queue_size = Some(queue_size);
                    let event = ParallelUIEvent::QueueStatus {
                        queued: queue_size,
                        capacity: self.config.queue_capacity,
                        policy: self.config.queue_policy.as_label().to_string(),
                    };
                    let _ = sender.try_send(event);
                }
            }

            if blocked_on_queue {
                tokio::time::sleep(self.config.queue_wait).await;
                continue;
            }

            // Check if we're done or stuck
            if ready_empty && in_flight.is_empty() {
                // No more stories to run and none in flight
                let state = self.execution_state.read().await;
                let stories_passed = state.completed.len();
                let has_failures = !state.failed.is_empty();
                drop(state);

                emit_run_complete(
                    &evidence,
                    if has_failures { "failed" } else { "success" },
                    if has_failures {
                        Some("failed_steps".to_string())
                    } else {
                        None
                    },
                    if has_failures {
                        Some("Some stories failed".to_string())
                    } else {
                        None
                    },
                )
                .await;
                save_metrics(&run_metrics);
                return RunResult {
                    all_passed: stories_passed == total_stories,
                    stories_passed,
                    total_stories,
                    total_iterations,
                    error: if has_failures {
                        Some("Some stories failed".to_string())
                    } else {
                        None
                    },
                };
            }

            // If nothing queued and some in flight, wait for them
            if pending_queue.is_empty() && !in_flight.is_empty() {
                // Wait for any in-flight task to complete
                tokio::task::yield_now().await;
                continue;
            }
            if pending_queue.is_empty() {
                tokio::task::yield_now().await;
                continue;
            }

            if self.semaphore.available_permits() == 0 {
                tokio::time::sleep(self.config.queue_wait).await;
                continue;
            }

            // Spawn tasks for queued stories (up to available semaphore permits)
            let mut handles = Vec::new();
            let mut dispatch_slots = self.semaphore.available_permits();

            while dispatch_slots > 0 {
                let story = match pending_queue.pop_front() {
                    Some(story) => story,
                    None => break,
                };
                queued_ids.remove(&story.id);

                let story_id = story.id.clone();
                let target_files = story.target_files.clone();

                let permit = self.semaphore.clone().acquire_owned().await;

                // Try to acquire file locks; requeue if files are locked
                {
                    let mut state = self.execution_state.write().await;
                    if !state.acquire_locks(&story_id, &target_files) {
                        drop(permit);
                        pending_queue.push_back(story);
                        queued_ids.insert(story_id.clone());
                        dispatch_slots = dispatch_slots.saturating_sub(1);
                        continue;
                    }
                    // Mark story as in-flight
                    state.in_flight.insert(story_id.clone());
                }

                let concurrent_count = {
                    let state = self.execution_state.read().await;
                    state.in_flight.len()
                };

                // Clone values for the spawned task
                let executor_config = ExecutorConfig {
                    prd_path: self.base_config.prd_path.clone(),
                    project_root: self.base_config.working_dir.clone(),
                    progress_path: self.base_config.working_dir.join("progress.txt"),
                    quality_profile: None,
                    agent_command: agent.clone(),
                    max_iterations: self.base_config.max_iterations_per_story,
                    git_mutex: Some(self.git_mutex.clone()),
                    timeout_config: self.config.timeout_config.clone(),
                    ..Default::default()
                };

                let execution_state = self.execution_state.clone();
                let story_id_clone = story_id.clone();
                let task_ui_sender = ui_sender.clone();
                let story_info = story_info_map
                    .get(&story_id)
                    .cloned()
                    .unwrap_or_else(|| StoryDisplayInfo::new(&story_id, &story_id, story.priority));

                // Subscribe to shared cancel channel for circuit breaker graceful shutdown
                let task_cancel_rx = cancel_tx.subscribe();

                let task_evidence = evidence.clone();
                let task_run_metrics = run_metrics.clone();
                let handle = tokio::spawn(async move {
                    // Hold the permit until the task completes (RAII)
                    let _permit = permit;

                    // Send StoryStarted event
                    let start_time = Instant::now();
                    task_run_metrics.start_step(&story_id_clone);
                    if let Some(ref sender) = task_ui_sender {
                        let event = ParallelUIEvent::StoryStarted {
                            story: story_info.clone(),
                            iteration: 1,
                            concurrent_count,
                        };
                        let _ = sender.try_send(event);
                    }

                    let executor = StoryExecutor::new(executor_config);
                    let cancel_rx = task_cancel_rx;

                    // Clone for iteration callback closure
                    let iter_story_id = story_id_clone.clone();
                    let iter_ui_sender = task_ui_sender.clone();

                    let result = executor
                        .execute_story(&story_id_clone, cancel_rx, |iter, max| {
                            if let Some(ref sender) = iter_ui_sender {
                                let event = ParallelUIEvent::IterationUpdate {
                                    story_id: iter_story_id.clone(),
                                    iteration: iter,
                                    max_iterations: max,
                                    message: None,
                                };
                                let _ = sender.try_send(event);
                            }
                        })
                        .await;

                    let duration = start_time.elapsed();
                    let duration_ms = duration.as_millis() as u64;

                    // Update state based on result
                    let mut state = execution_state.write().await;
                    state.in_flight.remove(&story_id_clone);
                    // Release file locks (success or failure)
                    state.release_locks(&story_id_clone);

                    // Result tuple: (story_id, success, iterations, is_transient_failure)
                    // is_transient_failure is true only for transient errors (not quality gate failures)
                    let (result_tuple, step_event) = match result {
                        Ok(exec_result) if exec_result.success => {
                            state.completed.insert(story_id_clone.clone());
                            // Send StoryCompleted event
                            if let Some(ref sender) = task_ui_sender {
                                let event = ParallelUIEvent::StoryCompleted {
                                    story_id: story_id_clone.clone(),
                                    iterations_used: exec_result.iterations_used,
                                    duration_ms,
                                };
                                let _ = sender.try_send(event);
                            }
                            let attempts = exec_result.iterations_used.max(1);
                            task_run_metrics.complete_step(
                                &story_id_clone,
                                true,
                                attempts,
                                duration,
                                None,
                            );
                            (
                                (story_id_clone, true, exec_result.iterations_used, false),
                                Some(("completed".to_string(), None, None)),
                            )
                        }
                        Ok(exec_result) => {
                            // Quality gate failure - this is NOT transient (agent ran but tests failed)
                            let error_msg = exec_result
                                .error
                                .unwrap_or_else(|| "Unknown error".to_string());
                            state
                                .failed
                                .insert(story_id_clone.clone(), error_msg.clone());
                            // Send StoryFailed event
                            if let Some(ref sender) = task_ui_sender {
                                let event = ParallelUIEvent::StoryFailed {
                                    story_id: story_id_clone.clone(),
                                    error: error_msg.clone(),
                                    iteration: exec_result.iterations_used,
                                };
                                let _ = sender.try_send(event);
                            }
                            let attempts = exec_result.iterations_used.max(1);
                            task_run_metrics.complete_step(
                                &story_id_clone,
                                false,
                                attempts,
                                duration,
                                Some(error_msg.clone()),
                            );
                            (
                                (story_id_clone, false, exec_result.iterations_used, false),
                                Some((
                                    "failed".to_string(),
                                    Some("quality_gates_failed".to_string()),
                                    Some(error_msg),
                                )),
                            )
                        }
                        Err(e) => {
                            state.failed.insert(story_id_clone.clone(), e.to_string());
                            // Send StoryFailed event
                            if let Some(ref sender) = task_ui_sender {
                                let event = ParallelUIEvent::StoryFailed {
                                    story_id: story_id_clone.clone(),
                                    error: e.to_string(),
                                    iteration: 1,
                                };
                                let _ = sender.try_send(event);
                            }
                            let category = e.classify();
                            // Check if this is a transient error
                            let is_transient = matches!(category, ErrorCategory::Transient(_));
                            task_run_metrics.complete_step(
                                &story_id_clone,
                                false,
                                1,
                                duration,
                                Some(e.to_string()),
                            );
                            (
                                (story_id_clone, false, 1, is_transient),
                                Some((
                                    "failed".to_string(),
                                    Some(error_category_label(&category).to_string()),
                                    Some(e.to_string()),
                                )),
                            )
                        }
                    };
                    drop(state);

                    if let Some((status, error_type, error_message)) = step_event {
                        emit_step_event(
                            &task_evidence,
                            &task_run_metrics,
                            &result_tuple.0,
                            &status,
                            error_type,
                            error_message,
                        )
                        .await;
                    }
                    // Permit is dropped here, releasing the semaphore slot
                    result_tuple
                });

                handles.push(handle);
                dispatch_slots = dispatch_slots.saturating_sub(1);
            }

            // Wait for all tasks in this batch to complete (with timeout)
            if !handles.is_empty() {
                let batch_story_ids: Vec<String> = {
                    let state = self.execution_state.read().await;
                    state.in_flight.iter().cloned().collect()
                };

                // Wait for all handles to complete, with batch timeout
                let batch_future = futures::future::join_all(handles);
                let batch_result =
                    tokio::time::timeout(self.config.batch_timeout, batch_future).await;

                match batch_result {
                    Ok(results) => {
                        // Count non-transient failures in this batch for circuit breaker
                        let mut batch_non_transient_failures: u32 = 0;
                        for result in results.into_iter().flatten() {
                            let (_story_id, success, iterations, is_transient) = result;
                            total_iterations += iterations;
                            // Count non-transient failures (quality gate failures or fatal/timeout errors)
                            if !success && !is_transient {
                                batch_non_transient_failures += 1;
                            }
                        }
                        cumulative_failures += batch_non_transient_failures;

                        // Check circuit breaker threshold
                        if cumulative_failures >= circuit_breaker_threshold {
                            // Send cancel signal to any remaining in-flight stories
                            let _ = cancel_tx.send(true);

                            // Get a failed story ID for the checkpoint
                            let state = self.execution_state.read().await;
                            let failed_story_id = state
                                .failed
                                .keys()
                                .next()
                                .cloned()
                                .unwrap_or_else(|| "unknown".to_string());
                            drop(state);

                            // Save checkpoint with circuit breaker reason
                            self.save_checkpoint(
                                &failed_story_id,
                                1,
                                self.base_config.max_iterations_per_story,
                                PauseReason::CircuitBreakerTriggered {
                                    consecutive_failures: cumulative_failures,
                                    threshold: circuit_breaker_threshold,
                                },
                            );

                            let circuit_breaker_msg = format!(
                                "Circuit breaker triggered: {} failures across batches (threshold: {})",
                                cumulative_failures, circuit_breaker_threshold
                            );
                            println!("\n⚡ {}", circuit_breaker_msg);

                            let state = self.execution_state.read().await;
                            emit_run_complete(
                                &evidence,
                                "failed",
                                Some("circuit_breaker".to_string()),
                                Some(circuit_breaker_msg.clone()),
                            )
                            .await;
                            save_metrics(&run_metrics);
                            return RunResult {
                                all_passed: false,
                                stories_passed: state.completed.len(),
                                total_stories,
                                total_iterations,
                                error: Some(format!(
                                    "{}. Checkpoint saved. Resume with: ralph --resume",
                                    circuit_breaker_msg
                                )),
                            };
                        }
                    }
                    Err(_) => {
                        // Batch timed out - mark all in-flight stories as failed (non-transient)
                        let timed_out_count = batch_story_ids.len() as u32;
                        let mut state = self.execution_state.write().await;
                        for story_id in &batch_story_ids {
                            if state.in_flight.contains(story_id) {
                                state.in_flight.remove(story_id);
                                state.release_locks(story_id);
                                state.failed.insert(
                                    story_id.clone(),
                                    format!(
                                        "Batch timed out after {:?}",
                                        self.config.batch_timeout
                                    ),
                                );
                                emit_step_event(
                                    &evidence,
                                    &run_metrics,
                                    story_id,
                                    "failed",
                                    Some("batch_timeout".to_string()),
                                    Some(format!(
                                        "Batch timed out after {:?}",
                                        self.config.batch_timeout
                                    )),
                                )
                                .await;
                                // Send StoryFailed event
                                if let Some(ref sender) = ui_sender {
                                    let event = ParallelUIEvent::StoryFailed {
                                        story_id: story_id.clone(),
                                        error: format!(
                                            "Batch timed out after {:?}",
                                            self.config.batch_timeout
                                        ),
                                        iteration: 1,
                                    };
                                    let _ = sender.try_send(event);
                                }
                            }
                        }
                        drop(state);

                        // Batch timeouts are non-transient failures
                        cumulative_failures += timed_out_count;

                        // Check circuit breaker after timeout
                        if cumulative_failures >= circuit_breaker_threshold {
                            let _ = cancel_tx.send(true);

                            let failed_story_id = batch_story_ids
                                .first()
                                .cloned()
                                .unwrap_or_else(|| "unknown".to_string());

                            self.save_checkpoint(
                                &failed_story_id,
                                1,
                                self.base_config.max_iterations_per_story,
                                PauseReason::CircuitBreakerTriggered {
                                    consecutive_failures: cumulative_failures,
                                    threshold: circuit_breaker_threshold,
                                },
                            );

                            let circuit_breaker_msg = format!(
                                "Circuit breaker triggered: {} failures across batches (threshold: {})",
                                cumulative_failures, circuit_breaker_threshold
                            );
                            println!("\n⚡ {}", circuit_breaker_msg);

                            let state = self.execution_state.read().await;
                            emit_run_complete(
                                &evidence,
                                "failed",
                                Some("circuit_breaker".to_string()),
                                Some(circuit_breaker_msg.clone()),
                            )
                            .await;
                            save_metrics(&run_metrics);
                            return RunResult {
                                all_passed: false,
                                stories_passed: state.completed.len(),
                                total_stories,
                                total_iterations,
                                error: Some(format!(
                                    "{}. Checkpoint saved. Resume with: ralph --resume",
                                    circuit_breaker_msg
                                )),
                            };
                        }

                        // Continue to next iteration - some stories may have completed
                        // before the timeout and their results were handled in the spawned task
                        continue;
                    }
                }

                // Run reconciliation after each batch completes
                let reconciliation_result = self
                    .run_reconciliation(
                        &batch_story_ids,
                        &graph,
                        &agent,
                        &mut total_iterations,
                        &evidence,
                        &run_metrics,
                        &ui_sender,
                        &story_info_map,
                    )
                    .await;

                // If reconciliation failed and we couldn't recover, return error
                if let Some(error) = reconciliation_result {
                    let state = self.execution_state.read().await;
                    emit_run_complete(
                        &evidence,
                        "failed",
                        Some("reconciliation_failed".to_string()),
                        Some(error.clone()),
                    )
                    .await;
                    save_metrics(&run_metrics);
                    return RunResult {
                        all_passed: false,
                        stories_passed: state.completed.len(),
                        total_stories,
                        total_iterations,
                        error: Some(error),
                    };
                }
            }
        }
    }

    /// Runs reconciliation after a batch completes and handles any issues found.
    ///
    /// Returns `None` if reconciliation passed or issues were resolved via sequential retry.
    /// Returns `Some(error)` if reconciliation found issues that couldn't be resolved.
    #[allow(clippy::too_many_arguments)]
    async fn run_reconciliation(
        &self,
        batch_story_ids: &[String],
        graph: &DependencyGraph,
        agent: &str,
        total_iterations: &mut u32,
        evidence: &Option<Arc<Mutex<EvidenceWriter>>>,
        run_metrics: &RunMetricsCollector,
        ui_sender: &Option<mpsc::Sender<ParallelUIEvent>>,
        story_info_map: &HashMap<String, StoryDisplayInfo>,
    ) -> Option<String> {
        let engine = ReconciliationEngine::new(self.base_config.working_dir.clone());
        let result = engine.reconcile();

        match result {
            ReconciliationResult::Clean => {
                // Send ReconciliationStatus event for clean result
                if let Some(ref sender) = ui_sender {
                    let event = ParallelUIEvent::ReconciliationStatus {
                        success: true,
                        issues_count: 0,
                        message: "Clean after batch".to_string(),
                    };
                    let _ = sender.try_send(event);
                }
                None
            }
            ReconciliationResult::IssuesFound(issues) => {
                // Build issue summary message
                let issue_descriptions: Vec<String> = issues
                    .iter()
                    .map(|issue| match issue {
                        ReconciliationIssue::GitConflict { affected_files } => {
                            format!("git conflict in: {}", affected_files.join(", "))
                        }
                        ReconciliationIssue::TypeMismatch { file, error } => {
                            format!("type error in {}: {}", file, error)
                        }
                        ReconciliationIssue::ImportDuplicate => {
                            "duplicate import detected".to_string()
                        }
                    })
                    .collect();

                // Send ReconciliationStatus event for issues found
                if let Some(ref sender) = ui_sender {
                    let event = ParallelUIEvent::ReconciliationStatus {
                        success: false,
                        issues_count: issues.len(),
                        message: format!("Issues found: {}", issue_descriptions.join("; ")),
                    };
                    let _ = sender.try_send(event);
                }

                // If fallback is enabled, retry affected stories sequentially
                if self.config.fallback_to_sequential {
                    // Get affected stories - for now, we retry all stories from the batch
                    // that have issues (based on file overlap with issue files)
                    let affected_story_ids =
                        self.get_affected_stories(&issues, batch_story_ids, graph);

                    if !affected_story_ids.is_empty() {
                        // Remove affected stories from completed set so they can be retried
                        {
                            let mut state = self.execution_state.write().await;
                            for story_id in &affected_story_ids {
                                state.completed.remove(story_id);
                                state.failed.remove(story_id);
                            }
                        }

                        // Retry affected stories sequentially
                        for story_id in &affected_story_ids {
                            // Send SequentialRetryStarted event
                            if let Some(ref sender) = ui_sender {
                                let event = ParallelUIEvent::SequentialRetryStarted {
                                    story_id: story_id.clone(),
                                    reason: "Reconciliation issues detected".to_string(),
                                };
                                let _ = sender.try_send(event);
                            }

                            // Send StoryStarted event for sequential retry
                            let start_time = Instant::now();
                            run_metrics.start_step(story_id);
                            if let Some(ref sender) = ui_sender {
                                let story_info =
                                    story_info_map.get(story_id).cloned().unwrap_or_else(|| {
                                        StoryDisplayInfo::new(story_id, story_id, 0)
                                    });
                                let event = ParallelUIEvent::StoryStarted {
                                    story: story_info,
                                    iteration: 1,
                                    concurrent_count: 1,
                                };
                                let _ = sender.try_send(event);
                            }

                            let executor_config = ExecutorConfig {
                                prd_path: self.base_config.prd_path.clone(),
                                project_root: self.base_config.working_dir.clone(),
                                progress_path: self.base_config.working_dir.join("progress.txt"),
                                quality_profile: None,
                                agent_command: agent.to_string(),
                                max_iterations: self.base_config.max_iterations_per_story,
                                git_mutex: Some(self.git_mutex.clone()),
                                timeout_config: self.config.timeout_config.clone(),
                                ..Default::default()
                            };

                            let executor = StoryExecutor::new(executor_config);
                            let (_cancel_tx, cancel_rx) = watch::channel(false);

                            // Clone for iteration callback closure
                            let iter_story_id = story_id.clone();
                            let iter_ui_sender = ui_sender.clone();

                            let result = executor
                                .execute_story(story_id, cancel_rx, |iter, max| {
                                    if let Some(ref sender) = iter_ui_sender {
                                        let event = ParallelUIEvent::IterationUpdate {
                                            story_id: iter_story_id.clone(),
                                            iteration: iter,
                                            max_iterations: max,
                                            message: None,
                                        };
                                        let _ = sender.try_send(event);
                                    }
                                })
                                .await;

                            let duration = start_time.elapsed();
                            let duration_ms = duration.as_millis() as u64;

                            match result {
                                Ok(exec_result) if exec_result.success => {
                                    let mut state = self.execution_state.write().await;
                                    state.completed.insert(story_id.clone());
                                    *total_iterations += exec_result.iterations_used;
                                    // Record metrics and evidence
                                    let attempts = exec_result.iterations_used.max(1);
                                    run_metrics
                                        .complete_step(story_id, true, attempts, duration, None);
                                    emit_step_event(
                                        evidence,
                                        run_metrics,
                                        story_id,
                                        "completed",
                                        None,
                                        None,
                                    )
                                    .await;
                                    // Send StoryCompleted event
                                    if let Some(ref sender) = ui_sender {
                                        let event = ParallelUIEvent::StoryCompleted {
                                            story_id: story_id.clone(),
                                            iterations_used: exec_result.iterations_used,
                                            duration_ms,
                                        };
                                        let _ = sender.try_send(event);
                                    }
                                }
                                Ok(exec_result) => {
                                    let mut state = self.execution_state.write().await;
                                    let error_msg = exec_result
                                        .error
                                        .clone()
                                        .unwrap_or_else(|| "Unknown error".to_string());
                                    state.failed.insert(story_id.clone(), error_msg.clone());
                                    *total_iterations += exec_result.iterations_used;
                                    // Record metrics and evidence
                                    let attempts = exec_result.iterations_used.max(1);
                                    run_metrics.complete_step(
                                        story_id,
                                        false,
                                        attempts,
                                        duration,
                                        Some(error_msg.clone()),
                                    );
                                    emit_step_event(
                                        evidence,
                                        run_metrics,
                                        story_id,
                                        "failed",
                                        Some("quality_gates_failed".to_string()),
                                        Some(error_msg.clone()),
                                    )
                                    .await;
                                    // Send StoryFailed event
                                    if let Some(ref sender) = ui_sender {
                                        let event = ParallelUIEvent::StoryFailed {
                                            story_id: story_id.clone(),
                                            error: error_msg,
                                            iteration: exec_result.iterations_used,
                                        };
                                        let _ = sender.try_send(event);
                                    }
                                }
                                Err(e) => {
                                    let mut state = self.execution_state.write().await;
                                    state.failed.insert(story_id.clone(), e.to_string());
                                    *total_iterations += 1;
                                    // Record metrics and evidence
                                    let category = e.classify();
                                    run_metrics.complete_step(
                                        story_id,
                                        false,
                                        1,
                                        duration,
                                        Some(e.to_string()),
                                    );
                                    emit_step_event(
                                        evidence,
                                        run_metrics,
                                        story_id,
                                        "failed",
                                        Some(error_category_label(&category).to_string()),
                                        Some(e.to_string()),
                                    )
                                    .await;
                                    // Send StoryFailed event
                                    if let Some(ref sender) = ui_sender {
                                        let event = ParallelUIEvent::StoryFailed {
                                            story_id: story_id.clone(),
                                            error: e.to_string(),
                                            iteration: 1,
                                        };
                                        let _ = sender.try_send(event);
                                    }
                                }
                            }
                        }

                        // Run reconciliation again after sequential retry
                        let post_retry_result = engine.reconcile();
                        match post_retry_result {
                            ReconciliationResult::Clean => {
                                // Send ReconciliationStatus event for clean after retry
                                if let Some(ref sender) = ui_sender {
                                    let event = ParallelUIEvent::ReconciliationStatus {
                                        success: true,
                                        issues_count: 0,
                                        message: "Clean after sequential retry".to_string(),
                                    };
                                    let _ = sender.try_send(event);
                                }
                                None
                            }
                            ReconciliationResult::IssuesFound(remaining_issues) => {
                                // Send ReconciliationStatus event for remaining issues
                                if let Some(ref sender) = ui_sender {
                                    let event = ParallelUIEvent::ReconciliationStatus {
                                        success: false,
                                        issues_count: remaining_issues.len(),
                                        message: format!(
                                            "{} issues remain after sequential retry",
                                            remaining_issues.len()
                                        ),
                                    };
                                    let _ = sender.try_send(event);
                                }
                                Some(format!(
                                    "Reconciliation failed with {} issues after sequential retry",
                                    remaining_issues.len()
                                ))
                            }
                        }
                    } else {
                        // No affected stories identified, but issues exist
                        Some(format!(
                            "Reconciliation failed with {} issues",
                            issues.len()
                        ))
                    }
                } else {
                    // Fallback disabled, return error
                    Some(format!(
                        "Reconciliation failed with {} issues (fallback disabled)",
                        issues.len()
                    ))
                }
            }
        }
    }

    /// Identifies stories affected by reconciliation issues.
    ///
    /// Returns a list of story IDs that should be retried based on the issues found.
    fn get_affected_stories(
        &self,
        issues: &[ReconciliationIssue],
        batch_story_ids: &[String],
        graph: &DependencyGraph,
    ) -> Vec<String> {
        // Collect all affected files from issues
        let mut affected_files: HashSet<String> = HashSet::new();

        for issue in issues {
            match issue {
                ReconciliationIssue::GitConflict {
                    affected_files: files,
                } => {
                    affected_files.extend(files.iter().cloned());
                }
                ReconciliationIssue::TypeMismatch { file, .. } => {
                    if file != "unknown" {
                        affected_files.insert(file.clone());
                    }
                }
                ReconciliationIssue::ImportDuplicate => {
                    // For import duplicates, we can't easily determine which files are affected
                    // So we mark all batch stories as affected
                    return batch_story_ids.to_vec();
                }
            }
        }

        // If we couldn't identify specific files, retry all batch stories
        if affected_files.is_empty() {
            return batch_story_ids.to_vec();
        }

        // Find stories whose target_files overlap with affected files
        let mut affected_story_ids = Vec::new();

        for story_id in batch_story_ids {
            if let Some(story) = graph.get_story(story_id) {
                for target_file in &story.target_files {
                    // Check if any affected file matches or is contained in target_file pattern
                    for affected_file in &affected_files {
                        if target_file.contains(affected_file)
                            || affected_file.contains(target_file)
                        {
                            affected_story_ids.push(story_id.clone());
                            break;
                        }
                    }
                }
            }
        }

        // If we still couldn't match any stories, retry all
        if affected_story_ids.is_empty() {
            batch_story_ids.to_vec()
        } else {
            affected_story_ids
        }
    }

    /// Load the PRD file.
    fn load_prd(&self) -> Result<PrdFile, String> {
        validate_prd(&self.base_config.prd_path).map_err(|e| e.to_string())
    }

    /// Save a checkpoint with the current execution state.
    ///
    /// Does nothing if checkpointing is disabled.
    fn save_checkpoint(
        &self,
        story_id: &str,
        iteration: u32,
        max_iterations: u32,
        pause_reason: PauseReason,
    ) {
        if let Some(ref manager) = self.checkpoint_manager {
            let uncommitted_files = self.get_uncommitted_files().unwrap_or_default();
            let checkpoint = Checkpoint::new(
                Some(StoryCheckpoint::new(story_id, iteration, max_iterations)),
                pause_reason,
                uncommitted_files,
            );

            if let Err(e) = manager.save(&checkpoint) {
                eprintln!("Warning: Failed to save checkpoint: {}", e);
            }
        }
    }

    /// Get list of uncommitted files from git.
    fn get_uncommitted_files(&self) -> Result<Vec<String>, String> {
        use std::process::Command;

        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&self.base_config.working_dir)
            .output()
            .map_err(|e| format!("Failed to run git status: {}", e))?;

        if !output.status.success() {
            return Ok(vec![]);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let files: Vec<String> = stdout
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.len() > 3 {
                    Some(line[3..].to_string())
                } else {
                    None
                }
            })
            .filter(|path| path != ".ralph/checkpoint.json" && path != ".ralph/checkpoint.json.tmp")
            .collect();

        Ok(files)
    }
}

async fn emit_run_complete(
    evidence: &Option<Arc<Mutex<EvidenceWriter>>>,
    status: &str,
    error_type: Option<String>,
    error_message: Option<String>,
) {
    if let Some(writer) = evidence.as_ref() {
        let mut writer = writer.lock().await;
        writer.emit_run_complete(status, error_type, error_message);
    }
}

async fn emit_step_event(
    evidence: &Option<Arc<Mutex<EvidenceWriter>>>,
    run_metrics: &RunMetricsCollector,
    step_id: &str,
    status: &str,
    error_type: Option<String>,
    error_message: Option<String>,
) {
    if let Some(writer) = evidence.as_ref() {
        let mut writer = writer.lock().await;
        writer.emit_step(step_id, status, error_type, error_message);
        run_metrics.record_evidence_step(step_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // Circuit Breaker Configuration Tests
    // ============================================================================

    #[test]
    fn test_parallel_runner_config_default_circuit_breaker_threshold() {
        let config = ParallelRunnerConfig::default();
        // Default circuit breaker threshold should be 5
        assert_eq!(config.circuit_breaker_threshold, 5);
    }

    #[test]
    fn test_parallel_runner_config_custom_circuit_breaker_threshold() {
        let config = ParallelRunnerConfig {
            circuit_breaker_threshold: 3,
            ..Default::default()
        };
        assert_eq!(config.circuit_breaker_threshold, 3);
    }

    #[test]
    fn test_parallel_runner_config_circuit_breaker_threshold_zero() {
        // Zero threshold means circuit breaker triggers immediately on first failure
        let config = ParallelRunnerConfig {
            circuit_breaker_threshold: 0,
            ..Default::default()
        };
        assert_eq!(config.circuit_breaker_threshold, 0);
    }

    // ============================================================================
    // Cumulative Failure Counter Logic Tests
    // ============================================================================

    #[test]
    fn test_circuit_breaker_cumulative_failures_threshold() {
        // Simulate the circuit breaker counter logic as it works in run()
        let circuit_breaker_threshold: u32 = 5;
        let mut cumulative_failures: u32 = 0;

        // Simulate 4 cumulative failures across batches (should not trigger)
        for _ in 0..4 {
            cumulative_failures += 1;
            assert!(
                cumulative_failures < circuit_breaker_threshold,
                "Should not trigger circuit breaker yet"
            );
        }
        assert_eq!(cumulative_failures, 4);

        // 5th failure triggers circuit breaker
        cumulative_failures += 1;
        assert!(
            cumulative_failures >= circuit_breaker_threshold,
            "Should trigger circuit breaker at 5th failure"
        );
    }

    #[test]
    fn test_circuit_breaker_cumulative_failures_across_batches() {
        // Verify failures accumulate across multiple batches
        let threshold: u32 = 5;
        let mut cumulative: u32 = 0;

        // Batch 1: 2 failures
        cumulative += 2;
        assert!(cumulative < threshold);

        // Batch 2: 1 failure
        cumulative += 1;
        assert!(cumulative < threshold);

        // Batch 3: 2 failures - triggers
        cumulative += 2;
        assert!(cumulative >= threshold);
        assert_eq!(cumulative, 5);
    }

    #[test]
    fn test_circuit_breaker_does_not_reset_on_success() {
        // In parallel mode, cumulative failures do NOT reset on success
        // (unlike sequential mode which tracks consecutive failures)
        let threshold: u32 = 5;
        let mut cumulative: u32 = 0;

        // Batch 1: 2 failures
        cumulative += 2;

        // Batch 2: has some successes, but cumulative doesn't reset
        // Only add non-transient failures
        cumulative += 0; // successes don't decrement

        // Batch 3: 3 more failures - triggers
        cumulative += 3;
        assert!(cumulative >= threshold);
    }

    // ============================================================================
    // Non-Transient Failure Classification Tests
    // ============================================================================

    #[test]
    fn test_non_transient_failure_quality_gate() {
        // Quality gate failures (success=false, is_transient=false) count toward circuit breaker
        let success = false;
        let is_transient = false;
        let should_count = !success && !is_transient;
        assert!(should_count, "Quality gate failures should count");
    }

    #[test]
    fn test_transient_failure_does_not_count() {
        // Transient failures (network errors, etc) don't count toward circuit breaker
        let success = false;
        let is_transient = true;
        let should_count = !success && !is_transient;
        assert!(!should_count, "Transient failures should not count");
    }

    #[test]
    fn test_success_does_not_count() {
        // Successful executions don't count toward circuit breaker
        let success = true;
        let is_transient = false;
        let should_count = !success && !is_transient;
        assert!(!should_count, "Successes should not count");
    }

    #[test]
    fn test_batch_failure_counting_logic() {
        // Simulate batch result processing
        let results: Vec<(String, bool, u32, bool)> = vec![
            ("US-001".to_string(), true, 1, false),  // success
            ("US-002".to_string(), false, 3, false), // quality gate failure
            ("US-003".to_string(), false, 1, true),  // transient failure
            ("US-004".to_string(), false, 2, false), // quality gate failure
        ];

        let mut batch_non_transient_failures: u32 = 0;
        for (_story_id, success, _iterations, is_transient) in results {
            if !success && !is_transient {
                batch_non_transient_failures += 1;
            }
        }

        // Should count 2 non-transient failures (US-002 and US-004)
        assert_eq!(batch_non_transient_failures, 2);
    }

    // ============================================================================
    // Batch Timeout Failure Tests
    // ============================================================================

    #[test]
    fn test_batch_timeout_counts_as_non_transient() {
        // When a batch times out, all in-flight stories count as non-transient failures
        let threshold: u32 = 5;
        let mut cumulative: u32 = 0;

        // Batch 1: 2 regular failures
        cumulative += 2;

        // Batch 2: timeout with 3 in-flight stories
        let timed_out_count = 3;
        cumulative += timed_out_count;

        // Should trigger circuit breaker (5 >= 5)
        assert!(cumulative >= threshold);
    }

    // ============================================================================
    // Execution State Tests
    // ============================================================================

    #[test]
    fn test_execution_state_acquire_locks() {
        let mut state = ParallelExecutionState::default();

        // Acquire locks for story 1
        let acquired = state.acquire_locks("US-001", &["src/a.rs".to_string()]);
        assert!(acquired);

        // Can acquire same lock for same story
        let acquired = state.acquire_locks("US-001", &["src/a.rs".to_string()]);
        assert!(acquired);

        // Cannot acquire lock for different story
        let acquired = state.acquire_locks("US-002", &["src/a.rs".to_string()]);
        assert!(!acquired);

        // Can acquire different lock for different story
        let acquired = state.acquire_locks("US-002", &["src/b.rs".to_string()]);
        assert!(acquired);
    }

    #[test]
    fn test_execution_state_release_locks() {
        let mut state = ParallelExecutionState::default();

        // Acquire locks
        state.acquire_locks("US-001", &["src/a.rs".to_string(), "src/b.rs".to_string()]);
        assert_eq!(state.locked_files.len(), 2);

        // Release locks
        state.release_locks("US-001");
        assert!(state.locked_files.is_empty());
    }

    #[test]
    fn test_execution_state_track_in_flight() {
        let mut state = ParallelExecutionState::default();

        state.in_flight.insert("US-001".to_string());
        state.in_flight.insert("US-002".to_string());
        assert_eq!(state.in_flight.len(), 2);
        assert!(state.in_flight.contains("US-001"));

        state.in_flight.remove("US-001");
        assert!(!state.in_flight.contains("US-001"));
        assert_eq!(state.in_flight.len(), 1);
    }

    #[test]
    fn test_execution_state_track_failures() {
        let mut state = ParallelExecutionState::default();

        state
            .failed
            .insert("US-001".to_string(), "Quality gate failed".to_string());
        state
            .failed
            .insert("US-002".to_string(), "Timeout".to_string());

        assert_eq!(state.failed.len(), 2);
        assert_eq!(state.failed.get("US-001").unwrap(), "Quality gate failed");
    }

    // ============================================================================
    // Pre-execution Conflict Detection Tests
    // ============================================================================

    #[test]
    fn test_detect_preexecution_conflicts_no_overlap() {
        let stories = vec![
            StoryNode {
                id: "US-001".to_string(),
                priority: 1,
                passes: false,
                target_files: vec!["src/a.rs".to_string()],
                depends_on: vec![],
            },
            StoryNode {
                id: "US-002".to_string(),
                priority: 2,
                passes: false,
                target_files: vec!["src/b.rs".to_string()],
                depends_on: vec![],
            },
        ];

        let conflicts = detect_preexecution_conflicts(&stories);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn test_detect_preexecution_conflicts_with_overlap() {
        let stories = vec![
            StoryNode {
                id: "US-001".to_string(),
                priority: 1, // Higher priority (lower number)
                passes: false,
                target_files: vec!["src/shared.rs".to_string()],
                depends_on: vec![],
            },
            StoryNode {
                id: "US-002".to_string(),
                priority: 2, // Lower priority (higher number)
                passes: false,
                target_files: vec!["src/shared.rs".to_string()],
                depends_on: vec![],
            },
        ];

        let conflicts = detect_preexecution_conflicts(&stories);
        assert_eq!(conflicts.len(), 1);
        // US-002 (lower priority) should be deferred, US-001 runs first
        assert_eq!(conflicts[0], ("US-002".to_string(), "US-001".to_string()));
    }

    #[test]
    fn test_filter_conflicting_stories() {
        let stories = vec![
            StoryNode {
                id: "US-001".to_string(),
                priority: 1,
                passes: false,
                target_files: vec!["src/shared.rs".to_string()],
                depends_on: vec![],
            },
            StoryNode {
                id: "US-002".to_string(),
                priority: 2,
                passes: false,
                target_files: vec!["src/shared.rs".to_string()],
                depends_on: vec![],
            },
            StoryNode {
                id: "US-003".to_string(),
                priority: 3,
                passes: false,
                target_files: vec!["src/other.rs".to_string()],
                depends_on: vec![],
            },
        ];

        let (filtered, conflicts) = filter_conflicting_stories(stories);

        // US-002 should be filtered out (deferred), US-001 and US-003 remain
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|s| s.id == "US-001"));
        assert!(filtered.iter().any(|s| s.id == "US-003"));
        assert!(!filtered.iter().any(|s| s.id == "US-002"));

        // One conflict detected
        assert_eq!(conflicts.len(), 1);
    }
}
