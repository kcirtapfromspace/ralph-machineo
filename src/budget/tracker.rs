//! Token budget tracking and enforcement.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use super::config::TokenBudgetConfig;
use super::estimator::{TokenCount, TokenEstimator};

/// Status of budget usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetStatus {
    /// Under warning threshold, all good
    Ok,
    /// Approaching budget limit (over warning threshold)
    Warning,
    /// Near budget limit (over critical threshold)
    Critical,
    /// Budget exceeded
    Exceeded,
}

impl BudgetStatus {
    /// Check if execution should continue.
    pub fn should_continue(&self) -> bool {
        !matches!(self, BudgetStatus::Exceeded)
    }

    /// Check if warnings should be emitted.
    pub fn should_warn(&self) -> bool {
        matches!(self, BudgetStatus::Warning | BudgetStatus::Critical)
    }
}

/// Result of budget enforcement check.
#[derive(Debug, Clone)]
pub struct BudgetEnforcement {
    /// Whether execution can continue
    pub can_continue: bool,
    /// Current story budget status
    pub story_status: BudgetStatus,
    /// Total budget status
    pub total_status: BudgetStatus,
    /// Cost status (if cost limits are set)
    pub cost_status: BudgetStatus,
    /// Reason if execution should stop
    pub stop_reason: Option<String>,
    /// Recommended action
    pub recommendation: String,
    /// Remaining story tokens
    pub story_remaining: u64,
    /// Remaining total tokens
    pub total_remaining: u64,
    /// Remaining cost budget (cents)
    pub cost_remaining: f64,
}

impl BudgetEnforcement {
    /// Create an enforcement result that allows continuation.
    pub fn allow() -> Self {
        Self {
            can_continue: true,
            story_status: BudgetStatus::Ok,
            total_status: BudgetStatus::Ok,
            cost_status: BudgetStatus::Ok,
            stop_reason: None,
            recommendation: "Continue execution".to_string(),
            story_remaining: u64::MAX,
            total_remaining: u64::MAX,
            cost_remaining: f64::MAX,
        }
    }

    /// Create an enforcement result that stops execution.
    pub fn stop(reason: impl Into<String>) -> Self {
        Self {
            can_continue: false,
            story_status: BudgetStatus::Exceeded,
            total_status: BudgetStatus::Exceeded,
            cost_status: BudgetStatus::Exceeded,
            stop_reason: Some(reason.into()),
            recommendation: "Stop execution and review budget".to_string(),
            story_remaining: 0,
            total_remaining: 0,
            cost_remaining: 0.0,
        }
    }
}

/// Budget tracking for a single story.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryBudget {
    /// Story ID
    pub story_id: String,
    /// Total input tokens used
    pub input_tokens: u64,
    /// Total output tokens used
    pub output_tokens: u64,
    /// Number of iterations
    pub iterations: u32,
    /// Tokens used per iteration
    pub tokens_per_iteration: Vec<TokenCount>,
    /// Budget limit for this story
    pub budget_limit: u64,
    /// When tracking started
    #[serde(skip)]
    pub started_at: Option<Instant>,
}

impl StoryBudget {
    /// Create a new story budget tracker.
    pub fn new(story_id: impl Into<String>, budget_limit: u64) -> Self {
        Self {
            story_id: story_id.into(),
            input_tokens: 0,
            output_tokens: 0,
            iterations: 0,
            tokens_per_iteration: Vec::new(),
            budget_limit,
            started_at: Some(Instant::now()),
        }
    }

    /// Get total tokens used.
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }

    /// Get remaining budget.
    pub fn remaining(&self) -> u64 {
        if self.budget_limit == 0 {
            u64::MAX
        } else {
            self.budget_limit.saturating_sub(self.total_tokens())
        }
    }

    /// Get usage percentage.
    pub fn usage_percent(&self) -> f64 {
        if self.budget_limit == 0 {
            0.0
        } else {
            self.total_tokens() as f64 / self.budget_limit as f64
        }
    }

    /// Record tokens for an iteration.
    pub fn record_iteration(&mut self, tokens: TokenCount) {
        self.iterations += 1;
        self.input_tokens += tokens.input_tokens;
        self.output_tokens += tokens.output_tokens;
        self.tokens_per_iteration.push(tokens);
    }

    /// Get average tokens per iteration.
    pub fn avg_tokens_per_iteration(&self) -> f64 {
        if self.iterations == 0 {
            0.0
        } else {
            self.total_tokens() as f64 / self.iterations as f64
        }
    }

    /// Estimate remaining iterations based on current usage.
    pub fn estimated_remaining_iterations(&self) -> u32 {
        if self.iterations == 0 || self.budget_limit == 0 {
            return u32::MAX;
        }
        let avg = self.avg_tokens_per_iteration();
        if avg <= 0.0 {
            return u32::MAX;
        }
        (self.remaining() as f64 / avg).floor() as u32
    }

    /// Check if budget is exceeded.
    pub fn is_exceeded(&self) -> bool {
        self.budget_limit > 0 && self.total_tokens() >= self.budget_limit
    }
}

/// Main token budget tracker.
#[derive(Debug, Clone)]
pub struct TokenBudget {
    config: TokenBudgetConfig,
    estimator: TokenEstimator,
    /// Total tokens used across all stories
    total_input_tokens: u64,
    total_output_tokens: u64,
    /// Per-story budgets
    story_budgets: HashMap<String, StoryBudget>,
    /// Current active story
    current_story_id: Option<String>,
    /// Total cost incurred (cents)
    total_cost: f64,
}

impl TokenBudget {
    /// Create a new token budget tracker.
    pub fn new(config: TokenBudgetConfig) -> Self {
        Self {
            config,
            estimator: TokenEstimator::default(),
            total_input_tokens: 0,
            total_output_tokens: 0,
            story_budgets: HashMap::new(),
            current_story_id: None,
            total_cost: 0.0,
        }
    }

    /// Create with a custom estimator.
    pub fn with_estimator(config: TokenBudgetConfig, estimator: TokenEstimator) -> Self {
        Self {
            config,
            estimator,
            total_input_tokens: 0,
            total_output_tokens: 0,
            story_budgets: HashMap::new(),
            current_story_id: None,
            total_cost: 0.0,
        }
    }

    /// Start tracking a new story.
    pub fn start_story(&mut self, story_id: impl Into<String>) {
        let id = story_id.into();
        let budget = StoryBudget::new(&id, self.config.story_budget);
        self.story_budgets.insert(id.clone(), budget);
        self.current_story_id = Some(id);
    }

    /// Finish tracking the current story.
    pub fn finish_story(&mut self) {
        self.current_story_id = None;
    }

    /// Get the current story budget.
    pub fn current_story(&self) -> Option<&StoryBudget> {
        self.current_story_id
            .as_ref()
            .and_then(|id| self.story_budgets.get(id))
    }

    /// Get the current story budget mutably.
    fn current_story_mut(&mut self) -> Option<&mut StoryBudget> {
        self.current_story_id
            .as_ref()
            .and_then(|id| self.story_budgets.get_mut(id))
    }

    /// Record tokens used from a prompt.
    pub fn record_prompt(&mut self, prompt: &str) {
        let tokens = self.estimator.estimate(prompt);
        self.record_input_tokens(tokens);
    }

    /// Record tokens used from output.
    pub fn record_output(&mut self, output: &str) {
        let tokens = self.estimator.estimate(output);
        self.record_output_tokens(tokens);
    }

    /// Record input tokens directly.
    pub fn record_input_tokens(&mut self, tokens: u64) {
        self.total_input_tokens += tokens;
        if let Some(story) = self.current_story_mut() {
            story.input_tokens += tokens;
        }
        self.update_cost(tokens, 0);
    }

    /// Record output tokens directly.
    pub fn record_output_tokens(&mut self, tokens: u64) {
        self.total_output_tokens += tokens;
        if let Some(story) = self.current_story_mut() {
            story.output_tokens += tokens;
        }
        self.update_cost(0, tokens);
    }

    /// Record a complete iteration.
    pub fn record_iteration(&mut self, input: u64, output: u64) {
        self.total_input_tokens += input;
        self.total_output_tokens += output;
        if let Some(story) = self.current_story_mut() {
            story.record_iteration(TokenCount::new(input, output));
        }
        self.update_cost(input, output);
    }

    /// Record tokens from text (estimates both prompt and output).
    pub fn record_interaction(&mut self, prompt: &str, output: &str) {
        let count = self.estimator.estimate_interaction(prompt, output);
        self.record_iteration(count.input_tokens, count.output_tokens);
    }

    /// Update cost tracking.
    fn update_cost(&mut self, input_tokens: u64, output_tokens: u64) {
        self.total_cost += self.config.cost_settings.calculate_cost(input_tokens, output_tokens);
    }

    /// Get total tokens used.
    pub fn total_tokens(&self) -> u64 {
        self.total_input_tokens + self.total_output_tokens
    }

    /// Get total cost (cents).
    pub fn total_cost(&self) -> f64 {
        self.total_cost
    }

    /// Get remaining total budget.
    pub fn total_remaining(&self) -> u64 {
        if self.config.total_budget == 0 {
            u64::MAX
        } else {
            self.config.total_budget.saturating_sub(self.total_tokens())
        }
    }

    /// Get total usage percentage.
    pub fn total_usage_percent(&self) -> f64 {
        if self.config.total_budget == 0 {
            0.0
        } else {
            self.total_tokens() as f64 / self.config.total_budget as f64
        }
    }

    /// Get current story remaining budget.
    pub fn story_remaining(&self) -> u64 {
        self.current_story().map(|s| s.remaining()).unwrap_or(u64::MAX)
    }

    /// Get current story usage percentage.
    pub fn story_usage_percent(&self) -> f64 {
        self.current_story().map(|s| s.usage_percent()).unwrap_or(0.0)
    }

    /// Check budget status for current story.
    pub fn story_status(&self) -> BudgetStatus {
        let usage = self.story_usage_percent();
        self.compute_status(usage)
    }

    /// Check budget status for total.
    pub fn total_status(&self) -> BudgetStatus {
        let usage = self.total_usage_percent();
        self.compute_status(usage)
    }

    /// Check cost status.
    pub fn cost_status(&self) -> BudgetStatus {
        if self.config.max_cost_cents <= 0.0 {
            return BudgetStatus::Ok;
        }
        let usage = self.total_cost / self.config.max_cost_cents;
        self.compute_status(usage)
    }

    /// Compute status from usage percentage.
    fn compute_status(&self, usage: f64) -> BudgetStatus {
        if usage >= 1.0 {
            BudgetStatus::Exceeded
        } else if usage >= self.config.critical_threshold {
            BudgetStatus::Critical
        } else if usage >= self.config.warning_threshold {
            BudgetStatus::Warning
        } else {
            BudgetStatus::Ok
        }
    }

    /// Check if execution can continue.
    pub fn can_continue(&self) -> bool {
        self.enforce().can_continue
    }

    /// Check if current story can continue.
    pub fn can_continue_story(&self) -> bool {
        let story_status = self.story_status();
        let total_status = self.total_status();
        let cost_status = self.cost_status();

        // Check if any budget is exceeded and should abort
        if story_status == BudgetStatus::Exceeded && self.config.abort_on_story_budget_exceeded {
            return false;
        }
        if total_status == BudgetStatus::Exceeded && self.config.abort_on_total_budget_exceeded {
            return false;
        }
        if cost_status == BudgetStatus::Exceeded && self.config.max_cost_cents > 0.0 {
            return false;
        }

        true
    }

    /// Perform full budget enforcement check.
    pub fn enforce(&self) -> BudgetEnforcement {
        let story_status = self.story_status();
        let total_status = self.total_status();
        let cost_status = self.cost_status();

        let story_remaining = self.story_remaining();
        let total_remaining = self.total_remaining();
        let cost_remaining = if self.config.max_cost_cents > 0.0 {
            (self.config.max_cost_cents - self.total_cost).max(0.0)
        } else {
            f64::MAX
        };

        // Determine if we should stop
        let mut stop_reason = None;
        let mut can_continue = true;

        if story_status == BudgetStatus::Exceeded && self.config.abort_on_story_budget_exceeded {
            stop_reason = Some(format!(
                "Story budget exceeded: {}/{} tokens",
                self.current_story().map(|s| s.total_tokens()).unwrap_or(0),
                self.config.story_budget
            ));
            can_continue = false;
        }

        if total_status == BudgetStatus::Exceeded && self.config.abort_on_total_budget_exceeded {
            stop_reason = Some(format!(
                "Total budget exceeded: {}/{} tokens",
                self.total_tokens(),
                self.config.total_budget
            ));
            can_continue = false;
        }

        if cost_status == BudgetStatus::Exceeded && self.config.max_cost_cents > 0.0 {
            stop_reason = Some(format!(
                "Cost budget exceeded: ${:.4}/{:.4}",
                self.total_cost / 100.0,
                self.config.max_cost_cents / 100.0
            ));
            can_continue = false;
        }

        // Generate recommendation
        let recommendation = if !can_continue {
            "Stop execution and review budget settings".to_string()
        } else if story_status == BudgetStatus::Critical || total_status == BudgetStatus::Critical {
            "Switch to minimal prompt strategy to conserve tokens".to_string()
        } else if story_status == BudgetStatus::Warning || total_status == BudgetStatus::Warning {
            "Consider reducing context in prompts".to_string()
        } else {
            "Continue normal execution".to_string()
        };

        BudgetEnforcement {
            can_continue,
            story_status,
            total_status,
            cost_status,
            stop_reason,
            recommendation,
            story_remaining,
            total_remaining,
            cost_remaining,
        }
    }

    /// Get a summary of budget usage.
    pub fn summary(&self) -> BudgetSummary {
        BudgetSummary {
            total_input_tokens: self.total_input_tokens,
            total_output_tokens: self.total_output_tokens,
            total_tokens: self.total_tokens(),
            total_cost_cents: self.total_cost,
            stories_tracked: self.story_budgets.len(),
            current_story_id: self.current_story_id.clone(),
            story_budget_limit: self.config.story_budget,
            total_budget_limit: self.config.total_budget,
            cost_limit_cents: self.config.max_cost_cents,
            story_status: self.story_status(),
            total_status: self.total_status(),
            cost_status: self.cost_status(),
        }
    }

    /// Get the configuration.
    pub fn config(&self) -> &TokenBudgetConfig {
        &self.config
    }

    /// Get all story budgets.
    pub fn story_budgets(&self) -> &HashMap<String, StoryBudget> {
        &self.story_budgets
    }
}

/// Summary of budget usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetSummary {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub total_cost_cents: f64,
    pub stories_tracked: usize,
    pub current_story_id: Option<String>,
    pub story_budget_limit: u64,
    pub total_budget_limit: u64,
    pub cost_limit_cents: f64,
    pub story_status: BudgetStatus,
    pub total_status: BudgetStatus,
    pub cost_status: BudgetStatus,
}

impl BudgetSummary {
    /// Format as a human-readable string.
    pub fn format(&self) -> String {
        let mut output = String::from("## Token Budget Summary\n\n");

        output.push_str(&format!(
            "**Tokens Used**: {} ({} input, {} output)\n",
            self.total_tokens, self.total_input_tokens, self.total_output_tokens
        ));

        if self.total_budget_limit > 0 {
            let percent = (self.total_tokens as f64 / self.total_budget_limit as f64) * 100.0;
            output.push_str(&format!(
                "**Total Budget**: {}/{} ({:.1}%) [{}]\n",
                self.total_tokens,
                self.total_budget_limit,
                percent,
                format_status(self.total_status)
            ));
        }

        if self.story_budget_limit > 0 {
            output.push_str(&format!(
                "**Story Budget**: {} per story [{}]\n",
                self.story_budget_limit,
                format_status(self.story_status)
            ));
        }

        if self.cost_limit_cents > 0.0 {
            output.push_str(&format!(
                "**Cost**: ${:.4}/${:.4} [{}]\n",
                self.total_cost_cents / 100.0,
                self.cost_limit_cents / 100.0,
                format_status(self.cost_status)
            ));
        } else {
            output.push_str(&format!(
                "**Estimated Cost**: ${:.4}\n",
                self.total_cost_cents / 100.0
            ));
        }

        output.push_str(&format!("**Stories Tracked**: {}\n", self.stories_tracked));

        if let Some(ref story_id) = self.current_story_id {
            output.push_str(&format!("**Current Story**: {}\n", story_id));
        }

        output
    }
}

/// Format a budget status for display.
fn format_status(status: BudgetStatus) -> &'static str {
    match status {
        BudgetStatus::Ok => "OK",
        BudgetStatus::Warning => "WARNING",
        BudgetStatus::Critical => "CRITICAL",
        BudgetStatus::Exceeded => "EXCEEDED",
    }
}

/// Thread-safe token budget tracker.
#[derive(Debug, Clone)]
pub struct SharedTokenBudget {
    inner: Arc<RwLock<TokenBudget>>,
}

impl SharedTokenBudget {
    /// Create a new shared token budget.
    pub fn new(config: TokenBudgetConfig) -> Self {
        Self {
            inner: Arc::new(RwLock::new(TokenBudget::new(config))),
        }
    }

    /// Start tracking a story.
    pub fn start_story(&self, story_id: impl Into<String>) {
        if let Ok(mut budget) = self.inner.write() {
            budget.start_story(story_id);
        }
    }

    /// Finish tracking a story.
    pub fn finish_story(&self) {
        if let Ok(mut budget) = self.inner.write() {
            budget.finish_story();
        }
    }

    /// Record an iteration.
    pub fn record_iteration(&self, input: u64, output: u64) {
        if let Ok(mut budget) = self.inner.write() {
            budget.record_iteration(input, output);
        }
    }

    /// Record from text.
    pub fn record_interaction(&self, prompt: &str, output: &str) {
        if let Ok(mut budget) = self.inner.write() {
            budget.record_interaction(prompt, output);
        }
    }

    /// Check if execution can continue.
    pub fn can_continue(&self) -> bool {
        self.inner
            .read()
            .map(|b| b.can_continue())
            .unwrap_or(true)
    }

    /// Perform enforcement check.
    pub fn enforce(&self) -> BudgetEnforcement {
        self.inner
            .read()
            .map(|b| b.enforce())
            .unwrap_or_else(|_| BudgetEnforcement::allow())
    }

    /// Get budget summary.
    pub fn summary(&self) -> Option<BudgetSummary> {
        self.inner.read().ok().map(|b| b.summary())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_story_budget_new() {
        let budget = StoryBudget::new("US-001", 50_000);
        assert_eq!(budget.story_id, "US-001");
        assert_eq!(budget.budget_limit, 50_000);
        assert_eq!(budget.total_tokens(), 0);
    }

    #[test]
    fn test_story_budget_recording() {
        let mut budget = StoryBudget::new("US-001", 50_000);
        budget.record_iteration(TokenCount::new(1000, 2000));

        assert_eq!(budget.input_tokens, 1000);
        assert_eq!(budget.output_tokens, 2000);
        assert_eq!(budget.total_tokens(), 3000);
        assert_eq!(budget.iterations, 1);
    }

    #[test]
    fn test_story_budget_remaining() {
        let mut budget = StoryBudget::new("US-001", 50_000);
        budget.record_iteration(TokenCount::new(10_000, 10_000));

        assert_eq!(budget.remaining(), 30_000);
        assert!((budget.usage_percent() - 0.4).abs() < 0.001);
    }

    #[test]
    fn test_story_budget_exceeded() {
        let mut budget = StoryBudget::new("US-001", 10_000);
        budget.record_iteration(TokenCount::new(5000, 6000));

        assert!(budget.is_exceeded());
    }

    #[test]
    fn test_token_budget_new() {
        let config = TokenBudgetConfig::default();
        let budget = TokenBudget::new(config);

        assert_eq!(budget.total_tokens(), 0);
        assert!(budget.can_continue());
    }

    #[test]
    fn test_token_budget_story_tracking() {
        let config = TokenBudgetConfig::new().with_story_budget(10_000);
        let mut budget = TokenBudget::new(config);

        budget.start_story("US-001");
        budget.record_iteration(2000, 3000);

        assert_eq!(budget.total_tokens(), 5000);
        assert_eq!(budget.story_remaining(), 5000);
    }

    #[test]
    fn test_token_budget_enforcement() {
        let config = TokenBudgetConfig::new()
            .with_story_budget(10_000)
            .with_warning_threshold(0.5)
            .with_critical_threshold(0.8);
        let mut budget = TokenBudget::new(config);

        budget.start_story("US-001");

        // Under warning
        budget.record_iteration(2000, 1000);
        assert_eq!(budget.story_status(), BudgetStatus::Ok);

        // Over warning
        budget.record_iteration(2000, 1000);
        assert_eq!(budget.story_status(), BudgetStatus::Warning);

        // Over critical
        budget.record_iteration(2000, 1000);
        assert_eq!(budget.story_status(), BudgetStatus::Critical);

        // Exceeded
        budget.record_iteration(2000, 1000);
        assert_eq!(budget.story_status(), BudgetStatus::Exceeded);
        assert!(!budget.can_continue_story());
    }

    #[test]
    fn test_token_budget_total_tracking() {
        let config = TokenBudgetConfig::new().with_total_budget(50_000);
        let mut budget = TokenBudget::new(config);

        budget.start_story("US-001");
        budget.record_iteration(10_000, 10_000);
        budget.finish_story();

        budget.start_story("US-002");
        budget.record_iteration(10_000, 10_000);

        assert_eq!(budget.total_tokens(), 40_000);
        assert_eq!(budget.total_usage_percent(), 0.8);
    }

    #[test]
    fn test_budget_summary() {
        let config = TokenBudgetConfig::new()
            .with_story_budget(10_000)
            .with_total_budget(50_000);
        let mut budget = TokenBudget::new(config);

        budget.start_story("US-001");
        budget.record_iteration(5000, 3000);

        let summary = budget.summary();
        assert_eq!(summary.total_tokens, 8000);
        assert_eq!(summary.stories_tracked, 1);
        assert_eq!(summary.current_story_id, Some("US-001".to_string()));
    }

    #[test]
    fn test_shared_budget() {
        let config = TokenBudgetConfig::new().with_story_budget(50_000);
        let budget = SharedTokenBudget::new(config);

        budget.start_story("US-001");
        budget.record_iteration(5000, 5000);

        assert!(budget.can_continue());
        let summary = budget.summary().unwrap();
        assert_eq!(summary.total_tokens, 10_000);
    }

    #[test]
    fn test_cost_tracking() {
        let config = TokenBudgetConfig::new().with_max_cost(100.0); // $1.00
        let mut budget = TokenBudget::new(config);

        budget.start_story("US-001");
        budget.record_iteration(10_000, 10_000);

        // Default pricing: ~$0.18 for 10K/10K
        assert!(budget.total_cost() > 0.0);
        assert!(budget.total_cost() < 100.0);
    }

    #[test]
    fn test_unlimited_budget() {
        let config = TokenBudgetConfig::unlimited();
        let mut budget = TokenBudget::new(config);

        budget.start_story("US-001");
        budget.record_iteration(1_000_000, 1_000_000);

        assert!(budget.can_continue());
        assert_eq!(budget.story_status(), BudgetStatus::Ok);
    }
}
