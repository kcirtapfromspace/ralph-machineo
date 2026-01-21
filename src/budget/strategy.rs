//! Budget-aware execution strategies.
//!
//! This module provides strategies for adjusting agent prompts and behavior
//! based on remaining budget.

use serde::{Deserialize, Serialize};

use super::tracker::{BudgetStatus, TokenBudget};

/// Strategy for prompt generation based on budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptStrategy {
    /// Full prompts with all context and hints
    Full,
    /// Standard prompts with moderate context
    Standard,
    /// Minimal prompts to conserve tokens
    Minimal,
    /// Critical mode - bare minimum context only
    Critical,
}

impl Default for PromptStrategy {
    fn default() -> Self {
        Self::Standard
    }
}

impl PromptStrategy {
    /// Get maximum error history entries for this strategy.
    pub fn max_error_history(&self) -> usize {
        match self {
            PromptStrategy::Full => 10,
            PromptStrategy::Standard => 5,
            PromptStrategy::Minimal => 2,
            PromptStrategy::Critical => 1,
        }
    }

    /// Get maximum affected files to include.
    pub fn max_affected_files(&self) -> usize {
        match self {
            PromptStrategy::Full => 10,
            PromptStrategy::Standard => 5,
            PromptStrategy::Minimal => 2,
            PromptStrategy::Critical => 1,
        }
    }

    /// Whether to include approach hints.
    pub fn include_hints(&self) -> bool {
        matches!(self, PromptStrategy::Full | PromptStrategy::Standard)
    }

    /// Whether to include partial progress.
    pub fn include_partial_progress(&self) -> bool {
        matches!(self, PromptStrategy::Full | PromptStrategy::Standard)
    }

    /// Whether to include full error details.
    pub fn include_full_error_details(&self) -> bool {
        matches!(self, PromptStrategy::Full)
    }

    /// Get context line limit multiplier.
    pub fn context_multiplier(&self) -> f64 {
        match self {
            PromptStrategy::Full => 1.0,
            PromptStrategy::Standard => 0.7,
            PromptStrategy::Minimal => 0.3,
            PromptStrategy::Critical => 0.1,
        }
    }
}

/// Overall budget-aware execution strategy.
#[derive(Debug, Clone)]
pub struct BudgetStrategy {
    /// Current prompt strategy
    pub prompt_strategy: PromptStrategy,
    /// Whether to skip optional quality gates
    pub skip_optional_gates: bool,
    /// Whether to reduce retry attempts
    pub reduce_retries: bool,
    /// Maximum retries allowed (0 = use default)
    pub max_retries: u32,
    /// Whether to prioritize smaller stories
    pub prioritize_small_stories: bool,
    /// Reason for current strategy
    pub reason: String,
}

impl Default for BudgetStrategy {
    fn default() -> Self {
        Self {
            prompt_strategy: PromptStrategy::Standard,
            skip_optional_gates: false,
            reduce_retries: false,
            max_retries: 0,
            prioritize_small_stories: false,
            reason: "Default strategy".to_string(),
        }
    }
}

impl BudgetStrategy {
    /// Create a strategy from budget status.
    pub fn from_budget(budget: &TokenBudget) -> Self {
        let story_status = budget.story_status();
        let total_status = budget.total_status();

        // Use the worst status to determine strategy
        let effective_status = match (story_status, total_status) {
            (BudgetStatus::Exceeded, _) | (_, BudgetStatus::Exceeded) => BudgetStatus::Exceeded,
            (BudgetStatus::Critical, _) | (_, BudgetStatus::Critical) => BudgetStatus::Critical,
            (BudgetStatus::Warning, _) | (_, BudgetStatus::Warning) => BudgetStatus::Warning,
            _ => BudgetStatus::Ok,
        };

        Self::from_status(effective_status)
    }

    /// Create a strategy from a budget status.
    pub fn from_status(status: BudgetStatus) -> Self {
        match status {
            BudgetStatus::Ok => Self::normal(),
            BudgetStatus::Warning => Self::conservative(),
            BudgetStatus::Critical => Self::minimal(),
            BudgetStatus::Exceeded => Self::stop(),
        }
    }

    /// Normal execution strategy.
    pub fn normal() -> Self {
        Self {
            prompt_strategy: PromptStrategy::Standard,
            skip_optional_gates: false,
            reduce_retries: false,
            max_retries: 0,
            prioritize_small_stories: false,
            reason: "Budget OK - normal execution".to_string(),
        }
    }

    /// Conservative strategy for when budget is getting low.
    pub fn conservative() -> Self {
        Self {
            prompt_strategy: PromptStrategy::Standard,
            skip_optional_gates: true,
            reduce_retries: false,
            max_retries: 0,
            prioritize_small_stories: true,
            reason: "Budget warning - conservative execution".to_string(),
        }
    }

    /// Minimal strategy for critical budget situations.
    pub fn minimal() -> Self {
        Self {
            prompt_strategy: PromptStrategy::Minimal,
            skip_optional_gates: true,
            reduce_retries: true,
            max_retries: 2,
            prioritize_small_stories: true,
            reason: "Budget critical - minimal execution".to_string(),
        }
    }

    /// Stop strategy when budget is exceeded.
    pub fn stop() -> Self {
        Self {
            prompt_strategy: PromptStrategy::Critical,
            skip_optional_gates: true,
            reduce_retries: true,
            max_retries: 0, // No retries
            prioritize_small_stories: true,
            reason: "Budget exceeded - stopping execution".to_string(),
        }
    }

    /// Get effective max iterations based on strategy.
    pub fn effective_max_iterations(&self, default_max: u32) -> u32 {
        if self.max_retries > 0 {
            self.max_retries
        } else if self.reduce_retries {
            (default_max / 2).max(1)
        } else {
            default_max
        }
    }

    /// Check if execution should continue.
    pub fn should_continue(&self) -> bool {
        self.max_retries != 0 || !matches!(self.prompt_strategy, PromptStrategy::Critical)
    }
}

/// Builder for constructing budget-aware prompts.
#[derive(Debug, Clone)]
pub struct BudgetAwarePromptBuilder {
    strategy: PromptStrategy,
}

impl BudgetAwarePromptBuilder {
    /// Create a new prompt builder with the given strategy.
    pub fn new(strategy: PromptStrategy) -> Self {
        Self { strategy }
    }

    /// Build error history section based on strategy.
    pub fn build_error_history(
        &self,
        errors: &[crate::iteration::context::IterationError],
    ) -> String {
        let max_errors = self.strategy.max_error_history();
        let errors_to_include: Vec<_> = errors.iter().rev().take(max_errors).collect();

        if errors_to_include.is_empty() {
            return String::new();
        }

        let mut section = String::from("\n### Previous Errors\n\n");

        for error in errors_to_include.iter().rev() {
            section.push_str(&format!(
                "- **Iteration {}** ({}): {}\n",
                error.iteration,
                error.category.as_str(),
                if self.strategy.include_full_error_details() {
                    &error.message
                } else {
                    // Truncate long messages
                    &error.message[..error.message.len().min(100)]
                }
            ));

            if let Some(gate) = &error.failed_gate {
                section.push_str(&format!("  - Failed gate: {}\n", gate));
            }

            if !error.affected_files.is_empty() && self.strategy.max_affected_files() > 0 {
                let files: Vec<_> = error
                    .affected_files
                    .iter()
                    .take(self.strategy.max_affected_files())
                    .cloned()
                    .collect();
                section.push_str(&format!("  - Affected files: {}\n", files.join(", ")));
            }
        }

        section
    }

    /// Build hints section based on strategy.
    pub fn build_hints(&self, hints: &[crate::iteration::context::ApproachHint]) -> String {
        if !self.strategy.include_hints() || hints.is_empty() {
            return String::new();
        }

        let mut section = String::from("\n### Suggested Approaches\n\n");
        let max_hints = match self.strategy {
            PromptStrategy::Full => 5,
            PromptStrategy::Standard => 3,
            _ => 0,
        };

        for hint in hints.iter().take(max_hints) {
            section.push_str(&format!(
                "- {} (success rate: {:.0}%)\n",
                hint.description,
                hint.success_rate * 100.0
            ));
        }

        section
    }

    /// Build partial progress section based on strategy.
    pub fn build_partial_progress(
        &self,
        progress: &std::collections::HashMap<String, Vec<String>>,
    ) -> String {
        if !self.strategy.include_partial_progress() || progress.is_empty() {
            return String::new();
        }

        let mut section = String::from("\n### Partial Progress\n\n");

        for (gate, files) in progress {
            if !files.is_empty() {
                section.push_str(&format!(
                    "- Gate '{}': {} files already passing\n",
                    gate,
                    files.len()
                ));
            }
        }

        section
    }

    /// Get the strategy being used.
    pub fn strategy(&self) -> PromptStrategy {
        self.strategy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::TokenBudgetConfig;

    #[test]
    fn test_prompt_strategy_limits() {
        assert_eq!(PromptStrategy::Full.max_error_history(), 10);
        assert_eq!(PromptStrategy::Minimal.max_error_history(), 2);
        assert_eq!(PromptStrategy::Critical.max_error_history(), 1);
    }

    #[test]
    fn test_prompt_strategy_features() {
        assert!(PromptStrategy::Full.include_hints());
        assert!(PromptStrategy::Standard.include_hints());
        assert!(!PromptStrategy::Minimal.include_hints());
        assert!(!PromptStrategy::Critical.include_hints());
    }

    #[test]
    fn test_budget_strategy_from_status() {
        let normal = BudgetStrategy::from_status(BudgetStatus::Ok);
        assert_eq!(normal.prompt_strategy, PromptStrategy::Standard);
        assert!(!normal.skip_optional_gates);

        let warning = BudgetStrategy::from_status(BudgetStatus::Warning);
        assert!(warning.skip_optional_gates);

        let critical = BudgetStrategy::from_status(BudgetStatus::Critical);
        assert_eq!(critical.prompt_strategy, PromptStrategy::Minimal);
        assert!(critical.reduce_retries);

        let exceeded = BudgetStrategy::from_status(BudgetStatus::Exceeded);
        assert!(!exceeded.should_continue());
    }

    #[test]
    fn test_budget_strategy_from_budget() {
        let config = TokenBudgetConfig::new()
            .with_story_budget(10_000)
            .with_warning_threshold(0.5);

        let mut budget = TokenBudget::new(config);
        budget.start_story("US-001");

        // Under warning
        let strategy = BudgetStrategy::from_budget(&budget);
        assert_eq!(strategy.prompt_strategy, PromptStrategy::Standard);

        // Over warning
        budget.record_iteration(3000, 3000);
        let strategy = BudgetStrategy::from_budget(&budget);
        assert!(strategy.skip_optional_gates);
    }

    #[test]
    fn test_effective_max_iterations() {
        let normal = BudgetStrategy::normal();
        assert_eq!(normal.effective_max_iterations(10), 10);

        let minimal = BudgetStrategy::minimal();
        assert_eq!(minimal.effective_max_iterations(10), 2);

        let conservative = BudgetStrategy::conservative();
        assert_eq!(conservative.effective_max_iterations(10), 10);
    }

    #[test]
    fn test_prompt_builder() {
        use crate::iteration::context::{ErrorCategory, IterationError};

        let builder = BudgetAwarePromptBuilder::new(PromptStrategy::Minimal);

        let errors = vec![
            IterationError::new(1, ErrorCategory::Lint, "Error 1"),
            IterationError::new(2, ErrorCategory::Lint, "Error 2"),
            IterationError::new(3, ErrorCategory::Lint, "Error 3"),
        ];

        let section = builder.build_error_history(&errors);

        // Minimal strategy should only include 2 errors
        assert!(section.contains("Iteration 2"));
        assert!(section.contains("Iteration 3"));
        // Iteration 1 should be excluded due to limit
        assert!(!section.contains("Iteration 1"));
    }
}
