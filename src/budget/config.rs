//! Token budget configuration.

use serde::{Deserialize, Serialize};

/// Cost per 1000 tokens for different models/operations.
/// These are approximate and can be configured.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCost {
    /// Cost per 1K input tokens (in cents)
    pub input_cost_per_1k: f64,
    /// Cost per 1K output tokens (in cents)
    pub output_cost_per_1k: f64,
    /// Model name for reference
    pub model_name: String,
}

impl Default for TokenCost {
    fn default() -> Self {
        // Claude Sonnet 3.5 pricing as default
        Self {
            input_cost_per_1k: 0.3,  // $0.003 per 1K input
            output_cost_per_1k: 1.5, // $0.015 per 1K output
            model_name: "claude-sonnet".to_string(),
        }
    }
}

impl TokenCost {
    /// Create cost settings for Claude Haiku (cheaper model).
    pub fn haiku() -> Self {
        Self {
            input_cost_per_1k: 0.025,
            output_cost_per_1k: 0.125,
            model_name: "claude-haiku".to_string(),
        }
    }

    /// Create cost settings for Claude Opus (expensive model).
    pub fn opus() -> Self {
        Self {
            input_cost_per_1k: 1.5,
            output_cost_per_1k: 7.5,
            model_name: "claude-opus".to_string(),
        }
    }

    /// Calculate cost for given token counts.
    pub fn calculate_cost(&self, input_tokens: u64, output_tokens: u64) -> f64 {
        let input_cost = (input_tokens as f64 / 1000.0) * self.input_cost_per_1k;
        let output_cost = (output_tokens as f64 / 1000.0) * self.output_cost_per_1k;
        input_cost + output_cost
    }
}

/// Configuration for token budget enforcement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBudgetConfig {
    /// Maximum tokens per story (0 = unlimited)
    pub story_budget: u64,

    /// Maximum total tokens across all stories (0 = unlimited)
    pub total_budget: u64,

    /// Maximum cost in cents (0 = unlimited)
    pub max_cost_cents: f64,

    /// Warning threshold as percentage of budget (0.0 - 1.0)
    /// When usage exceeds this percentage, warnings are emitted
    pub warning_threshold: f64,

    /// Critical threshold as percentage of budget (0.0 - 1.0)
    /// When usage exceeds this percentage, strategy switches to minimal
    pub critical_threshold: f64,

    /// Whether to abort when story budget is exceeded
    pub abort_on_story_budget_exceeded: bool,

    /// Whether to abort when total budget is exceeded
    pub abort_on_total_budget_exceeded: bool,

    /// Cost settings for the model being used
    pub cost_settings: TokenCost,

    /// Reserve buffer for quality gates and git operations (tokens)
    /// This is subtracted from story budget to ensure we have room for finalization
    pub reserve_buffer: u64,

    /// Enable detailed token logging
    pub verbose_logging: bool,
}

impl Default for TokenBudgetConfig {
    fn default() -> Self {
        Self {
            story_budget: 100_000,        // 100K tokens per story
            total_budget: 1_000_000,      // 1M tokens total
            max_cost_cents: 0.0,          // No cost limit by default
            warning_threshold: 0.7,       // Warn at 70%
            critical_threshold: 0.9,      // Critical at 90%
            abort_on_story_budget_exceeded: true,
            abort_on_total_budget_exceeded: true,
            cost_settings: TokenCost::default(),
            reserve_buffer: 5_000, // Reserve 5K tokens for finalization
            verbose_logging: false,
        }
    }
}

impl TokenBudgetConfig {
    /// Create a new budget config with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an unlimited budget config (no enforcement).
    pub fn unlimited() -> Self {
        Self {
            story_budget: 0,
            total_budget: 0,
            max_cost_cents: 0.0,
            warning_threshold: 1.0,
            critical_threshold: 1.0,
            abort_on_story_budget_exceeded: false,
            abort_on_total_budget_exceeded: false,
            cost_settings: TokenCost::default(),
            reserve_buffer: 0,
            verbose_logging: false,
        }
    }

    /// Create a conservative budget config (stricter limits).
    pub fn conservative() -> Self {
        Self {
            story_budget: 50_000,    // 50K tokens per story
            total_budget: 500_000,   // 500K tokens total
            max_cost_cents: 100.0,   // $1.00 max
            warning_threshold: 0.5,  // Warn at 50%
            critical_threshold: 0.8, // Critical at 80%
            abort_on_story_budget_exceeded: true,
            abort_on_total_budget_exceeded: true,
            cost_settings: TokenCost::default(),
            reserve_buffer: 10_000, // Reserve 10K tokens
            verbose_logging: true,
        }
    }

    /// Set the per-story token budget.
    pub fn with_story_budget(mut self, tokens: u64) -> Self {
        self.story_budget = tokens;
        self
    }

    /// Set the total token budget.
    pub fn with_total_budget(mut self, tokens: u64) -> Self {
        self.total_budget = tokens;
        self
    }

    /// Set the maximum cost in cents.
    pub fn with_max_cost(mut self, cents: f64) -> Self {
        self.max_cost_cents = cents;
        self
    }

    /// Set the warning threshold.
    pub fn with_warning_threshold(mut self, threshold: f64) -> Self {
        self.warning_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// Set the critical threshold.
    pub fn with_critical_threshold(mut self, threshold: f64) -> Self {
        self.critical_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// Set whether to abort on story budget exceeded.
    pub fn with_abort_on_story_exceeded(mut self, abort: bool) -> Self {
        self.abort_on_story_budget_exceeded = abort;
        self
    }

    /// Set whether to abort on total budget exceeded.
    pub fn with_abort_on_total_exceeded(mut self, abort: bool) -> Self {
        self.abort_on_total_budget_exceeded = abort;
        self
    }

    /// Set the cost settings.
    pub fn with_cost_settings(mut self, cost: TokenCost) -> Self {
        self.cost_settings = cost;
        self
    }

    /// Set the reserve buffer.
    pub fn with_reserve_buffer(mut self, tokens: u64) -> Self {
        self.reserve_buffer = tokens;
        self
    }

    /// Enable verbose logging.
    pub fn with_verbose_logging(mut self, verbose: bool) -> Self {
        self.verbose_logging = verbose;
        self
    }

    /// Check if budgets are enabled.
    pub fn is_enabled(&self) -> bool {
        self.story_budget > 0 || self.total_budget > 0 || self.max_cost_cents > 0.0
    }

    /// Get the effective story budget (accounting for reserve).
    pub fn effective_story_budget(&self) -> u64 {
        self.story_budget.saturating_sub(self.reserve_buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = TokenBudgetConfig::default();
        assert_eq!(config.story_budget, 100_000);
        assert_eq!(config.total_budget, 1_000_000);
        assert!(config.is_enabled());
    }

    #[test]
    fn test_unlimited_config() {
        let config = TokenBudgetConfig::unlimited();
        assert_eq!(config.story_budget, 0);
        assert_eq!(config.total_budget, 0);
        assert!(!config.is_enabled());
    }

    #[test]
    fn test_conservative_config() {
        let config = TokenBudgetConfig::conservative();
        assert_eq!(config.story_budget, 50_000);
        assert_eq!(config.warning_threshold, 0.5);
    }

    #[test]
    fn test_builder_pattern() {
        let config = TokenBudgetConfig::new()
            .with_story_budget(25_000)
            .with_total_budget(250_000)
            .with_warning_threshold(0.6);

        assert_eq!(config.story_budget, 25_000);
        assert_eq!(config.total_budget, 250_000);
        assert_eq!(config.warning_threshold, 0.6);
    }

    #[test]
    fn test_effective_story_budget() {
        let config = TokenBudgetConfig::new()
            .with_story_budget(50_000)
            .with_reserve_buffer(5_000);

        assert_eq!(config.effective_story_budget(), 45_000);
    }

    #[test]
    fn test_token_cost_calculation() {
        let cost = TokenCost::default();
        let total = cost.calculate_cost(1000, 1000);
        // 1K input at $0.003 + 1K output at $0.015 = $0.018 = 1.8 cents
        assert!((total - 1.8).abs() < 0.001);
    }

    #[test]
    fn test_haiku_cost() {
        let cost = TokenCost::haiku();
        assert!(cost.input_cost_per_1k < TokenCost::default().input_cost_per_1k);
    }

    #[test]
    fn test_opus_cost() {
        let cost = TokenCost::opus();
        assert!(cost.input_cost_per_1k > TokenCost::default().input_cost_per_1k);
    }
}
