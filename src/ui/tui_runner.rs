//! TUI runner display for enhanced terminal output.
//!
//! Uses ratatui widgets for rich progress and status display.

#![allow(dead_code)]

use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::sync::Mutex;

use crate::ui::display::{DisplayCallback, DisplayOptions, LastActivityInfo, SharedActivityState};
use crate::ui::keyboard::{render_compact_hint, KeyboardListener, ToggleState};
use crate::ui::tui::{
    AnimationState, CompletionSummaryWidget, GateChainWidget, GateInfo, GateStatus, GitSummary,
    IterationWidget, StoryHeaderWidget, StoryProgressWidget, StoryState,
};

/// Re-export LastActivityInfo as LastActivity for backward compatibility.
pub type LastActivity = LastActivityInfo;

/// A TUI-based display for the runner loop.
pub struct TuiRunnerDisplay {
    /// Animation state for spinners
    animation: AnimationState,
    /// Current story ID
    current_story_id: Option<String>,
    /// Current story title
    current_story_title: Option<String>,
    /// Current story priority
    current_story_priority: u32,
    /// All stories with their status
    stories: Vec<(String, StoryState)>,
    /// Current iteration
    current_iteration: u32,
    /// Max iterations
    max_iterations: u32,
    /// Current gates
    gates: Vec<GateInfo>,
    /// Execution start time
    start_time: Option<Instant>,
    /// Whether to use colors
    use_colors: bool,
    /// Terminal width
    term_width: usize,
    /// Whether quiet mode is enabled
    quiet: bool,
    /// Display options for enhanced UX features
    display_options: DisplayOptions,
    /// Live toggle state (shared with keyboard listener)
    toggle_state: Arc<ToggleState>,
    /// Last activity from the agent (shared with streaming callback)
    shared_activity: Option<SharedActivityState>,
    /// When the current iteration started
    iteration_start: Mutex<Option<Instant>>,
}

impl Default for TuiRunnerDisplay {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiRunnerDisplay {
    /// Create a new TUI runner display.
    pub fn new() -> Self {
        let term_width = terminal_width();
        let toggle_state = Arc::new(ToggleState::new(true, false)); // Streaming on by default
        Self {
            animation: AnimationState::new(30),
            current_story_id: None,
            current_story_title: None,
            current_story_priority: 1,
            stories: Vec::new(),
            current_iteration: 0,
            max_iterations: 10,
            gates: Vec::new(),
            start_time: None,
            use_colors: true,
            term_width,
            quiet: false,
            display_options: DisplayOptions::default(),
            toggle_state,
            shared_activity: None,
            iteration_start: Mutex::new(None),
        }
    }

    /// Create a TUI runner display with custom display options.
    pub fn with_display_options(options: DisplayOptions) -> Self {
        let term_width = terminal_width();
        let toggle_state = Arc::new(ToggleState::new(
            options.should_show_streaming(),
            options.should_expand_details(),
        ));
        Self {
            animation: AnimationState::new(30),
            current_story_id: None,
            current_story_title: None,
            current_story_priority: 1,
            stories: Vec::new(),
            current_iteration: 0,
            max_iterations: 10,
            gates: Vec::new(),
            start_time: None,
            use_colors: options.should_enable_colors(),
            term_width,
            quiet: options.quiet,
            display_options: options,
            toggle_state,
            shared_activity: None,
            iteration_start: Mutex::new(None),
        }
    }

    /// Set the shared activity state for displaying last activity.
    ///
    /// Call this after creating the display and the streaming callback,
    /// passing the callback's shared activity state.
    pub fn with_shared_activity(mut self, activity: SharedActivityState) -> Self {
        self.shared_activity = Some(activity);
        self
    }

    /// Set the shared activity state (mutable reference version).
    pub fn set_shared_activity(&mut self, activity: SharedActivityState) {
        self.shared_activity = Some(activity);
    }

    /// Get the last activity, if any.
    pub fn get_last_activity(&self) -> Option<LastActivity> {
        self.shared_activity.as_ref()?.lock().ok()?.clone()
    }

    /// Get elapsed time since iteration started.
    pub fn get_iteration_elapsed(&self) -> Option<Duration> {
        self.iteration_start
            .lock()
            .ok()?
            .map(|start| start.elapsed())
    }

    /// Redraw the iteration status line after streaming output.
    fn redraw_iteration_status(&self) {
        let iter_widget = IterationWidget::new(self.current_iteration, self.max_iterations)
            .with_gates(self.gates.clone())
            .with_animation(self.animation.clone());
        print!("  {}", iter_widget.render_string());
        io::stdout().flush().ok();
    }

    /// Set quiet mode.
    pub fn with_quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self.display_options.quiet = quiet;
        self
    }

    /// Set whether to use colors.
    pub fn with_colors(mut self, use_colors: bool) -> Self {
        self.use_colors = use_colors;
        self
    }

    /// Get the toggle state for sharing with a keyboard listener.
    pub fn toggle_state(&self) -> Arc<ToggleState> {
        Arc::clone(&self.toggle_state)
    }

    /// Check if streaming output should be shown (respects live toggle).
    pub fn should_show_streaming(&self) -> bool {
        self.toggle_state.should_show_streaming()
    }

    /// Check if details should be expanded (respects live toggle).
    pub fn should_expand_details(&self) -> bool {
        self.toggle_state.should_expand_details()
    }

    /// Get the verbosity level.
    pub fn verbosity(&self) -> u8 {
        self.display_options.verbosity
    }

    /// Render the toggle hint bar.
    pub fn render_toggle_hint(&self) -> String {
        let streaming = if self.should_show_streaming() {
            "on"
        } else {
            "off"
        };
        let expand = if self.should_expand_details() {
            "on"
        } else {
            "off"
        };
        self.style_dim(&format!(
            " [s] stream: {} | [e] expand: {} | [p] pause | [q] quit",
            streaming, expand
        ))
        .to_string()
    }

    /// Initialize stories from PRD data.
    pub fn init_stories(&mut self, stories: Vec<(String, bool)>) {
        self.stories = stories
            .into_iter()
            .map(|(id, passes)| {
                let state = if passes {
                    StoryState::Passed
                } else {
                    StoryState::Pending
                };
                (id, state)
            })
            .collect();
    }

    /// Display startup banner.
    pub fn display_startup(&self, prd_path: &str, agent: &str, passing: usize, total: usize) {
        if self.quiet {
            return;
        }

        println!();
        println!(
            "{}",
            self.style_header("╔════════════════════════════════════════════════════════════╗")
        );
        println!(
            "{}",
            self.style_header("║               🥋 RALPH ITERATION LOOP                      ║")
        );
        println!(
            "{}",
            self.style_header("╚════════════════════════════════════════════════════════════╝")
        );
        println!();
        println!("  {} {}", self.style_dim("PRD:"), prd_path);
        println!("  {} {}", self.style_dim("Agent:"), agent);
        println!(
            "  {} {}/{} passing",
            self.style_dim("Stories:"),
            passing,
            total
        );
        println!();
    }

    /// Display story started.
    pub fn start_story(&mut self, story_id: &str, title: &str, priority: u32) {
        if self.quiet {
            return;
        }

        self.current_story_id = Some(story_id.to_string());
        self.current_story_title = Some(title.to_string());
        self.current_story_priority = priority;
        self.current_iteration = 0;
        self.start_time = Some(Instant::now());
        self.gates.clear();

        // Mark story as running
        for (id, state) in &mut self.stories {
            if id == story_id {
                *state = StoryState::Running;
            }
        }

        // Render story header
        let header = StoryHeaderWidget::new(story_id, title, priority);
        println!();
        println!("{}", header.render_string(self.term_width));

        // Render progress
        self.render_progress();
    }

    /// Update iteration progress.
    pub fn update_iteration(&mut self, iteration: u32, max: u32) {
        if self.quiet {
            return;
        }

        self.current_iteration = iteration;
        self.max_iterations = max;
        self.animation.tick();

        // Clear previous lines (iteration line and optional activity line)
        // Move up one line if we might have an activity line, then clear both
        print!("\x1b[1A\r\x1b[K\x1b[1B\r\x1b[K\x1b[1A"); // Up, clear, down, clear, up

        // Render iteration widget
        let iter_widget = IterationWidget::new(iteration, max)
            .with_gates(self.gates.clone())
            .with_animation(self.animation.clone());

        // Build the elapsed time string
        let elapsed_str = if let Some(elapsed) = self.get_iteration_elapsed() {
            let secs = elapsed.as_secs();
            if secs >= 60 {
                format!(" {}m {}s", secs / 60, secs % 60)
            } else {
                format!(" {}s", secs)
            }
        } else {
            String::new()
        };

        print!(
            "  {}{}",
            iter_widget.render_string(),
            self.style_dim(&elapsed_str)
        );

        // Render last activity indicator on a new line if available
        if let Some(activity) = self.get_last_activity() {
            let time_ago = activity.timestamp.elapsed();
            let time_str = if time_ago.as_secs() >= 60 {
                format!(
                    "{}m {}s ago",
                    time_ago.as_secs() / 60,
                    time_ago.as_secs() % 60
                )
            } else {
                format!("{}s ago", time_ago.as_secs())
            };

            // Truncate activity description to fit terminal width
            let max_desc_len = self.term_width.saturating_sub(20); // Leave room for prefix and time
            let desc = if activity.description.len() > max_desc_len {
                format!(
                    "{}...",
                    &activity.description[..max_desc_len.saturating_sub(3)]
                )
            } else {
                activity.description.clone()
            };

            println!();
            print!(
                "  {}",
                self.style_dim(&format!("└── {} ({})", desc, time_str))
            );
        } else {
            // Print empty activity line placeholder
            println!();
            print!("  {}", self.style_dim("└── Waiting for agent output..."));
        }

        io::stdout().flush().ok();
    }

    /// Update gate status.
    pub fn update_gate(&mut self, name: &str, passed: bool) {
        let status = if passed {
            GateStatus::Passed
        } else {
            GateStatus::Failed
        };

        // Update existing gate or add new
        let mut found = false;
        for gate in &mut self.gates {
            if gate.name == name {
                gate.status = status;
                found = true;
                break;
            }
        }

        if !found {
            self.gates.push(GateInfo::new(name, status));
        }

        // Re-render iteration with updated gates
        if !self.quiet {
            print!("\r\x1b[K");
            let iter_widget = IterationWidget::new(self.current_iteration, self.max_iterations)
                .with_gates(self.gates.clone())
                .with_animation(self.animation.clone());
            print!("  {}", iter_widget.render_string());
            io::stdout().flush().ok();
        }
    }

    /// Start a gate (mark as running).
    pub fn start_gate(&mut self, name: &str) {
        // Check if gate exists
        let mut found = false;
        for gate in &mut self.gates {
            if gate.name == name {
                gate.status = GateStatus::Running;
                found = true;
                break;
            }
        }

        if !found {
            self.gates.push(GateInfo::new(name, GateStatus::Running));
        }
    }

    /// Display story completed.
    pub fn complete_story(&mut self, story_id: &str, commit_hash: Option<&str>) {
        // Mark story as passed
        for (id, state) in &mut self.stories {
            if id == story_id {
                *state = StoryState::Passed;
            }
        }

        if self.quiet {
            return;
        }

        println!(); // New line after iteration display

        let duration = self
            .start_time
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);

        // Build git summary
        let git = if let Some(hash) = commit_hash {
            GitSummary::new().with_commit(hash)
        } else {
            GitSummary::new()
        };

        // Display completion summary
        let summary = CompletionSummaryWidget::new(
            story_id,
            true,
            duration,
            self.current_iteration,
            self.max_iterations,
        )
        .with_gates(self.gates.clone())
        .with_git(git);

        println!();
        println!("{}", summary.render_string(self.term_width));
    }

    /// Display story failed.
    pub fn fail_story(&mut self, story_id: &str, error: &str) {
        // Mark story as failed
        for (id, state) in &mut self.stories {
            if id == story_id {
                *state = StoryState::Failed;
            }
        }

        if self.quiet {
            return;
        }

        println!(); // New line after iteration display

        let duration = self
            .start_time
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);

        // Display failure summary
        let summary = CompletionSummaryWidget::new(
            story_id,
            false,
            duration,
            self.current_iteration,
            self.max_iterations,
        )
        .with_gates(self.gates.clone());

        println!();
        println!("{}", summary.render_string(self.term_width));
        println!("  {} {}", self.style_error("Error:"), error);
        println!("  Continuing to next story...");
    }

    /// Display all stories complete.
    pub fn display_all_complete(&self, total: usize) {
        if self.quiet {
            return;
        }

        let message = format!("🎉 ALL {} STORIES COMPLETE! 🎉", total);
        let box_width = message.len() + 4;

        println!();
        println!(
            "{}",
            self.style_success(&format!("╔{}╗", "═".repeat(box_width - 2)))
        );
        println!("{}", self.style_success(&format!("║  {}  ║", message)));
        println!(
            "{}",
            self.style_success(&format!("╚{}╝", "═".repeat(box_width - 2)))
        );
        println!();
        println!("<promise>COMPLETE</promise>");
    }

    /// Render progress bar.
    fn render_progress(&self) {
        let widget =
            StoryProgressWidget::new(self.stories.clone()).with_animation(self.animation.clone());
        println!("{}", widget.render_string());
    }

    // Style helpers
    fn style_header(&self, text: &str) -> String {
        if self.use_colors {
            format!("\x1b[38;2;34;211;238m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    fn style_dim(&self, text: &str) -> String {
        if self.use_colors {
            format!("\x1b[38;2;107;114;128m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    fn style_success(&self, text: &str) -> String {
        if self.use_colors {
            format!("\x1b[38;2;34;197;94m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    fn style_error(&self, text: &str) -> String {
        if self.use_colors {
            format!("\x1b[38;2;239;68;68m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }
}

/// Get terminal width, defaulting to 80.
fn terminal_width() -> usize {
    // Try to get terminal size
    if let Ok((cols, _)) = crossterm::terminal::size() {
        cols as usize
    } else {
        80
    }
}

/// Parse agent output line to extract a human-readable activity description.
///
/// Returns an icon and description if a recognizable pattern is found.
fn parse_activity_from_line(line: &str) -> Option<(char, String)> {
    let line_lower = line.to_lowercase();

    // File reading patterns
    if line_lower.contains("reading") || line_lower.contains("read ") {
        // Try to extract filename
        if let Some(path) = extract_path_from_line(line) {
            return Some(('\u{1F4D6}', format!("Reading {}", path))); // 📖
        }
        return Some(('\u{1F4D6}', "Reading file...".to_string()));
    }

    // File writing patterns
    if line_lower.contains("writing")
        || line_lower.contains("write ")
        || line_lower.contains("edit ")
        || line_lower.contains("created ")
        || line_lower.contains("updated ")
    {
        if let Some(path) = extract_path_from_line(line) {
            return Some(('\u{270F}', format!("Writing {}", path))); // ✏️
        }
        return Some(('\u{270F}', "Writing file...".to_string()));
    }

    // Command execution patterns
    if line_lower.contains("running")
        || line_lower.contains("executing")
        || line_lower.starts_with("$ ")
        || line_lower.contains("cargo ")
        || line_lower.contains("npm ")
        || line_lower.contains("git ")
    {
        let cmd = line.trim_start_matches("$ ").trim();
        let short_cmd = if cmd.len() > 40 {
            format!("{}...", &cmd[..37])
        } else {
            cmd.to_string()
        };
        return Some(('\u{1F527}', format!("Running: {}", short_cmd))); // 🔧
    }

    // Thinking/analysis patterns
    if line_lower.contains("thinking")
        || line_lower.contains("analyzing")
        || line_lower.contains("planning")
        || line_lower.contains("considering")
    {
        return Some(('\u{1F9E0}', "Analyzing...".to_string())); // 🧠
    }

    None
}

/// Extract a file path from a line of output.
fn extract_path_from_line(line: &str) -> Option<String> {
    // Common patterns: "Reading file: path", "Read path", etc.
    let patterns = ["file:", "file ", ": "];
    for pattern in &patterns {
        if let Some(idx) = line.to_lowercase().find(pattern) {
            let after = &line[idx + pattern.len()..];
            let path = after.split_whitespace().next()?;
            // Clean up path (remove quotes, trailing punctuation)
            let path = path.trim_matches(|c| c == '"' || c == '\'' || c == ',' || c == '.');
            if !path.is_empty() && (path.contains('/') || path.contains('.')) {
                // Shorten long paths
                let display_path = if path.len() > 50 {
                    format!("...{}", &path[path.len() - 47..])
                } else {
                    path.to_string()
                };
                return Some(display_path);
            }
        }
    }
    None
}

impl DisplayCallback for TuiRunnerDisplay {
    fn on_agent_output(&self, line: &str, is_stderr: bool) {
        // Skip if quiet mode
        if self.quiet {
            return;
        }

        // Update shared activity if available
        if let Some(ref shared) = self.shared_activity {
            if let Some((icon, description)) = parse_activity_from_line(line) {
                if let Ok(mut activity) = shared.lock() {
                    *activity = Some(LastActivity {
                        timestamp: Instant::now(),
                        description: format!("{} {}", icon, description),
                        is_stderr,
                    });
                }
            } else {
                // Still update timestamp even without a parsed description
                if let Ok(mut activity) = shared.lock() {
                    let desc = if line.len() > 60 {
                        format!("{}...", &line[..57])
                    } else {
                        line.to_string()
                    };
                    *activity = Some(LastActivity {
                        timestamp: Instant::now(),
                        description: desc,
                        is_stderr,
                    });
                }
            }
        }

        // Stream output to terminal if streaming is enabled
        if self.should_show_streaming() {
            // Clear current line and print output with prefix
            print!("\r\x1b[K"); // Clear line
                                // Use dim prefix for both stdout/stderr; could differentiate later
            let _ = is_stderr; // Reserved for future stderr styling
            let prefix = self.style_dim("  │ ");
            println!("{}{}", prefix, line);
            // Redraw iteration status
            self.redraw_iteration_status();
        }
    }

    fn on_agent_started(&self, story_id: &str, iteration: u32) {
        // Reset iteration timer
        if let Ok(mut start) = self.iteration_start.lock() {
            *start = Some(Instant::now());
        }
        // Clear shared activity if available
        if let Some(ref shared) = self.shared_activity {
            if let Ok(mut activity) = shared.lock() {
                *activity = None;
            }
        }

        if !self.quiet && self.display_options.verbosity >= 1 {
            eprintln!(
                "{}",
                self.style_dim(&format!(
                    "  [Agent started: {} iteration {}]",
                    story_id, iteration
                ))
            );
        }
    }

    fn on_agent_completed(&self, story_id: &str, success: bool) {
        if !self.quiet && self.display_options.verbosity >= 1 {
            let status = if success { "completed" } else { "failed" };
            eprintln!(
                "{}",
                self.style_dim(&format!("  [Agent {}: {}]", status, story_id))
            );
        }
    }
}

// Helper methods for DisplayCallback impl are defined in the main impl block above
