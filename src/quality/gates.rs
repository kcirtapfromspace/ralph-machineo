//! Quality gate checking functionality for Ralph.
//!
//! This module provides the infrastructure for running quality gates
//! against a codebase, including coverage, linting, formatting, and security checks.

// Allow dead_code for now - these types will be used in future stories
#![allow(dead_code)]

use crate::quality::Profile;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// JSON message format from `cargo clippy --message-format=json`.
///
/// Each line of clippy's stdout is a separate JSON object with this structure.
#[derive(Debug, Deserialize)]
struct ClippyMessage {
    /// Type of message (e.g., "compiler-message", "compiler-artifact")
    reason: String,
    /// The actual diagnostic message (only present for compiler-message)
    message: Option<ClippyDiagnostic>,
}

/// A diagnostic message from clippy.
#[derive(Debug, Deserialize)]
struct ClippyDiagnostic {
    /// Severity level (e.g., "error", "warning", "note", "help")
    level: String,
    /// Human-readable message text
    message: String,
    /// Error code information (e.g., for "E0382" or "clippy::unwrap_used")
    code: Option<ClippyCode>,
    /// Source locations where the diagnostic applies
    #[serde(default)]
    spans: Vec<ClippySpan>,
    /// Child diagnostics (notes, help messages, suggestions)
    #[serde(default)]
    children: Vec<ClippyDiagnostic>,
}

/// Error code information from clippy.
#[derive(Debug, Deserialize)]
struct ClippyCode {
    /// The error code string (e.g., "E0382", "clippy::unwrap_used")
    code: String,
    /// Optional explanation text
    explanation: Option<String>,
}

/// Source location span from clippy.
#[derive(Debug, Deserialize)]
struct ClippySpan {
    /// File path
    file_name: String,
    /// Starting line number (1-indexed)
    line_start: u32,
    /// Starting column number (1-indexed)
    column_start: u32,
    /// Suggested replacement text (for fixable warnings)
    suggested_replacement: Option<String>,
}

/// Category of a quality gate failure.
///
/// Represents the type of check that failed, allowing downstream code
/// to programmatically handle different failure types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
    /// Code linting failure (e.g., clippy warnings)
    Lint,
    /// Type checking failure (e.g., compilation errors)
    TypeCheck,
    /// Test failure (e.g., unit or integration tests)
    Test,
    /// Code formatting failure (e.g., rustfmt issues)
    Format,
    /// Security vulnerability detected (e.g., cargo-audit findings)
    Security,
    /// Code coverage below threshold
    Coverage,
}

/// Structured details about a quality gate failure.
///
/// Provides programmatic access to failure information, including
/// source location, error codes, and remediation suggestions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateFailureDetail {
    /// File path where the failure occurred (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,

    /// Line number where the failure occurred (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,

    /// Column number where the failure occurred (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,

    /// Error code or identifier (e.g., "E0382", "clippy::unwrap_used")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,

    /// Category of the failure
    pub category: FailureCategory,

    /// Human-readable message describing the failure
    pub message: String,

    /// Suggested fix or remediation action
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,

    /// URL to documentation about this type of failure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_url: Option<String>,
}

impl GateFailureDetail {
    /// Create a new gate failure detail with required fields.
    ///
    /// # Arguments
    ///
    /// * `category` - The category of failure
    /// * `message` - Human-readable description of the failure
    pub fn new(category: FailureCategory, message: impl Into<String>) -> Self {
        Self {
            file: None,
            line: None,
            column: None,
            error_code: None,
            category,
            message: message.into(),
            suggestion: None,
            doc_url: None,
        }
    }

    /// Set the file path where the failure occurred.
    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    /// Set the line number where the failure occurred.
    pub fn with_line(mut self, line: u32) -> Self {
        self.line = Some(line);
        self
    }

    /// Set the column number where the failure occurred.
    pub fn with_column(mut self, column: u32) -> Self {
        self.column = Some(column);
        self
    }

    /// Set the source location (file, line, column) where the failure occurred.
    pub fn with_location(
        mut self,
        file: impl Into<String>,
        line: u32,
        column: Option<u32>,
    ) -> Self {
        self.file = Some(file.into());
        self.line = Some(line);
        self.column = column;
        self
    }

    /// Set the error code or identifier.
    pub fn with_error_code(mut self, error_code: impl Into<String>) -> Self {
        self.error_code = Some(error_code.into());
        self
    }

    /// Set the suggested fix or remediation action.
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// Set the documentation URL.
    pub fn with_doc_url(mut self, doc_url: impl Into<String>) -> Self {
        self.doc_url = Some(doc_url.into());
        self
    }
}

/// Progress state for a quality gate.
///
/// Used in progress callbacks to report gate status changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateProgressState {
    /// Gate is currently running
    Running,
    /// Gate completed successfully
    Passed,
    /// Gate failed
    Failed,
}

/// Progress update for a quality gate.
///
/// Contains information about a gate's current state and duration.
#[derive(Debug, Clone)]
pub struct GateProgressUpdate {
    /// Name of the quality gate
    pub gate_name: String,
    /// Current progress state
    pub state: GateProgressState,
    /// Duration of the gate execution (only set for Passed/Failed states)
    pub duration: Option<Duration>,
}

impl GateProgressUpdate {
    /// Create a new Running progress update.
    pub fn running(gate_name: impl Into<String>) -> Self {
        Self {
            gate_name: gate_name.into(),
            state: GateProgressState::Running,
            duration: None,
        }
    }

    /// Create a new Passed progress update with duration.
    pub fn passed(gate_name: impl Into<String>, duration: Duration) -> Self {
        Self {
            gate_name: gate_name.into(),
            state: GateProgressState::Passed,
            duration: Some(duration),
        }
    }

    /// Create a new Failed progress update with duration.
    pub fn failed(gate_name: impl Into<String>, duration: Duration) -> Self {
        Self {
            gate_name: gate_name.into(),
            state: GateProgressState::Failed,
            duration: Some(duration),
        }
    }

    /// Check if this is a Running state.
    pub fn is_running(&self) -> bool {
        self.state == GateProgressState::Running
    }

    /// Check if this is a Passed state.
    pub fn is_passed(&self) -> bool {
        self.state == GateProgressState::Passed
    }

    /// Check if this is a Failed state.
    pub fn is_failed(&self) -> bool {
        self.state == GateProgressState::Failed
    }

    /// Check if the gate has completed (Passed or Failed).
    pub fn is_completed(&self) -> bool {
        matches!(
            self.state,
            GateProgressState::Passed | GateProgressState::Failed
        )
    }

    /// Format the duration for display, if available.
    pub fn format_duration(&self) -> Option<String> {
        self.duration.map(|d| {
            if d.as_secs() >= 60 {
                format!(
                    "{}m{:.1}s",
                    d.as_secs() / 60,
                    (d.as_millis() % 60000) as f64 / 1000.0
                )
            } else {
                format!("{:.1}s", d.as_secs_f64())
            }
        })
    }
}

/// The result of running a single quality gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateResult {
    /// Name of the quality gate that was run
    pub gate_name: String,
    /// Whether the gate passed
    pub passed: bool,
    /// Human-readable message describing the result
    pub message: String,
    /// Additional details about the gate result (e.g., specific errors, metrics)
    pub details: Option<String>,
    /// Structured failure details for programmatic access
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<GateFailureDetail>,
}

impl GateResult {
    /// Create a new passing gate result.
    pub fn pass(gate_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            gate_name: gate_name.into(),
            passed: true,
            message: message.into(),
            details: None,
            failures: Vec::new(),
        }
    }

    /// Create a new failing gate result.
    ///
    /// # Arguments
    ///
    /// * `gate_name` - Name of the quality gate
    /// * `message` - Human-readable message describing the failure
    /// * `details` - Optional additional details (retained for backward compatibility)
    /// * `failures` - Optional structured failure details for programmatic access
    pub fn fail(
        gate_name: impl Into<String>,
        message: impl Into<String>,
        details: Option<String>,
        failures: Option<Vec<GateFailureDetail>>,
    ) -> Self {
        Self {
            gate_name: gate_name.into(),
            passed: false,
            message: message.into(),
            details,
            failures: failures.unwrap_or_default(),
        }
    }

    /// Create a new skipped gate result.
    pub fn skipped(gate_name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            gate_name: gate_name.into(),
            passed: true, // Skipped gates count as passed
            message: format!("Skipped: {}", reason.into()),
            details: None,
            failures: Vec::new(),
        }
    }
}

/// A checker that runs quality gates based on a profile configuration.
pub struct QualityGateChecker {
    /// The quality profile to check against
    profile: Profile,
    /// The root directory of the project to check
    project_root: PathBuf,
}

impl QualityGateChecker {
    /// Create a new quality gate checker.
    ///
    /// # Arguments
    ///
    /// * `profile` - The quality profile containing gate configurations
    /// * `project_root` - The root directory of the project to check
    pub fn new(profile: Profile, project_root: impl Into<PathBuf>) -> Self {
        Self {
            profile,
            project_root: project_root.into(),
        }
    }

    /// Get the profile being used for quality checks.
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// Get the project root directory.
    pub fn project_root(&self) -> &PathBuf {
        &self.project_root
    }

    /// Check code coverage against the profile threshold.
    ///
    /// This method runs either `cargo llvm-cov` or `cargo tarpaulin` to measure
    /// code coverage and compares it against the threshold configured in the profile.
    ///
    /// # Returns
    ///
    /// A `GateResult` indicating whether the coverage threshold was met.
    /// If coverage tools are not installed, returns a failure with installation instructions.
    pub fn check_coverage(&self) -> GateResult {
        let threshold = self.profile.testing.coverage_threshold;

        // If threshold is 0, skip coverage check
        if threshold == 0 {
            return GateResult::skipped("coverage", "Coverage threshold is 0 - no check required");
        }

        // Try cargo-llvm-cov first (more common in CI environments)
        let llvm_cov_result = self.run_llvm_cov();
        if let Some(result) = llvm_cov_result {
            return result;
        }

        // Fall back to cargo-tarpaulin
        let tarpaulin_result = self.run_tarpaulin();
        if let Some(result) = tarpaulin_result {
            return result;
        }

        // Neither tool is available
        GateResult::fail(
            "coverage",
            "No coverage tool available",
            Some(
                "Install cargo-llvm-cov: cargo install cargo-llvm-cov\n\
                 Or install cargo-tarpaulin: cargo install cargo-tarpaulin"
                    .to_string(),
            ),
            None,
        )
    }

    /// Run cargo-llvm-cov and parse the coverage percentage.
    fn run_llvm_cov(&self) -> Option<GateResult> {
        // Check if cargo-llvm-cov is installed
        let check_installed = Command::new("cargo")
            .args(["llvm-cov", "--version"])
            .current_dir(&self.project_root)
            .output();

        if check_installed.is_err() || !check_installed.unwrap().status.success() {
            return None; // Tool not installed
        }

        // Run cargo llvm-cov with JSON output for parsing
        let output = Command::new("cargo")
            .args(["llvm-cov", "--json", "--quiet"])
            .current_dir(&self.project_root)
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                if !output.status.success() {
                    return Some(GateResult::fail(
                        "coverage",
                        "cargo llvm-cov failed",
                        Some(format!("stderr: {}", stderr)),
                        None,
                    ));
                }

                // Parse the JSON output for coverage percentage
                if let Some(coverage) = Self::parse_llvm_cov_json(&stdout) {
                    Some(self.evaluate_coverage(coverage, "cargo-llvm-cov"))
                } else {
                    // If JSON parsing fails, try running with summary output
                    self.run_llvm_cov_summary()
                }
            }
            Err(e) => Some(GateResult::fail(
                "coverage",
                "Failed to run cargo llvm-cov",
                Some(e.to_string()),
                None,
            )),
        }
    }

    /// Run cargo-llvm-cov with summary output and parse the percentage.
    fn run_llvm_cov_summary(&self) -> Option<GateResult> {
        let output = Command::new("cargo")
            .args(["llvm-cov", "--quiet"])
            .current_dir(&self.project_root)
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                if !output.status.success() {
                    return Some(GateResult::fail(
                        "coverage",
                        "cargo llvm-cov failed",
                        Some(format!("stderr: {}", stderr)),
                        None,
                    ));
                }

                // Parse the summary output for coverage percentage
                // llvm-cov outputs lines like "TOTAL ... 75.00%"
                if let Some(coverage) = Self::parse_coverage_percentage(&stdout) {
                    Some(self.evaluate_coverage(coverage, "cargo-llvm-cov"))
                } else {
                    Some(GateResult::fail(
                        "coverage",
                        "Failed to parse llvm-cov output",
                        Some(format!("Output: {}", stdout)),
                        None,
                    ))
                }
            }
            Err(e) => Some(GateResult::fail(
                "coverage",
                "Failed to run cargo llvm-cov",
                Some(e.to_string()),
                None,
            )),
        }
    }

    /// Parse llvm-cov JSON output for total coverage percentage.
    fn parse_llvm_cov_json(json_str: &str) -> Option<f64> {
        // llvm-cov JSON has a "data" array with coverage info
        // We need to extract the total line coverage percentage
        // Format: { "data": [{ "totals": { "lines": { "percent": 75.5 } } }] }
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
            json.get("data")
                .and_then(|d| d.get(0))
                .and_then(|d| d.get("totals"))
                .and_then(|t| t.get("lines"))
                .and_then(|l| l.get("percent"))
                .and_then(|p| p.as_f64())
        } else {
            None
        }
    }

    /// Run cargo-tarpaulin and parse the coverage percentage.
    fn run_tarpaulin(&self) -> Option<GateResult> {
        // Check if cargo-tarpaulin is installed
        let check_installed = Command::new("cargo")
            .args(["tarpaulin", "--version"])
            .current_dir(&self.project_root)
            .output();

        if check_installed.is_err() || !check_installed.unwrap().status.success() {
            return None; // Tool not installed
        }

        // Run cargo tarpaulin
        let output = Command::new("cargo")
            .args(["tarpaulin", "--skip-clean", "--out", "Stdout"])
            .current_dir(&self.project_root)
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                // tarpaulin returns exit code 0 even on low coverage
                // Parse the output for coverage percentage
                // Format: "XX.XX% coverage"
                if let Some(coverage) = Self::parse_coverage_percentage(&stdout) {
                    Some(self.evaluate_coverage(coverage, "cargo-tarpaulin"))
                } else if let Some(coverage) = Self::parse_coverage_percentage(&stderr) {
                    // Sometimes tarpaulin outputs to stderr
                    Some(self.evaluate_coverage(coverage, "cargo-tarpaulin"))
                } else {
                    Some(GateResult::fail(
                        "coverage",
                        "Failed to parse tarpaulin output",
                        Some(format!("stdout: {}\nstderr: {}", stdout, stderr)),
                        None,
                    ))
                }
            }
            Err(e) => Some(GateResult::fail(
                "coverage",
                "Failed to run cargo tarpaulin",
                Some(e.to_string()),
                None,
            )),
        }
    }

    /// Parse coverage percentage from text output.
    /// Looks for patterns like "75.00%" or "75.00% coverage" or "TOTAL ... 75.00%"
    fn parse_coverage_percentage(output: &str) -> Option<f64> {
        // Look for percentage patterns
        let re_patterns = [
            // Match "XX.XX% coverage" (tarpaulin format)
            r"(\d+(?:\.\d+)?)\s*%\s*coverage",
            // Match "TOTAL ... XX.XX%" (llvm-cov format)
            r"TOTAL\s+.*?(\d+(?:\.\d+)?)\s*%",
            // Match standalone percentage at end of line
            r"(\d+(?:\.\d+)?)\s*%\s*$",
        ];

        for pattern in &re_patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                if let Some(captures) = re.captures(output) {
                    if let Some(match_) = captures.get(1) {
                        if let Ok(coverage) = match_.as_str().parse::<f64>() {
                            return Some(coverage);
                        }
                    }
                }
            }
        }

        None
    }

    /// Evaluate coverage against the threshold and return a GateResult.
    fn evaluate_coverage(&self, coverage: f64, tool_name: &str) -> GateResult {
        let threshold = self.profile.testing.coverage_threshold as f64;
        let coverage_str = format!("{:.2}%", coverage);

        if coverage >= threshold {
            GateResult::pass(
                "coverage",
                format!(
                    "Coverage {coverage_str} meets threshold of {threshold:.0}% (via {tool_name})"
                ),
            )
        } else {
            GateResult::fail(
                "coverage",
                format!("Coverage {coverage_str} is below threshold of {threshold:.0}%"),
                Some(format!(
                    "Measured with {tool_name}. Increase test coverage to meet the threshold."
                )),
                None,
            )
        }
    }

    /// Check code linting using cargo clippy.
    ///
    /// Runs `cargo clippy --message-format=json -- -D warnings` which treats all warnings as errors
    /// and outputs structured JSON for parsing.
    ///
    /// # Returns
    ///
    /// A `GateResult` indicating whether clippy passed without warnings.
    pub fn check_lint(&self) -> GateResult {
        if !self.profile.ci.lint_check {
            return GateResult::skipped("lint", "Lint checking not enabled in profile");
        }

        let output = Command::new("cargo")
            .args(["clippy", "--message-format=json", "--", "-D", "warnings"])
            .current_dir(&self.project_root)
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                if output.status.success() {
                    GateResult::pass("lint", "No clippy warnings found")
                } else {
                    // Extract structured failure details from JSON output
                    let failures = Self::extract_clippy_errors(&stdout, &stderr);
                    let details = Self::format_clippy_summary(&failures);
                    GateResult::fail(
                        "lint",
                        "Clippy found warnings or errors",
                        Some(details),
                        Some(failures),
                    )
                }
            }
            Err(e) => GateResult::fail(
                "lint",
                "Failed to run cargo clippy",
                Some(format!("Error: {}. Is clippy installed?", e)),
                None,
            ),
        }
    }

    /// Maximum number of clippy failures to include in results.
    const MAX_CLIPPY_FAILURES: usize = 20;

    /// Extract structured error details from clippy output.
    ///
    /// Attempts to parse JSON output first (from --message-format=json),
    /// falling back to text parsing if JSON parsing fails.
    ///
    /// # Arguments
    ///
    /// * `stdout` - Standard output from clippy (contains JSON messages)
    /// * `stderr` - Standard error from clippy (contains text output)
    ///
    /// # Returns
    ///
    /// A vector of structured failure details, limited to first 20 failures.
    fn extract_clippy_errors(stdout: &str, stderr: &str) -> Vec<GateFailureDetail> {
        // Try JSON parsing first
        let failures = Self::parse_clippy_json(stdout);
        if !failures.is_empty() {
            return failures;
        }

        // Fall back to text parsing
        Self::parse_clippy_text(stderr)
    }

    /// Parse clippy JSON output format (from --message-format=json).
    ///
    /// Each line is a separate JSON object representing a compiler message.
    fn parse_clippy_json(stdout: &str) -> Vec<GateFailureDetail> {
        let mut failures = Vec::new();

        for line in stdout.lines() {
            if failures.len() >= Self::MAX_CLIPPY_FAILURES {
                break;
            }

            // Skip empty lines
            if line.trim().is_empty() {
                continue;
            }

            // Try to parse as JSON
            if let Ok(msg) = serde_json::from_str::<ClippyMessage>(line) {
                // Only process compiler messages with actual diagnostics
                if msg.reason == "compiler-message" {
                    if let Some(message) = msg.message {
                        // Only include errors and warnings
                        if matches!(message.level.as_str(), "error" | "warning") {
                            // Skip "aborting due to" messages
                            if message.message.starts_with("aborting due to") {
                                continue;
                            }

                            let mut detail = GateFailureDetail::new(
                                FailureCategory::Lint,
                                message.message.clone(),
                            );

                            // Extract error code
                            if let Some(code) = &message.code {
                                detail = detail.with_error_code(&code.code);
                                // Add explanation URL if available
                                if let Some(ref explanation) = code.explanation {
                                    if !explanation.is_empty() {
                                        detail = detail.with_doc_url(explanation);
                                    }
                                }
                            }

                            // Extract location from spans
                            if let Some(span) = message.spans.first() {
                                detail = detail.with_location(
                                    &span.file_name,
                                    span.line_start,
                                    Some(span.column_start),
                                );

                                // Extract suggestion if available
                                if let Some(ref suggested) = span.suggested_replacement {
                                    if !suggested.is_empty() {
                                        detail = detail.with_suggestion(suggested);
                                    }
                                }
                            }

                            // Check children for suggestions
                            if detail.suggestion.is_none() {
                                for child in &message.children {
                                    if child.level == "help" {
                                        if let Some(span) = child.spans.first() {
                                            if let Some(ref suggested) = span.suggested_replacement
                                            {
                                                if !suggested.is_empty() {
                                                    detail = detail.with_suggestion(suggested);
                                                    break;
                                                }
                                            }
                                        }
                                        // Use child message as suggestion if no replacement
                                        if detail.suggestion.is_none() && !child.message.is_empty()
                                        {
                                            detail = detail.with_suggestion(&child.message);
                                            break;
                                        }
                                    }
                                }
                            }

                            failures.push(detail);
                        }
                    }
                }
            }
        }

        failures
    }

    /// Parse clippy text output (fallback when JSON parsing fails).
    ///
    /// Extracts error information from stderr using regex patterns.
    fn parse_clippy_text(stderr: &str) -> Vec<GateFailureDetail> {
        let mut failures = Vec::new();
        let mut current_message: Option<String> = None;
        let mut current_file: Option<String> = None;
        let mut current_line: Option<u32> = None;
        let mut current_column: Option<u32> = None;

        for line in stderr.lines() {
            if failures.len() >= Self::MAX_CLIPPY_FAILURES {
                break;
            }

            // Match error/warning lines: "error[E0001]: message" or "warning: message"
            if line.starts_with("error") || line.starts_with("warning") {
                // Save previous error if any
                if let Some(msg) = current_message.take() {
                    let mut detail = GateFailureDetail::new(FailureCategory::Lint, msg);
                    if let Some(file) = current_file.take() {
                        detail = detail.with_file(file);
                    }
                    if let Some(line_num) = current_line.take() {
                        detail = detail.with_line(line_num);
                    }
                    if let Some(col) = current_column.take() {
                        detail = detail.with_column(col);
                    }
                    failures.push(detail);
                }

                // Extract message, handling both "error[CODE]: msg" and "error: msg"
                let msg = if line.contains('[') && line.contains("]: ") {
                    if let Some(colon_pos) = line.find("]: ") {
                        line[colon_pos + 3..].to_string()
                    } else {
                        line.to_string()
                    }
                } else if let Some(colon_pos) = line.find(": ") {
                    line[colon_pos + 2..].to_string()
                } else {
                    line.to_string()
                };

                current_message = Some(msg);
            }
            // Match location lines: "  --> src/file.rs:10:5"
            else if line.trim().starts_with("-->") {
                let location = line.trim().trim_start_matches("-->").trim();
                // Parse "file:line:column"
                let parts: Vec<&str> = location.rsplitn(3, ':').collect();
                if parts.len() >= 2 {
                    current_column = parts[0].parse().ok();
                    current_line = parts[1].parse().ok();
                    if parts.len() >= 3 {
                        current_file = Some(parts[2].to_string());
                    }
                }
            }
        }

        // Don't forget the last error
        if let Some(msg) = current_message {
            if failures.len() < Self::MAX_CLIPPY_FAILURES {
                let mut detail = GateFailureDetail::new(FailureCategory::Lint, msg);
                if let Some(file) = current_file {
                    detail = detail.with_file(file);
                }
                if let Some(line_num) = current_line {
                    detail = detail.with_line(line_num);
                }
                if let Some(col) = current_column {
                    detail = detail.with_column(col);
                }
                failures.push(detail);
            }
        }

        failures
    }

    /// Format a summary of clippy failures for the details field.
    fn format_clippy_summary(failures: &[GateFailureDetail]) -> String {
        if failures.is_empty() {
            return "No specific failures extracted".to_string();
        }

        let mut summary = format!("{} issue(s) found:\n", failures.len());
        for (i, failure) in failures.iter().enumerate() {
            if let Some(ref file) = failure.file {
                if let Some(line) = failure.line {
                    summary.push_str(&format!(
                        "{}. {}:{}: {}\n",
                        i + 1,
                        file,
                        line,
                        failure.message
                    ));
                } else {
                    summary.push_str(&format!("{}. {}: {}\n", i + 1, file, failure.message));
                }
            } else {
                summary.push_str(&format!("{}. {}\n", i + 1, failure.message));
            }
        }
        summary
    }

    /// Check code formatting using cargo fmt.
    ///
    /// Runs `cargo fmt --check` which returns non-zero if formatting changes are needed.
    ///
    /// # Returns
    ///
    /// A `GateResult` indicating whether code is properly formatted.
    pub fn check_format(&self) -> GateResult {
        if !self.profile.ci.format_check {
            return GateResult::skipped("format", "Format checking not enabled in profile");
        }

        let output = Command::new("cargo")
            .args(["fmt", "--check"])
            .current_dir(&self.project_root)
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                if output.status.success() {
                    GateResult::pass("format", "All files are properly formatted")
                } else {
                    // cargo fmt --check outputs files that need formatting to stdout
                    let details = Self::extract_format_errors(&stdout, &stderr);
                    GateResult::fail("format", "Some files need formatting", Some(details), None)
                }
            }
            Err(e) => GateResult::fail(
                "format",
                "Failed to run cargo fmt",
                Some(format!("Error: {}. Is rustfmt installed?", e)),
                None,
            ),
        }
    }

    /// Extract relevant information from cargo fmt output.
    fn extract_format_errors(stdout: &str, stderr: &str) -> String {
        // cargo fmt --check outputs "Diff in <file>" for each unformatted file
        let mut result = String::new();

        // Count files needing formatting
        let unformatted_files: Vec<&str> = stdout
            .lines()
            .filter(|line| line.starts_with("Diff in"))
            .collect();

        if !unformatted_files.is_empty() {
            result.push_str(&format!(
                "{} file(s) need formatting:\n",
                unformatted_files.len()
            ));
            for file in unformatted_files.iter().take(20) {
                result.push_str(file);
                result.push('\n');
            }
            if unformatted_files.len() > 20 {
                result.push_str(&format!(
                    "... and {} more files\n",
                    unformatted_files.len() - 20
                ));
            }
            result.push_str("\nRun `cargo fmt` to fix formatting issues.");
        } else if !stderr.is_empty() {
            result = stderr.to_string();
        } else if !stdout.is_empty() {
            result = stdout.to_string();
        } else {
            result = "Formatting check failed (no additional details)".to_string();
        }

        result
    }

    /// Check for security vulnerabilities using cargo-audit.
    ///
    /// Runs `cargo audit` to check for known security vulnerabilities in dependencies.
    ///
    /// # Returns
    ///
    /// A `GateResult` indicating whether any vulnerabilities were found.
    /// If cargo-audit is not installed, returns a failure with installation instructions.
    pub fn check_security_audit(&self) -> GateResult {
        if !self.profile.security.cargo_audit {
            return GateResult::skipped("security_audit", "Security audit not enabled in profile");
        }

        // Check if cargo-audit is installed
        let check_installed = Command::new("cargo")
            .args(["audit", "--version"])
            .current_dir(&self.project_root)
            .output();

        match check_installed {
            Ok(output) if output.status.success() => {
                // cargo-audit is installed, run the audit
                self.run_cargo_audit()
            }
            _ => {
                // cargo-audit is not installed
                GateResult::fail(
                    "security_audit",
                    "cargo-audit is not installed",
                    Some(
                        "Install cargo-audit: cargo install cargo-audit\n\
                         cargo-audit checks for known security vulnerabilities in dependencies."
                            .to_string(),
                    ),
                    None,
                )
            }
        }
    }

    /// Run cargo audit and parse the results.
    fn run_cargo_audit(&self) -> GateResult {
        // Run cargo audit with JSON output for easier parsing
        let output = Command::new("cargo")
            .args(["audit", "--json"])
            .current_dir(&self.project_root)
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                // Try to parse JSON output first
                if let Some(result) = self.parse_audit_json(&stdout) {
                    return result;
                }

                // If JSON parsing fails, fall back to exit code and text parsing
                if output.status.success() {
                    GateResult::pass("security_audit", "No known vulnerabilities found")
                } else {
                    // Non-zero exit means vulnerabilities found or error
                    let details = Self::extract_audit_vulnerabilities(&stdout, &stderr);
                    GateResult::fail(
                        "security_audit",
                        "Security vulnerabilities found",
                        Some(details),
                        None,
                    )
                }
            }
            Err(e) => GateResult::fail(
                "security_audit",
                "Failed to run cargo audit",
                Some(format!("Error: {}", e)),
                None,
            ),
        }
    }

    /// Parse cargo audit JSON output.
    fn parse_audit_json(&self, json_str: &str) -> Option<GateResult> {
        // cargo audit --json outputs a JSON object with vulnerabilities
        // Format: { "vulnerabilities": { "count": N, "list": [...] }, ... }
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
            let vuln_count = json
                .get("vulnerabilities")
                .and_then(|v| v.get("count"))
                .and_then(|c| c.as_u64())
                .unwrap_or(0);

            if vuln_count == 0 {
                return Some(GateResult::pass(
                    "security_audit",
                    "No known vulnerabilities found",
                ));
            }

            // Extract vulnerability details
            let details = self.format_vulnerabilities_from_json(&json, vuln_count);
            return Some(GateResult::fail(
                "security_audit",
                format!(
                    "Found {} known vulnerabilit{}",
                    vuln_count,
                    if vuln_count == 1 { "y" } else { "ies" }
                ),
                Some(details),
                None,
            ));
        }

        None
    }

    /// Format vulnerability details from JSON output.
    fn format_vulnerabilities_from_json(&self, json: &serde_json::Value, count: u64) -> String {
        let mut details = format!(
            "{} vulnerabilit{} found:\n\n",
            count,
            if count == 1 { "y" } else { "ies" }
        );

        if let Some(list) = json
            .get("vulnerabilities")
            .and_then(|v| v.get("list"))
            .and_then(|l| l.as_array())
        {
            for (i, vuln) in list.iter().take(10).enumerate() {
                let advisory = vuln.get("advisory");
                let id = advisory
                    .and_then(|a| a.get("id"))
                    .and_then(|i| i.as_str())
                    .unwrap_or("Unknown");
                let title = advisory
                    .and_then(|a| a.get("title"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("Unknown");
                let severity = advisory
                    .and_then(|a| a.get("severity"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown");
                let package_name = vuln
                    .get("package")
                    .and_then(|p| p.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown");
                let package_version = vuln
                    .get("package")
                    .and_then(|p| p.get("version"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                details.push_str(&format!(
                    "{}. {} ({})\n   Package: {} v{}\n   Severity: {}\n\n",
                    i + 1,
                    id,
                    title,
                    package_name,
                    package_version,
                    severity
                ));
            }

            if list.len() > 10 {
                details.push_str(&format!("... and {} more\n", list.len() - 10));
            }
        }

        details.push_str("\nRun `cargo audit` for full details.");
        details
    }

    /// Extract vulnerability information from text output.
    fn extract_audit_vulnerabilities(stdout: &str, stderr: &str) -> String {
        let mut result = String::new();

        // Look for vulnerability indicators in the output
        let combined = format!("{}\n{}", stdout, stderr);

        // Count warnings/errors
        let warning_count = combined.matches("warning:").count();
        let error_count = combined.matches("error:").count();

        if warning_count > 0 || error_count > 0 {
            result.push_str(&format!(
                "Found {} warning(s) and {} error(s)\n\n",
                warning_count, error_count
            ));
        }

        // Extract lines containing vulnerability IDs (RUSTSEC-YYYY-NNNN)
        let vuln_lines: Vec<&str> = combined
            .lines()
            .filter(|line| {
                line.contains("RUSTSEC-")
                    || line.contains("Crate:")
                    || line.contains("Version:")
                    || line.contains("Title:")
                    || line.contains("warning:")
                    || line.contains("error:")
            })
            .take(50)
            .collect();

        if !vuln_lines.is_empty() {
            result.push_str(&vuln_lines.join("\n"));
        } else if !combined.trim().is_empty() {
            // If no structured output found, include the raw output (limited)
            let truncated: String = combined.chars().take(2000).collect();
            result.push_str(&truncated);
            if combined.len() > 2000 {
                result.push_str("\n... (output truncated)");
            }
        } else {
            result.push_str("Security audit failed (no additional details available)");
        }

        result.push_str("\n\nRun `cargo audit` for full details.");
        result
    }

    /// Run all quality gates configured in the profile.
    ///
    /// Returns a vector of `GateResult` for each gate that was run.
    /// Gates that are not enabled in the profile will be skipped.
    ///
    /// # Returns
    ///
    /// A `Vec<GateResult>` containing the results of all gates.
    pub fn run_all(&self) -> Vec<GateResult> {
        vec![
            self.check_coverage(),
            self.check_lint(),
            self.check_format(),
            self.check_security_audit(),
        ]
    }

    /// Run all quality gates with progress callbacks.
    ///
    /// This method runs all configured quality gates and calls the progress
    /// callback before and after each gate execution:
    ///
    /// - Emits `Running` state before each gate starts
    /// - Emits `Passed` or `Failed` state after each gate completes, with duration
    ///
    /// # Arguments
    ///
    /// * `callback` - A mutable callback that receives progress updates.
    ///   The callback signature is `FnMut(&str, GateProgressState)` for simple
    ///   status tracking, but it also receives duration information via
    ///   `GateProgressUpdate`.
    ///
    /// # Returns
    ///
    /// A `Vec<GateResult>` containing the results of all gates.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let checker = QualityGateChecker::new(profile, project_root);
    /// let results = checker.run_all_gates_with_progress(|update| {
    ///     match update.state {
    ///         GateProgressState::Running => println!("Starting: {}", update.gate_name),
    ///         GateProgressState::Passed => println!("Passed: {} ({:?})", update.gate_name, update.duration),
    ///         GateProgressState::Failed => println!("Failed: {} ({:?})", update.gate_name, update.duration),
    ///     }
    /// });
    /// ```
    pub fn run_all_gates_with_progress<F>(&self, mut callback: F) -> Vec<GateResult>
    where
        F: FnMut(GateProgressUpdate),
    {
        let mut results = Vec::new();

        // Run coverage check
        callback(GateProgressUpdate::running("coverage"));
        let start = Instant::now();
        let result = self.check_coverage();
        let duration = start.elapsed();
        if result.passed {
            callback(GateProgressUpdate::passed("coverage", duration));
        } else {
            callback(GateProgressUpdate::failed("coverage", duration));
        }
        results.push(result);

        // Run lint check
        callback(GateProgressUpdate::running("lint"));
        let start = Instant::now();
        let result = self.check_lint();
        let duration = start.elapsed();
        if result.passed {
            callback(GateProgressUpdate::passed("lint", duration));
        } else {
            callback(GateProgressUpdate::failed("lint", duration));
        }
        results.push(result);

        // Run format check
        callback(GateProgressUpdate::running("format"));
        let start = Instant::now();
        let result = self.check_format();
        let duration = start.elapsed();
        if result.passed {
            callback(GateProgressUpdate::passed("format", duration));
        } else {
            callback(GateProgressUpdate::failed("format", duration));
        }
        results.push(result);

        // Run security audit
        callback(GateProgressUpdate::running("security_audit"));
        let start = Instant::now();
        let result = self.check_security_audit();
        let duration = start.elapsed();
        if result.passed {
            callback(GateProgressUpdate::passed("security_audit", duration));
        } else {
            callback(GateProgressUpdate::failed("security_audit", duration));
        }
        results.push(result);

        results
    }

    /// Check if all gates passed.
    pub fn all_passed(results: &[GateResult]) -> bool {
        results.iter().all(|r| r.passed)
    }

    /// Get a summary of gate results.
    pub fn summary(results: &[GateResult]) -> String {
        let passed = results.iter().filter(|r| r.passed).count();
        let total = results.len();
        let failed: Vec<&str> = results
            .iter()
            .filter(|r| !r.passed)
            .map(|r| r.gate_name.as_str())
            .collect();

        if failed.is_empty() {
            format!("All {total} gates passed")
        } else {
            format!(
                "{passed}/{total} gates passed. Failed: {}",
                failed.join(", ")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::{CiConfig, Profile, SecurityConfig, TestingConfig};

    fn create_test_profile(coverage: u8, lint: bool, format: bool, audit: bool) -> Profile {
        Profile {
            description: "Test profile".to_string(),
            testing: TestingConfig {
                coverage_threshold: coverage,
                unit_tests: true,
                integration_tests: false,
            },
            ci: CiConfig {
                required: true,
                lint_check: lint,
                format_check: format,
            },
            security: SecurityConfig {
                cargo_audit: audit,
                cargo_deny: false,
                sast: false,
            },
            ..Default::default()
        }
    }

    #[test]
    fn test_gate_result_pass() {
        let result = GateResult::pass("test_gate", "Test passed");
        assert!(result.passed);
        assert_eq!(result.gate_name, "test_gate");
        assert_eq!(result.message, "Test passed");
        assert!(result.details.is_none());
    }

    #[test]
    fn test_gate_result_fail() {
        let result = GateResult::fail(
            "test_gate",
            "Test failed",
            Some("Error details".to_string()),
            None,
        );
        assert!(!result.passed);
        assert_eq!(result.gate_name, "test_gate");
        assert_eq!(result.message, "Test failed");
        assert_eq!(result.details, Some("Error details".to_string()));
        assert!(result.failures.is_empty());
    }

    #[test]
    fn test_gate_result_skipped() {
        let result = GateResult::skipped("test_gate", "Not enabled");
        assert!(result.passed); // Skipped counts as passed
        assert_eq!(result.gate_name, "test_gate");
        assert!(result.message.contains("Skipped"));
    }

    #[test]
    fn test_checker_run_all_minimal() {
        let profile = create_test_profile(0, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");
        let results = checker.run_all();

        assert_eq!(results.len(), 4);
        assert!(QualityGateChecker::all_passed(&results));
    }

    #[test]
    fn test_checker_run_all_comprehensive() {
        let profile = create_test_profile(90, true, true, true);
        let checker = QualityGateChecker::new(profile, "/tmp/test");
        let results = checker.run_all();

        assert_eq!(results.len(), 4);
        // Coverage gate may fail if tools not installed, lint/format/security are still skipped
    }

    #[test]
    fn test_all_passed_true() {
        let results = vec![
            GateResult::pass("gate1", "Passed"),
            GateResult::pass("gate2", "Passed"),
        ];
        assert!(QualityGateChecker::all_passed(&results));
    }

    #[test]
    fn test_all_passed_false() {
        let results = vec![
            GateResult::pass("gate1", "Passed"),
            GateResult::fail("gate2", "Failed", None, None),
        ];
        assert!(!QualityGateChecker::all_passed(&results));
    }

    #[test]
    fn test_summary_all_passed() {
        let results = vec![
            GateResult::pass("gate1", "Passed"),
            GateResult::pass("gate2", "Passed"),
        ];
        let summary = QualityGateChecker::summary(&results);
        assert_eq!(summary, "All 2 gates passed");
    }

    #[test]
    fn test_summary_some_failed() {
        let results = vec![
            GateResult::pass("gate1", "Passed"),
            GateResult::fail("gate2", "Failed", None, None),
            GateResult::fail("gate3", "Failed", None, None),
        ];
        let summary = QualityGateChecker::summary(&results);
        assert!(summary.contains("1/3 gates passed"));
        assert!(summary.contains("gate2"));
        assert!(summary.contains("gate3"));
    }

    // Coverage gate tests

    #[test]
    fn test_check_coverage_zero_threshold_skipped() {
        let profile = create_test_profile(0, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");
        let result = checker.check_coverage();

        assert!(result.passed);
        assert_eq!(result.gate_name, "coverage");
        assert!(result.message.contains("Skipped"));
        assert!(result.message.contains("threshold is 0"));
    }

    #[test]
    fn test_check_coverage_with_threshold() {
        // This test checks that the coverage gate attempts to run when threshold > 0
        let profile = create_test_profile(70, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");
        let result = checker.check_coverage();

        // Should either pass (if tools installed) or fail with "no coverage tool available"
        assert_eq!(result.gate_name, "coverage");
        // The result depends on whether coverage tools are installed
        // If not installed, it should fail with a helpful message
        if !result.passed {
            assert!(
                result.message.contains("No coverage tool")
                    || result.message.contains("failed")
                    || result.message.contains("below threshold"),
                "Unexpected failure message: {}",
                result.message
            );
        }
    }

    #[test]
    fn test_parse_coverage_percentage_tarpaulin_format() {
        // Test tarpaulin-style output
        assert_eq!(
            QualityGateChecker::parse_coverage_percentage("75.00% coverage"),
            Some(75.0)
        );
        assert_eq!(
            QualityGateChecker::parse_coverage_percentage("100% coverage"),
            Some(100.0)
        );
        assert_eq!(
            QualityGateChecker::parse_coverage_percentage("0.5% coverage"),
            Some(0.5)
        );
    }

    #[test]
    fn test_parse_coverage_percentage_llvm_cov_format() {
        // Test llvm-cov-style TOTAL line
        assert_eq!(
            QualityGateChecker::parse_coverage_percentage("TOTAL 100 50 50.00%"),
            Some(50.0)
        );
        assert_eq!(
            QualityGateChecker::parse_coverage_percentage(
                "Filename   Functions  Lines\nTOTAL      10         75.50%"
            ),
            Some(75.5)
        );
    }

    #[test]
    fn test_parse_coverage_percentage_invalid() {
        assert_eq!(
            QualityGateChecker::parse_coverage_percentage("no match here"),
            None
        );
        assert_eq!(QualityGateChecker::parse_coverage_percentage(""), None);
    }

    #[test]
    fn test_parse_llvm_cov_json() {
        let json = r#"{
            "data": [{
                "totals": {
                    "lines": {
                        "percent": 82.5
                    }
                }
            }]
        }"#;
        assert_eq!(QualityGateChecker::parse_llvm_cov_json(json), Some(82.5));
    }

    #[test]
    fn test_parse_llvm_cov_json_invalid() {
        assert_eq!(QualityGateChecker::parse_llvm_cov_json("not json"), None);
        assert_eq!(QualityGateChecker::parse_llvm_cov_json("{}"), None);
        assert_eq!(
            QualityGateChecker::parse_llvm_cov_json(r#"{"data": []}"#),
            None
        );
    }

    #[test]
    fn test_evaluate_coverage_pass() {
        let profile = create_test_profile(70, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");
        let result = checker.evaluate_coverage(80.0, "test-tool");

        assert!(result.passed);
        assert!(result.message.contains("80.00%"));
        assert!(result.message.contains("meets threshold"));
        assert!(result.message.contains("70%"));
    }

    #[test]
    fn test_evaluate_coverage_fail() {
        let profile = create_test_profile(70, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");
        let result = checker.evaluate_coverage(50.0, "test-tool");

        assert!(!result.passed);
        assert!(result.message.contains("50.00%"));
        assert!(result.message.contains("below threshold"));
        assert!(result.details.is_some());
        assert!(result.details.unwrap().contains("test-tool"));
    }

    #[test]
    fn test_evaluate_coverage_exact_threshold() {
        let profile = create_test_profile(70, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");
        let result = checker.evaluate_coverage(70.0, "test-tool");

        assert!(result.passed, "Coverage at exactly threshold should pass");
    }

    // Lint gate tests

    #[test]
    fn test_check_lint_disabled() {
        let profile = create_test_profile(0, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");
        let result = checker.check_lint();

        assert!(result.passed);
        assert_eq!(result.gate_name, "lint");
        assert!(result.message.contains("Skipped"));
        assert!(result.message.contains("not enabled"));
    }

    #[test]
    fn test_check_lint_enabled() {
        // This test runs against a real project directory if available
        let profile = create_test_profile(0, true, false, false);
        // Use the actual Ralph project directory for testing
        let project_root = std::env::current_dir().unwrap_or_else(|_| "/tmp/test".into());
        let checker = QualityGateChecker::new(profile, &project_root);
        let result = checker.check_lint();

        assert_eq!(result.gate_name, "lint");
        // Result depends on whether clippy finds issues
        // If it passes or fails, the message should reflect that
        if result.passed {
            assert!(result.message.contains("No clippy warnings"));
        } else {
            assert!(
                result.message.contains("warnings")
                    || result.message.contains("errors")
                    || result.message.contains("Failed"),
                "Unexpected failure message: {}",
                result.message
            );
        }
    }

    #[test]
    fn test_extract_clippy_errors_with_json() {
        // Sample clippy JSON output from --message-format=json
        let stdout = r#"{"reason":"compiler-message","package_id":"test 0.1.0","manifest_path":"/test/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"test","src_path":"/test/src/lib.rs","edition":"2021","doc":true,"doctest":true,"test":true},"message":{"rendered":"warning: unused variable: `x`\n --> src/lib.rs:10:9\n  |\n10 |     let x = 5;\n  |         ^ help: if this is intentional, prefix it with an underscore: `_x`\n  |\n  = note: `#[warn(unused_variables)]` on by default\n\n","$message_type":"diagnostic","children":[{"children":[],"code":null,"level":"help","message":"if this is intentional, prefix it with an underscore","rendered":null,"spans":[{"byte_end":150,"byte_start":149,"column_end":10,"column_start":9,"expansion":null,"file_name":"src/lib.rs","is_primary":true,"label":null,"line_end":10,"line_start":10,"suggested_replacement":"_x","suggestion_applicability":"MachineApplicable","text":[{"highlight_end":10,"highlight_start":9,"text":"    let x = 5;"}]}]}],"code":{"code":"unused_variables","explanation":null},"level":"warning","message":"unused variable: `x`","spans":[{"byte_end":150,"byte_start":149,"column_end":10,"column_start":9,"expansion":null,"file_name":"src/lib.rs","is_primary":true,"label":null,"line_end":10,"line_start":10,"suggested_replacement":null,"suggestion_applicability":null,"text":[{"highlight_end":10,"highlight_start":9,"text":"    let x = 5;"}]}]}}
{"reason":"compiler-message","package_id":"test 0.1.0","manifest_path":"/test/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"test","src_path":"/test/src/lib.rs","edition":"2021","doc":true,"doctest":true,"test":true},"message":{"rendered":"error[E0382]: use of moved value: `s`\n --> src/lib.rs:15:20\n  |\n13 |     let s = String::from(\"hello\");\n  |         - move occurs because `s` has type `String`\n14 |     takes_ownership(s);\n  |                     - value moved here\n15 |     println!(\"{}\", s);\n  |                    ^ value used here after move\n\n","$message_type":"diagnostic","children":[],"code":{"code":"E0382","explanation":"https://doc.rust-lang.org/error-index.html#E0382"},"level":"error","message":"use of moved value: `s`","spans":[{"byte_end":280,"byte_start":279,"column_end":21,"column_start":20,"expansion":null,"file_name":"src/lib.rs","is_primary":true,"label":"value used here after move","line_end":15,"line_start":15,"suggested_replacement":null,"suggestion_applicability":null,"text":[{"highlight_end":21,"highlight_start":20,"text":"    println!(\"{}\", s);"}]}]}}"#;

        let result = QualityGateChecker::extract_clippy_errors(stdout, "");

        assert_eq!(result.len(), 2);

        // Check first failure (warning)
        assert_eq!(result[0].category, FailureCategory::Lint);
        assert_eq!(result[0].message, "unused variable: `x`");
        assert_eq!(result[0].file, Some("src/lib.rs".to_string()));
        assert_eq!(result[0].line, Some(10));
        assert_eq!(result[0].column, Some(9));
        assert_eq!(result[0].error_code, Some("unused_variables".to_string()));
        assert_eq!(result[0].suggestion, Some("_x".to_string()));

        // Check second failure (error)
        assert_eq!(result[1].category, FailureCategory::Lint);
        assert_eq!(result[1].message, "use of moved value: `s`");
        assert_eq!(result[1].file, Some("src/lib.rs".to_string()));
        assert_eq!(result[1].line, Some(15));
        assert_eq!(result[1].error_code, Some("E0382".to_string()));
    }

    #[test]
    fn test_extract_clippy_errors_text_fallback() {
        // When JSON parsing fails/returns empty, fall back to text parsing
        let stderr = r#"error: unused variable: `x`
  --> src/main.rs:10:5
   |
10 |     let x = 5;
   |         ^ help: if this is intentional, prefix it with an underscore: `_x`
   |
   = note: `#[deny(unused_variables)]` on by default

warning: function `foo` is never used
  --> src/main.rs:5:4
   |
5  | fn foo() {}
   |    ^^^
"#;
        let result = QualityGateChecker::extract_clippy_errors("", stderr);

        assert_eq!(result.len(), 2);

        // Check first failure
        assert_eq!(result[0].category, FailureCategory::Lint);
        assert!(result[0].message.contains("unused variable"));
        assert_eq!(result[0].file, Some("src/main.rs".to_string()));
        assert_eq!(result[0].line, Some(10));
        assert_eq!(result[0].column, Some(5));

        // Check second failure
        assert_eq!(result[1].category, FailureCategory::Lint);
        assert!(result[1].message.contains("function `foo` is never used"));
        assert_eq!(result[1].file, Some("src/main.rs".to_string()));
        assert_eq!(result[1].line, Some(5));
    }

    #[test]
    fn test_extract_clippy_errors_empty() {
        let result = QualityGateChecker::extract_clippy_errors("", "");
        assert!(result.is_empty());
    }

    #[test]
    fn test_extract_clippy_errors_limit_20() {
        // Generate more than 20 JSON messages
        let mut stdout = String::new();
        for i in 1..=25 {
            let json = format!(
                r#"{{"reason":"compiler-message","package_id":"test 0.1.0","manifest_path":"/test/Cargo.toml","target":{{"kind":["lib"],"crate_types":["lib"],"name":"test","src_path":"/test/src/lib.rs","edition":"2021","doc":true,"doctest":true,"test":true}},"message":{{"rendered":"warning: unused variable\n","$message_type":"diagnostic","children":[],"code":{{"code":"unused_variables","explanation":null}},"level":"warning","message":"unused variable {}","spans":[{{"byte_end":150,"byte_start":149,"column_end":10,"column_start":9,"expansion":null,"file_name":"src/lib.rs","is_primary":true,"label":null,"line_end":{},"line_start":{},"suggested_replacement":null,"suggestion_applicability":null,"text":[]}}]}}}}"#,
                i, i, i
            );
            stdout.push_str(&json);
            stdout.push('\n');
        }

        let result = QualityGateChecker::extract_clippy_errors(&stdout, "");

        // Should be limited to 20
        assert_eq!(result.len(), 20);
    }

    #[test]
    fn test_parse_clippy_json_skips_aborting_message() {
        let stdout = r#"{"reason":"compiler-message","package_id":"test 0.1.0","manifest_path":"/test/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"test","src_path":"/test/src/lib.rs","edition":"2021","doc":true,"doctest":true,"test":true},"message":{"rendered":"error: aborting due to 1 previous error\n","$message_type":"diagnostic","children":[],"code":null,"level":"error","message":"aborting due to 1 previous error","spans":[]}}"#;

        let result = QualityGateChecker::parse_clippy_json(stdout);

        // Should skip the "aborting due to" message
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_clippy_summary() {
        let failures = vec![
            GateFailureDetail::new(FailureCategory::Lint, "unused variable")
                .with_file("src/lib.rs")
                .with_line(10),
            GateFailureDetail::new(FailureCategory::Lint, "missing docs"),
        ];

        let summary = QualityGateChecker::format_clippy_summary(&failures);

        assert!(summary.contains("2 issue(s) found"));
        assert!(summary.contains("src/lib.rs:10"));
        assert!(summary.contains("unused variable"));
        assert!(summary.contains("missing docs"));
    }

    #[test]
    fn test_format_clippy_summary_empty() {
        let failures: Vec<GateFailureDetail> = vec![];
        let summary = QualityGateChecker::format_clippy_summary(&failures);
        assert_eq!(summary, "No specific failures extracted");
    }

    // Format gate tests

    #[test]
    fn test_check_format_disabled() {
        let profile = create_test_profile(0, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");
        let result = checker.check_format();

        assert!(result.passed);
        assert_eq!(result.gate_name, "format");
        assert!(result.message.contains("Skipped"));
        assert!(result.message.contains("not enabled"));
    }

    #[test]
    fn test_check_format_enabled() {
        // This test runs against a real project directory if available
        let profile = create_test_profile(0, false, true, false);
        // Use the actual Ralph project directory for testing
        let project_root = std::env::current_dir().unwrap_or_else(|_| "/tmp/test".into());
        let checker = QualityGateChecker::new(profile, &project_root);
        let result = checker.check_format();

        assert_eq!(result.gate_name, "format");
        // Result depends on whether files need formatting
        if result.passed {
            assert!(result.message.contains("properly formatted"));
        } else {
            assert!(
                result.message.contains("need formatting") || result.message.contains("Failed"),
                "Unexpected failure message: {}",
                result.message
            );
        }
    }

    #[test]
    fn test_extract_format_errors_with_diffs() {
        let stdout = "Diff in /src/main.rs at line 1:\nDiff in /src/lib.rs at line 5:\n";
        let stderr = "";
        let result = QualityGateChecker::extract_format_errors(stdout, stderr);

        assert!(result.contains("2 file(s) need formatting"));
        assert!(result.contains("cargo fmt"));
    }

    #[test]
    fn test_extract_format_errors_empty() {
        let stdout = "";
        let stderr = "";
        let result = QualityGateChecker::extract_format_errors(stdout, stderr);
        assert!(result.contains("Formatting check failed"));
    }

    #[test]
    fn test_extract_format_errors_with_stderr() {
        let stdout = "";
        let stderr = "error: couldn't parse file";
        let result = QualityGateChecker::extract_format_errors(stdout, stderr);
        assert!(result.contains("couldn't parse"));
    }

    // Security audit gate tests

    #[test]
    fn test_check_security_audit_disabled() {
        let profile = create_test_profile(0, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");
        let result = checker.check_security_audit();

        assert!(result.passed);
        assert_eq!(result.gate_name, "security_audit");
        assert!(result.message.contains("Skipped"));
        assert!(result.message.contains("not enabled"));
    }

    #[test]
    fn test_check_security_audit_enabled() {
        // This test runs against a real project directory if available
        let profile = create_test_profile(0, false, false, true);
        // Use the actual Ralph project directory for testing
        let project_root = std::env::current_dir().unwrap_or_else(|_| "/tmp/test".into());
        let checker = QualityGateChecker::new(profile, &project_root);
        let result = checker.check_security_audit();

        assert_eq!(result.gate_name, "security_audit");
        // Result depends on whether cargo-audit is installed and if vulnerabilities exist
        if result.passed {
            assert!(result.message.contains("No known vulnerabilities"));
        } else {
            // Could fail due to: not installed, vulnerabilities found, or command error
            assert!(
                result.message.contains("not installed")
                    || result.message.contains("vulnerabilit")
                    || result.message.contains("Failed"),
                "Unexpected failure message: {}",
                result.message
            );
        }
    }

    #[test]
    fn test_parse_audit_json_no_vulnerabilities() {
        let profile = create_test_profile(0, false, false, true);
        let checker = QualityGateChecker::new(profile, "/tmp/test");

        let json = r#"{
            "database": {},
            "lockfile": {},
            "vulnerabilities": {
                "count": 0,
                "list": []
            }
        }"#;

        let result = checker.parse_audit_json(json);
        assert!(result.is_some());
        let gate_result = result.unwrap();
        assert!(gate_result.passed);
        assert!(gate_result.message.contains("No known vulnerabilities"));
    }

    #[test]
    fn test_parse_audit_json_with_vulnerabilities() {
        let profile = create_test_profile(0, false, false, true);
        let checker = QualityGateChecker::new(profile, "/tmp/test");

        let json = r#"{
            "database": {},
            "lockfile": {},
            "vulnerabilities": {
                "count": 2,
                "list": [
                    {
                        "advisory": {
                            "id": "RUSTSEC-2021-0001",
                            "title": "Test vulnerability 1",
                            "severity": "high"
                        },
                        "package": {
                            "name": "test-crate",
                            "version": "1.0.0"
                        }
                    },
                    {
                        "advisory": {
                            "id": "RUSTSEC-2021-0002",
                            "title": "Test vulnerability 2",
                            "severity": "medium"
                        },
                        "package": {
                            "name": "another-crate",
                            "version": "2.0.0"
                        }
                    }
                ]
            }
        }"#;

        let result = checker.parse_audit_json(json);
        assert!(result.is_some());
        let gate_result = result.unwrap();
        assert!(!gate_result.passed);
        assert!(gate_result.message.contains("2 known vulnerabilities"));
        let details = gate_result.details.unwrap();
        assert!(details.contains("RUSTSEC-2021-0001"));
        assert!(details.contains("RUSTSEC-2021-0002"));
        assert!(details.contains("test-crate"));
        assert!(details.contains("high"));
    }

    #[test]
    fn test_parse_audit_json_single_vulnerability() {
        let profile = create_test_profile(0, false, false, true);
        let checker = QualityGateChecker::new(profile, "/tmp/test");

        let json = r#"{
            "vulnerabilities": {
                "count": 1,
                "list": [
                    {
                        "advisory": {
                            "id": "RUSTSEC-2022-0001",
                            "title": "Single vuln",
                            "severity": "critical"
                        },
                        "package": {
                            "name": "vulnerable-crate",
                            "version": "0.1.0"
                        }
                    }
                ]
            }
        }"#;

        let result = checker.parse_audit_json(json);
        assert!(result.is_some());
        let gate_result = result.unwrap();
        assert!(!gate_result.passed);
        // Check singular form
        assert!(gate_result.message.contains("1 known vulnerability"));
    }

    #[test]
    fn test_parse_audit_json_invalid() {
        let profile = create_test_profile(0, false, false, true);
        let checker = QualityGateChecker::new(profile, "/tmp/test");

        assert!(checker.parse_audit_json("not json").is_none());
        assert!(checker.parse_audit_json("{}").is_some()); // Valid JSON but no vulnerabilities = 0 count = pass
    }

    #[test]
    fn test_extract_audit_vulnerabilities_with_rustsec() {
        let stdout = r#"
Crate:   test-crate
Version: 1.0.0
Title:   Test vulnerability
RUSTSEC-2021-0001
        "#;
        let stderr = "";

        let result = QualityGateChecker::extract_audit_vulnerabilities(stdout, stderr);
        assert!(result.contains("RUSTSEC-2021-0001"));
        assert!(result.contains("Crate:"));
        assert!(result.contains("cargo audit"));
    }

    #[test]
    fn test_extract_audit_vulnerabilities_with_warnings() {
        let stdout = "";
        let stderr = r#"
warning: 1 vulnerability found!
warning: some other warning
error: critical issue
        "#;

        let result = QualityGateChecker::extract_audit_vulnerabilities(stdout, stderr);
        assert!(result.contains("warning(s)"));
        assert!(result.contains("error(s)"));
    }

    #[test]
    fn test_extract_audit_vulnerabilities_empty() {
        let stdout = "";
        let stderr = "";

        let result = QualityGateChecker::extract_audit_vulnerabilities(stdout, stderr);
        assert!(result.contains("no additional details"));
    }

    #[test]
    fn test_format_vulnerabilities_from_json() {
        let profile = create_test_profile(0, false, false, true);
        let checker = QualityGateChecker::new(profile, "/tmp/test");

        let json: serde_json::Value = serde_json::from_str(
            r#"{
            "vulnerabilities": {
                "count": 1,
                "list": [
                    {
                        "advisory": {
                            "id": "RUSTSEC-2023-0001",
                            "title": "Memory safety issue",
                            "severity": "high"
                        },
                        "package": {
                            "name": "unsafe-crate",
                            "version": "3.0.0"
                        }
                    }
                ]
            }
        }"#,
        )
        .unwrap();

        let details = checker.format_vulnerabilities_from_json(&json, 1);
        assert!(details.contains("1 vulnerability found"));
        assert!(details.contains("RUSTSEC-2023-0001"));
        assert!(details.contains("Memory safety issue"));
        assert!(details.contains("unsafe-crate"));
        assert!(details.contains("v3.0.0"));
        assert!(details.contains("high"));
    }

    // ========================================================================
    // GateProgressState Tests
    // ========================================================================

    #[test]
    fn test_gate_progress_state_equality() {
        assert_eq!(GateProgressState::Running, GateProgressState::Running);
        assert_eq!(GateProgressState::Passed, GateProgressState::Passed);
        assert_eq!(GateProgressState::Failed, GateProgressState::Failed);
        assert_ne!(GateProgressState::Running, GateProgressState::Passed);
        assert_ne!(GateProgressState::Passed, GateProgressState::Failed);
    }

    // ========================================================================
    // GateProgressUpdate Tests
    // ========================================================================

    #[test]
    fn test_gate_progress_update_running() {
        let update = GateProgressUpdate::running("lint");
        assert_eq!(update.gate_name, "lint");
        assert_eq!(update.state, GateProgressState::Running);
        assert!(update.duration.is_none());
        assert!(update.is_running());
        assert!(!update.is_passed());
        assert!(!update.is_failed());
        assert!(!update.is_completed());
    }

    #[test]
    fn test_gate_progress_update_passed() {
        let duration = Duration::from_secs_f64(1.5);
        let update = GateProgressUpdate::passed("format", duration);
        assert_eq!(update.gate_name, "format");
        assert_eq!(update.state, GateProgressState::Passed);
        assert_eq!(update.duration, Some(duration));
        assert!(!update.is_running());
        assert!(update.is_passed());
        assert!(!update.is_failed());
        assert!(update.is_completed());
    }

    #[test]
    fn test_gate_progress_update_failed() {
        let duration = Duration::from_secs_f64(2.3);
        let update = GateProgressUpdate::failed("coverage", duration);
        assert_eq!(update.gate_name, "coverage");
        assert_eq!(update.state, GateProgressState::Failed);
        assert_eq!(update.duration, Some(duration));
        assert!(!update.is_running());
        assert!(!update.is_passed());
        assert!(update.is_failed());
        assert!(update.is_completed());
    }

    #[test]
    fn test_gate_progress_update_format_duration_none() {
        let update = GateProgressUpdate::running("test");
        assert!(update.format_duration().is_none());
    }

    #[test]
    fn test_gate_progress_update_format_duration_seconds() {
        let update = GateProgressUpdate::passed("test", Duration::from_secs_f64(1.234));
        let formatted = update.format_duration().unwrap();
        assert!(formatted.contains("1.2"));
        assert!(formatted.ends_with('s'));
    }

    #[test]
    fn test_gate_progress_update_format_duration_minutes() {
        let update = GateProgressUpdate::passed("test", Duration::from_secs(125));
        let formatted = update.format_duration().unwrap();
        assert!(formatted.contains("2m"));
    }

    // ========================================================================
    // run_all_gates_with_progress Tests
    // ========================================================================

    #[test]
    fn test_run_all_gates_with_progress_emits_running_first() {
        let profile = create_test_profile(0, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");

        let mut updates: Vec<GateProgressUpdate> = Vec::new();
        checker.run_all_gates_with_progress(|update| {
            updates.push(update);
        });

        // Should have 8 updates (Running + Passed/Failed for each of 4 gates)
        assert_eq!(updates.len(), 8);

        // First update should be Running for coverage
        assert!(updates[0].is_running());
        assert_eq!(updates[0].gate_name, "coverage");

        // Second update should be completed for coverage
        assert!(updates[1].is_completed());
        assert_eq!(updates[1].gate_name, "coverage");
    }

    #[test]
    fn test_run_all_gates_with_progress_correct_gate_order() {
        let profile = create_test_profile(0, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");

        let mut gate_names: Vec<String> = Vec::new();
        checker.run_all_gates_with_progress(|update| {
            if update.is_running() {
                gate_names.push(update.gate_name.clone());
            }
        });

        // Should run gates in order: coverage, lint, format, security_audit
        assert_eq!(
            gate_names,
            vec!["coverage", "lint", "format", "security_audit"]
        );
    }

    #[test]
    fn test_run_all_gates_with_progress_includes_duration() {
        let profile = create_test_profile(0, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");

        let mut completed_updates: Vec<GateProgressUpdate> = Vec::new();
        checker.run_all_gates_with_progress(|update| {
            if update.is_completed() {
                completed_updates.push(update);
            }
        });

        // All completed updates should have duration
        for update in &completed_updates {
            assert!(
                update.duration.is_some(),
                "Gate {} should have duration",
                update.gate_name
            );
            assert!(
                update.duration.unwrap().as_nanos() > 0,
                "Duration should be positive"
            );
        }
    }

    #[test]
    fn test_run_all_gates_with_progress_returns_results() {
        let profile = create_test_profile(0, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");

        let mut callback_count = 0;
        let results = checker.run_all_gates_with_progress(|_| {
            callback_count += 1;
        });

        // Should return 4 gate results
        assert_eq!(results.len(), 4);
        assert_eq!(results[0].gate_name, "coverage");
        assert_eq!(results[1].gate_name, "lint");
        assert_eq!(results[2].gate_name, "format");
        assert_eq!(results[3].gate_name, "security_audit");

        // Callback should be called 8 times (2 per gate)
        assert_eq!(callback_count, 8);
    }

    #[test]
    fn test_run_all_gates_with_progress_running_before_complete() {
        let profile = create_test_profile(0, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");

        let mut update_sequence: Vec<(String, GateProgressState)> = Vec::new();
        checker.run_all_gates_with_progress(|update| {
            update_sequence.push((update.gate_name.clone(), update.state));
        });

        // For each gate, Running should come before Passed/Failed
        let gate_order = ["coverage", "lint", "format", "security_audit"];
        for gate in gate_order {
            let running_pos = update_sequence
                .iter()
                .position(|(name, state)| name == gate && *state == GateProgressState::Running);
            let complete_pos = update_sequence.iter().position(|(name, state)| {
                name == gate
                    && matches!(
                        *state,
                        GateProgressState::Passed | GateProgressState::Failed
                    )
            });

            assert!(
                running_pos.is_some(),
                "Gate {} should have Running update",
                gate
            );
            assert!(
                complete_pos.is_some(),
                "Gate {} should have completed update",
                gate
            );
            assert!(
                running_pos.unwrap() < complete_pos.unwrap(),
                "Gate {} Running should come before completed",
                gate
            );
        }
    }

    #[test]
    fn test_run_all_gates_with_progress_matches_run_all_results() {
        let profile = create_test_profile(0, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");

        // Get results from run_all
        let run_all_results = checker.run_all();

        // Get results from run_all_gates_with_progress
        let progress_results = checker.run_all_gates_with_progress(|_| {});

        // Results should match (same gate names and pass/fail status)
        assert_eq!(run_all_results.len(), progress_results.len());
        for (ra, pr) in run_all_results.iter().zip(progress_results.iter()) {
            assert_eq!(ra.gate_name, pr.gate_name);
            assert_eq!(ra.passed, pr.passed);
        }
    }

    #[test]
    fn test_run_all_gates_with_progress_state_matches_result() {
        let profile = create_test_profile(0, false, false, false);
        let checker = QualityGateChecker::new(profile, "/tmp/test");

        let mut completed_states: std::collections::HashMap<String, GateProgressState> =
            std::collections::HashMap::new();

        let results = checker.run_all_gates_with_progress(|update| {
            if update.is_completed() {
                completed_states.insert(update.gate_name.clone(), update.state);
            }
        });

        // Verify that progress state matches result
        for result in results {
            let state = completed_states.get(&result.gate_name).unwrap();
            if result.passed {
                assert_eq!(*state, GateProgressState::Passed);
            } else {
                assert_eq!(*state, GateProgressState::Failed);
            }
        }
    }

    // ========================================================================
    // FailureCategory Tests
    // ========================================================================

    #[test]
    fn test_failure_category_serialization_roundtrip() {
        let categories = [
            FailureCategory::Lint,
            FailureCategory::TypeCheck,
            FailureCategory::Test,
            FailureCategory::Format,
            FailureCategory::Security,
            FailureCategory::Coverage,
        ];

        for category in &categories {
            let json = serde_json::to_string(category).expect("Failed to serialize");
            let deserialized: FailureCategory =
                serde_json::from_str(&json).expect("Failed to deserialize");
            assert_eq!(*category, deserialized);
        }
    }

    #[test]
    fn test_failure_category_snake_case_serialization() {
        assert_eq!(
            serde_json::to_string(&FailureCategory::Lint).unwrap(),
            "\"lint\""
        );
        assert_eq!(
            serde_json::to_string(&FailureCategory::TypeCheck).unwrap(),
            "\"type_check\""
        );
        assert_eq!(
            serde_json::to_string(&FailureCategory::Test).unwrap(),
            "\"test\""
        );
        assert_eq!(
            serde_json::to_string(&FailureCategory::Format).unwrap(),
            "\"format\""
        );
        assert_eq!(
            serde_json::to_string(&FailureCategory::Security).unwrap(),
            "\"security\""
        );
        assert_eq!(
            serde_json::to_string(&FailureCategory::Coverage).unwrap(),
            "\"coverage\""
        );
    }

    // ========================================================================
    // GateFailureDetail Tests
    // ========================================================================

    #[test]
    fn test_gate_failure_detail_new() {
        let detail = GateFailureDetail::new(FailureCategory::Lint, "Unused variable");
        assert_eq!(detail.category, FailureCategory::Lint);
        assert_eq!(detail.message, "Unused variable");
        assert!(detail.file.is_none());
        assert!(detail.line.is_none());
        assert!(detail.column.is_none());
        assert!(detail.error_code.is_none());
        assert!(detail.suggestion.is_none());
        assert!(detail.doc_url.is_none());
    }

    #[test]
    fn test_gate_failure_detail_builder_methods() {
        let detail = GateFailureDetail::new(FailureCategory::TypeCheck, "Type mismatch")
            .with_file("src/main.rs")
            .with_line(42)
            .with_column(10)
            .with_error_code("E0308")
            .with_suggestion("Expected i32, found String")
            .with_doc_url("https://doc.rust-lang.org/error-index.html#E0308");

        assert_eq!(detail.file, Some("src/main.rs".to_string()));
        assert_eq!(detail.line, Some(42));
        assert_eq!(detail.column, Some(10));
        assert_eq!(detail.error_code, Some("E0308".to_string()));
        assert_eq!(detail.category, FailureCategory::TypeCheck);
        assert_eq!(detail.message, "Type mismatch");
        assert_eq!(
            detail.suggestion,
            Some("Expected i32, found String".to_string())
        );
        assert_eq!(
            detail.doc_url,
            Some("https://doc.rust-lang.org/error-index.html#E0308".to_string())
        );
    }

    #[test]
    fn test_gate_failure_detail_with_location() {
        let detail = GateFailureDetail::new(FailureCategory::Lint, "Test").with_location(
            "src/lib.rs",
            100,
            Some(5),
        );

        assert_eq!(detail.file, Some("src/lib.rs".to_string()));
        assert_eq!(detail.line, Some(100));
        assert_eq!(detail.column, Some(5));

        // Test without column
        let detail2 = GateFailureDetail::new(FailureCategory::Lint, "Test").with_location(
            "src/lib.rs",
            200,
            None,
        );

        assert_eq!(detail2.file, Some("src/lib.rs".to_string()));
        assert_eq!(detail2.line, Some(200));
        assert!(detail2.column.is_none());
    }

    #[test]
    fn test_gate_failure_detail_serialization_roundtrip_minimal() {
        let detail = GateFailureDetail::new(FailureCategory::Coverage, "Coverage below threshold");

        let json = serde_json::to_string(&detail).expect("Failed to serialize");
        let deserialized: GateFailureDetail =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(detail.category, deserialized.category);
        assert_eq!(detail.message, deserialized.message);
        assert_eq!(detail.file, deserialized.file);
        assert_eq!(detail.line, deserialized.line);
        assert_eq!(detail.column, deserialized.column);
        assert_eq!(detail.error_code, deserialized.error_code);
        assert_eq!(detail.suggestion, deserialized.suggestion);
        assert_eq!(detail.doc_url, deserialized.doc_url);
    }

    #[test]
    fn test_gate_failure_detail_serialization_roundtrip_full() {
        let detail = GateFailureDetail::new(FailureCategory::Security, "Vulnerability detected")
            .with_file("Cargo.lock")
            .with_line(150)
            .with_column(1)
            .with_error_code("RUSTSEC-2023-0001")
            .with_suggestion("Upgrade the dependency to a patched version")
            .with_doc_url("https://rustsec.org/advisories/RUSTSEC-2023-0001");

        let json = serde_json::to_string(&detail).expect("Failed to serialize");
        let deserialized: GateFailureDetail =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(detail.category, deserialized.category);
        assert_eq!(detail.message, deserialized.message);
        assert_eq!(detail.file, deserialized.file);
        assert_eq!(detail.line, deserialized.line);
        assert_eq!(detail.column, deserialized.column);
        assert_eq!(detail.error_code, deserialized.error_code);
        assert_eq!(detail.suggestion, deserialized.suggestion);
        assert_eq!(detail.doc_url, deserialized.doc_url);
    }

    #[test]
    fn test_gate_failure_detail_skips_none_fields_in_json() {
        let detail = GateFailureDetail::new(FailureCategory::Format, "Unformatted file");

        let json = serde_json::to_string(&detail).expect("Failed to serialize");

        // Optional None fields should not appear in JSON
        assert!(!json.contains("\"file\""));
        assert!(!json.contains("\"line\""));
        assert!(!json.contains("\"column\""));
        assert!(!json.contains("\"error_code\""));
        assert!(!json.contains("\"suggestion\""));
        assert!(!json.contains("\"doc_url\""));

        // Required fields should appear
        assert!(json.contains("\"category\""));
        assert!(json.contains("\"message\""));
    }

    #[test]
    fn test_gate_failure_detail_json_deserialization_with_extra_fields() {
        // Test that deserialization is resilient to extra fields
        let json = r#"{
            "category": "test",
            "message": "Test failed",
            "extra_field": "should be ignored"
        }"#;

        let detail: GateFailureDetail =
            serde_json::from_str(json).expect("Should deserialize despite extra fields");
        assert_eq!(detail.category, FailureCategory::Test);
        assert_eq!(detail.message, "Test failed");
    }

    #[test]
    fn test_gate_failure_detail_all_categories_roundtrip() {
        let categories = [
            FailureCategory::Lint,
            FailureCategory::TypeCheck,
            FailureCategory::Test,
            FailureCategory::Format,
            FailureCategory::Security,
            FailureCategory::Coverage,
        ];

        for category in &categories {
            let detail =
                GateFailureDetail::new(*category, format!("Test message for {:?}", category));

            let json = serde_json::to_string(&detail).expect("Failed to serialize");
            let deserialized: GateFailureDetail =
                serde_json::from_str(&json).expect("Failed to deserialize");

            assert_eq!(detail.category, deserialized.category);
            assert_eq!(detail.message, deserialized.message);
        }
    }

    // ========================================================================
    // GateResult with failures Tests
    // ========================================================================

    #[test]
    fn test_gate_result_fail_with_failures() {
        let failures = vec![
            GateFailureDetail::new(FailureCategory::Lint, "Unused variable")
                .with_file("src/main.rs")
                .with_line(10),
            GateFailureDetail::new(FailureCategory::Lint, "Missing documentation")
                .with_file("src/lib.rs")
                .with_line(20),
        ];
        let result = GateResult::fail(
            "lint",
            "Clippy found warnings",
            Some("Details".to_string()),
            Some(failures.clone()),
        );

        assert!(!result.passed);
        assert_eq!(result.gate_name, "lint");
        assert_eq!(result.failures.len(), 2);
        assert_eq!(result.failures[0].message, "Unused variable");
        assert_eq!(result.failures[1].message, "Missing documentation");
    }

    #[test]
    fn test_gate_result_fail_without_failures() {
        let result = GateResult::fail("test", "Failed", None, None);
        assert!(result.failures.is_empty());
    }

    #[test]
    fn test_gate_result_pass_has_empty_failures() {
        let result = GateResult::pass("lint", "No warnings");
        assert!(result.failures.is_empty());
    }

    #[test]
    fn test_gate_result_skipped_has_empty_failures() {
        let result = GateResult::skipped("coverage", "Not enabled");
        assert!(result.failures.is_empty());
    }

    #[test]
    fn test_gate_result_serialization_without_failures() {
        let result = GateResult::pass("lint", "No warnings");
        let json = serde_json::to_string(&result).expect("Failed to serialize");

        // Empty failures should not appear in JSON (skip_serializing_if = "Vec::is_empty")
        assert!(!json.contains("\"failures\""));
    }

    #[test]
    fn test_gate_result_serialization_with_failures() {
        let failures = vec![GateFailureDetail::new(
            FailureCategory::TypeCheck,
            "Type mismatch",
        )];
        let result = GateResult::fail("typecheck", "Type errors found", None, Some(failures));
        let json = serde_json::to_string(&result).expect("Failed to serialize");

        // Non-empty failures should appear in JSON
        assert!(json.contains("\"failures\""));
        assert!(json.contains("Type mismatch"));
    }

    #[test]
    fn test_gate_result_deserialization_backward_compatible() {
        // Old JSON format without failures field should still deserialize
        let json = r#"{
            "gate_name": "lint",
            "passed": true,
            "message": "No warnings",
            "details": null
        }"#;

        let result: GateResult = serde_json::from_str(json).expect("Should deserialize old format");
        assert_eq!(result.gate_name, "lint");
        assert!(result.passed);
        assert!(result.failures.is_empty()); // Default empty vec
    }

    #[test]
    fn test_gate_result_serialization_roundtrip_with_failures() {
        let failures =
            vec![
                GateFailureDetail::new(FailureCategory::Security, "Vulnerability found")
                    .with_error_code("RUSTSEC-2023-0001")
                    .with_suggestion("Upgrade dependency"),
            ];
        let result = GateResult::fail(
            "security_audit",
            "Security issues",
            Some("1 vulnerability".to_string()),
            Some(failures),
        );

        let json = serde_json::to_string(&result).expect("Failed to serialize");
        let deserialized: GateResult = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(result.gate_name, deserialized.gate_name);
        assert_eq!(result.passed, deserialized.passed);
        assert_eq!(result.message, deserialized.message);
        assert_eq!(result.details, deserialized.details);
        assert_eq!(result.failures.len(), deserialized.failures.len());
        assert_eq!(result.failures[0].message, deserialized.failures[0].message);
        assert_eq!(
            result.failures[0].error_code,
            deserialized.failures[0].error_code
        );
    }
}
