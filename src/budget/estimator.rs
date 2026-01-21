//! Token estimation utilities.
//!
//! Since we can't get exact token counts from CLI agents, this module provides
//! estimation based on character counts and heuristics.

use serde::{Deserialize, Serialize};

/// Token estimation strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EstimationMethod {
    /// Simple character-based estimation (4 chars ≈ 1 token for English)
    CharacterBased,
    /// Word-based estimation (1 word ≈ 1.3 tokens)
    WordBased,
    /// Conservative estimation (higher estimate for safety margin)
    Conservative,
}

impl Default for EstimationMethod {
    fn default() -> Self {
        Self::Conservative
    }
}

/// Token estimator for counting tokens from text.
#[derive(Debug, Clone)]
pub struct TokenEstimator {
    method: EstimationMethod,
    /// Multiplier for conservative estimation
    safety_margin: f64,
}

impl Default for TokenEstimator {
    fn default() -> Self {
        Self {
            method: EstimationMethod::Conservative,
            safety_margin: 1.2, // 20% safety margin
        }
    }
}

impl TokenEstimator {
    /// Create a new token estimator with the specified method.
    pub fn new(method: EstimationMethod) -> Self {
        let safety_margin = match method {
            EstimationMethod::CharacterBased => 1.0,
            EstimationMethod::WordBased => 1.0,
            EstimationMethod::Conservative => 1.2,
        };
        Self {
            method,
            safety_margin,
        }
    }

    /// Create a conservative estimator with custom safety margin.
    pub fn conservative(safety_margin: f64) -> Self {
        Self {
            method: EstimationMethod::Conservative,
            safety_margin: safety_margin.max(1.0),
        }
    }

    /// Estimate tokens from text.
    pub fn estimate(&self, text: &str) -> u64 {
        let base_estimate = match self.method {
            EstimationMethod::CharacterBased => self.estimate_by_chars(text),
            EstimationMethod::WordBased => self.estimate_by_words(text),
            EstimationMethod::Conservative => {
                // Use the higher of the two methods
                let char_estimate = self.estimate_by_chars(text);
                let word_estimate = self.estimate_by_words(text);
                char_estimate.max(word_estimate)
            }
        };

        // Apply safety margin
        (base_estimate as f64 * self.safety_margin).ceil() as u64
    }

    /// Estimate tokens based on character count.
    /// Roughly 4 characters per token for English text.
    fn estimate_by_chars(&self, text: &str) -> u64 {
        let chars = text.chars().count();
        // Use 3.5 chars per token to be slightly conservative
        (chars as f64 / 3.5).ceil() as u64
    }

    /// Estimate tokens based on word count.
    /// Roughly 1.3 tokens per word for English text.
    fn estimate_by_words(&self, text: &str) -> u64 {
        let words = text.split_whitespace().count();
        (words as f64 * 1.3).ceil() as u64
    }

    /// Estimate prompt tokens including system overhead.
    ///
    /// This accounts for:
    /// - The actual prompt text
    /// - System message overhead (~500 tokens)
    /// - Tool definitions (~200 tokens per tool)
    /// - Message formatting overhead
    pub fn estimate_prompt(&self, prompt: &str, num_tools: usize) -> u64 {
        let base_estimate = self.estimate(prompt);
        let system_overhead = 500; // System message
        let tool_overhead = num_tools as u64 * 200;
        let formatting_overhead = 100;

        base_estimate + system_overhead + tool_overhead + formatting_overhead
    }

    /// Estimate total tokens for an agent interaction.
    pub fn estimate_interaction(&self, prompt: &str, output: &str) -> TokenCount {
        TokenCount {
            input_tokens: self.estimate(prompt),
            output_tokens: self.estimate(output),
        }
    }

    /// Estimate tokens for a story prompt based on story complexity.
    pub fn estimate_story_prompt(&self, story_title: &str, story_description: &str, acceptance_criteria: &[String], context_lines: usize) -> u64 {
        let mut total = 0u64;

        // Story header
        total += self.estimate(story_title);
        total += self.estimate(story_description);

        // Acceptance criteria
        for criterion in acceptance_criteria {
            total += self.estimate(criterion);
        }

        // Context overhead (previous errors, hints, etc.)
        // Estimate ~50 tokens per context line
        total += (context_lines as u64) * 50;

        // Base prompt template overhead
        total += 300;

        // Apply safety margin
        (total as f64 * self.safety_margin).ceil() as u64
    }
}

/// Token count for an interaction.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TokenCount {
    /// Input/prompt tokens
    pub input_tokens: u64,
    /// Output/completion tokens
    pub output_tokens: u64,
}

impl TokenCount {
    /// Create a new token count.
    pub fn new(input: u64, output: u64) -> Self {
        Self {
            input_tokens: input,
            output_tokens: output,
        }
    }

    /// Get total tokens.
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }

    /// Add another token count.
    pub fn add(&mut self, other: TokenCount) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }
}

impl std::ops::Add for TokenCount {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            input_tokens: self.input_tokens + other.input_tokens,
            output_tokens: self.output_tokens + other.output_tokens,
        }
    }
}

impl std::ops::AddAssign for TokenCount {
    fn add_assign(&mut self, other: Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_char_estimation() {
        let estimator = TokenEstimator::new(EstimationMethod::CharacterBased);
        // "Hello, World!" is 13 chars, ~4 tokens
        let tokens = estimator.estimate("Hello, World!");
        assert!(tokens >= 3 && tokens <= 6);
    }

    #[test]
    fn test_word_estimation() {
        let estimator = TokenEstimator::new(EstimationMethod::WordBased);
        // "Hello World" is 2 words, ~3 tokens
        let tokens = estimator.estimate("Hello World");
        assert!(tokens >= 2 && tokens <= 4);
    }

    #[test]
    fn test_conservative_estimation() {
        let estimator = TokenEstimator::default();
        let text = "This is a test sentence for token estimation.";

        let char_est = TokenEstimator::new(EstimationMethod::CharacterBased).estimate(text);
        let conservative_est = estimator.estimate(text);

        // Conservative should be >= character-based due to safety margin
        assert!(conservative_est >= char_est);
    }

    #[test]
    fn test_empty_string() {
        let estimator = TokenEstimator::default();
        let tokens = estimator.estimate("");
        assert_eq!(tokens, 0);
    }

    #[test]
    fn test_prompt_estimation() {
        let estimator = TokenEstimator::default();
        let prompt = "Implement a function that adds two numbers.";
        let tokens = estimator.estimate_prompt(prompt, 5);

        // Should include overhead
        assert!(tokens > estimator.estimate(prompt));
    }

    #[test]
    fn test_token_count_operations() {
        let count1 = TokenCount::new(100, 200);
        let count2 = TokenCount::new(50, 100);

        let sum = count1 + count2;
        assert_eq!(sum.input_tokens, 150);
        assert_eq!(sum.output_tokens, 300);
        assert_eq!(sum.total(), 450);
    }

    #[test]
    fn test_story_prompt_estimation() {
        let estimator = TokenEstimator::default();
        let tokens = estimator.estimate_story_prompt(
            "Add user authentication",
            "Implement login/logout functionality",
            &["Users can log in".to_string(), "Users can log out".to_string()],
            10, // 10 lines of context
        );

        // Should be substantial due to all components
        assert!(tokens > 500);
    }
}
