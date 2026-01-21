//! Token budget management for Ralph.
//!
//! This module provides infrastructure for tracking and enforcing token budgets
//! across story executions to prevent runaway costs and enable conservative
//! token management.
//!
//! # Overview
//!
//! The token budget system consists of:
//! - **TokenBudgetConfig**: Configuration for per-story and total budgets
//! - **TokenEstimator**: Estimates token counts from text (since we can't get exact counts from CLI agents)
//! - **TokenBudget**: Tracks usage against configured budgets
//! - **BudgetStrategy**: Adjusts behavior based on remaining budget
//!
//! # Example
//!
//! ```ignore
//! use ralphmacchio::budget::{TokenBudgetConfig, TokenBudget};
//!
//! let config = TokenBudgetConfig::default()
//!     .with_story_budget(50_000)
//!     .with_total_budget(500_000);
//!
//! let mut budget = TokenBudget::new(config);
//!
//! // Record tokens used
//! budget.record_prompt_tokens(2_000);
//! budget.record_output_tokens(5_000);
//!
//! // Check if we can continue
//! if budget.can_continue_story() {
//!     // Proceed with next iteration
//! }
//! ```

mod config;
mod estimator;
mod parser;
mod strategy;
mod tracker;

pub use config::{TokenBudgetConfig, TokenCost};
pub use estimator::TokenEstimator;
pub use parser::{extract_or_estimate, ParsedTokenUsage, TokenUsageParser};
pub use strategy::{BudgetAwarePromptBuilder, BudgetStrategy, PromptStrategy};
pub use tracker::{BudgetEnforcement, BudgetStatus, SharedTokenBudget, StoryBudget, TokenBudget};
