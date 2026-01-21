//! Token usage parsing from agent output.
//!
//! This module extracts actual token usage from CLI agent output.
//! Different agents output usage in different formats:
//!
//! - **Claude CLI**: `{"usage": {"input_tokens": N, "output_tokens": N}}`
//! - **OpenAI/Codex**: `{"usage": {"prompt_tokens": N, "completion_tokens": N}}`
//! - **Anthropic API**: Similar to Claude CLI
//!
//! The parser attempts to extract usage from any of these formats.

use serde::Deserialize;

use super::estimator::TokenCount;

/// Parsed token usage from agent output.
#[derive(Debug, Clone, Default)]
pub struct ParsedTokenUsage {
    /// Input/prompt tokens used
    pub input_tokens: Option<u64>,
    /// Output/completion tokens used
    pub output_tokens: Option<u64>,
    /// Total tokens (if provided separately)
    pub total_tokens: Option<u64>,
    /// Whether this was parsed from actual usage data (vs estimated)
    pub is_actual: bool,
    /// Source of the usage data (e.g., "claude", "openai", "estimated")
    pub source: String,
}

impl ParsedTokenUsage {
    /// Create empty parsed usage.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Create from actual parsed values.
    pub fn actual(input: u64, output: u64, source: impl Into<String>) -> Self {
        Self {
            input_tokens: Some(input),
            output_tokens: Some(output),
            total_tokens: Some(input + output),
            is_actual: true,
            source: source.into(),
        }
    }

    /// Create from estimation.
    pub fn estimated(input: u64, output: u64) -> Self {
        Self {
            input_tokens: Some(input),
            output_tokens: Some(output),
            total_tokens: Some(input + output),
            is_actual: false,
            source: "estimated".to_string(),
        }
    }

    /// Get total tokens.
    pub fn total(&self) -> u64 {
        self.total_tokens
            .or_else(|| {
                match (self.input_tokens, self.output_tokens) {
                    (Some(i), Some(o)) => Some(i + o),
                    (Some(i), None) => Some(i),
                    (None, Some(o)) => Some(o),
                    (None, None) => None,
                }
            })
            .unwrap_or(0)
    }

    /// Convert to TokenCount.
    pub fn to_token_count(&self) -> TokenCount {
        TokenCount::new(
            self.input_tokens.unwrap_or(0),
            self.output_tokens.unwrap_or(0),
        )
    }
}

/// Claude/Anthropic usage format.
#[derive(Debug, Deserialize)]
struct ClaudeUsage {
    input_tokens: u64,
    output_tokens: u64,
}

/// OpenAI usage format.
#[derive(Debug, Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
}

/// Wrapper for extracting usage from JSON.
#[derive(Debug, Deserialize)]
struct UsageWrapper {
    usage: serde_json::Value,
}

/// Token usage parser for extracting actual usage from agent output.
#[derive(Debug, Clone, Default)]
pub struct TokenUsageParser {
    /// Whether to log parsing attempts
    verbose: bool,
}

impl TokenUsageParser {
    /// Create a new parser.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable verbose logging.
    pub fn verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Parse token usage from agent output.
    ///
    /// Attempts to find and parse JSON usage data from the output.
    /// Returns None if no usage data is found.
    pub fn parse(&self, output: &str) -> Option<ParsedTokenUsage> {
        // Try to find JSON objects in the output
        for line in output.lines() {
            let line = line.trim();

            // Skip empty lines and non-JSON lines
            if !line.starts_with('{') {
                continue;
            }

            // Try to parse as JSON with usage field
            if let Some(usage) = self.try_parse_json_line(line) {
                return Some(usage);
            }
        }

        // Also try to find usage in the entire output (for multi-line JSON)
        if let Some(usage) = self.try_extract_usage_object(output) {
            return Some(usage);
        }

        None
    }

    /// Try to parse a single JSON line for usage data.
    fn try_parse_json_line(&self, line: &str) -> Option<ParsedTokenUsage> {
        // Try to parse as a wrapper with usage field
        if let Ok(wrapper) = serde_json::from_str::<UsageWrapper>(line) {
            return self.parse_usage_value(&wrapper.usage);
        }

        None
    }

    /// Parse a usage JSON value into ParsedTokenUsage.
    fn parse_usage_value(&self, value: &serde_json::Value) -> Option<ParsedTokenUsage> {
        // Try Claude/Anthropic format first
        if let Ok(claude) = serde_json::from_value::<ClaudeUsage>(value.clone()) {
            return Some(ParsedTokenUsage::actual(
                claude.input_tokens,
                claude.output_tokens,
                "anthropic",
            ));
        }

        // Try OpenAI format
        if let Ok(openai) = serde_json::from_value::<OpenAIUsage>(value.clone()) {
            return Some(ParsedTokenUsage::actual(
                openai.prompt_tokens,
                openai.completion_tokens,
                "openai",
            ));
        }

        // Try to extract fields manually
        if let serde_json::Value::Object(map) = value {
            let input = map.get("input_tokens")
                .or_else(|| map.get("prompt_tokens"))
                .and_then(|v| v.as_u64());

            let output = map.get("output_tokens")
                .or_else(|| map.get("completion_tokens"))
                .and_then(|v| v.as_u64());

            if let (Some(i), Some(o)) = (input, output) {
                return Some(ParsedTokenUsage::actual(i, o, "parsed"));
            }
        }

        None
    }

    /// Try to extract a usage object from the entire output.
    fn try_extract_usage_object(&self, output: &str) -> Option<ParsedTokenUsage> {
        // Look for "usage": { pattern
        let usage_pattern = "\"usage\"";
        if let Some(idx) = output.find(usage_pattern) {
            // Find the opening brace after "usage":
            let after_usage = &output[idx + usage_pattern.len()..];
            if let Some(colon_idx) = after_usage.find(':') {
                let after_colon = &after_usage[colon_idx + 1..].trim_start();
                if after_colon.starts_with('{') {
                    // Find matching closing brace
                    if let Some(end_idx) = self.find_matching_brace(after_colon) {
                        let usage_json = &after_colon[..=end_idx];
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(usage_json) {
                            return self.parse_usage_value(&value);
                        }
                    }
                }
            }
        }

        None
    }

    /// Find the index of the matching closing brace.
    fn find_matching_brace(&self, s: &str) -> Option<usize> {
        let mut depth = 0;
        for (i, c) in s.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Parse token usage from Claude CLI output.
    ///
    /// Claude CLI with --print flag outputs JSON responses that may include usage.
    pub fn parse_claude_cli(&self, output: &str) -> Option<ParsedTokenUsage> {
        self.parse(output)
    }

    /// Parse token usage from Codex CLI output.
    ///
    /// Codex uses JSON-lines format which may include usage data.
    pub fn parse_codex_cli(&self, output: &str) -> Option<ParsedTokenUsage> {
        // Codex outputs JSON lines, look for usage in any line
        for line in output.lines() {
            if let Some(usage) = self.try_parse_json_line(line.trim()) {
                return Some(usage);
            }
        }
        None
    }
}

/// Extract token usage from agent output, falling back to estimation if not found.
pub fn extract_or_estimate(
    output: &str,
    prompt: &str,
    estimator: &super::TokenEstimator,
) -> ParsedTokenUsage {
    let parser = TokenUsageParser::new();

    // Try to parse actual usage first
    if let Some(usage) = parser.parse(output) {
        return usage;
    }

    // Fall back to estimation
    let input_estimate = estimator.estimate(prompt);
    let output_estimate = estimator.estimate(output);

    ParsedTokenUsage::estimated(input_estimate, output_estimate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_claude_usage() {
        let output = r#"{"usage": {"input_tokens": 1500, "output_tokens": 500}}"#;
        let parser = TokenUsageParser::new();
        let usage = parser.parse(output).unwrap();

        assert!(usage.is_actual);
        assert_eq!(usage.input_tokens, Some(1500));
        assert_eq!(usage.output_tokens, Some(500));
        assert_eq!(usage.total(), 2000);
        assert_eq!(usage.source, "anthropic");
    }

    #[test]
    fn test_parse_openai_usage() {
        let output = r#"{"usage": {"prompt_tokens": 1000, "completion_tokens": 800, "total_tokens": 1800}}"#;
        let parser = TokenUsageParser::new();
        let usage = parser.parse(output).unwrap();

        assert!(usage.is_actual);
        assert_eq!(usage.input_tokens, Some(1000));
        assert_eq!(usage.output_tokens, Some(800));
        assert_eq!(usage.source, "openai");
    }

    #[test]
    fn test_parse_multiline_output() {
        let output = r#"
Some text before
{"id": "msg_123", "content": "Hello", "usage": {"input_tokens": 100, "output_tokens": 50}}
Some text after
"#;
        let parser = TokenUsageParser::new();
        let usage = parser.parse(output).unwrap();

        assert!(usage.is_actual);
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(50));
    }

    #[test]
    fn test_parse_no_usage() {
        let output = "Just some plain text output without any JSON";
        let parser = TokenUsageParser::new();
        let usage = parser.parse(output);

        assert!(usage.is_none());
    }

    #[test]
    fn test_parsed_token_usage_empty() {
        let usage = ParsedTokenUsage::empty();
        assert!(!usage.is_actual);
        assert_eq!(usage.total(), 0);
    }

    #[test]
    fn test_parsed_token_usage_estimated() {
        let usage = ParsedTokenUsage::estimated(1000, 500);
        assert!(!usage.is_actual);
        assert_eq!(usage.source, "estimated");
        assert_eq!(usage.total(), 1500);
    }

    #[test]
    fn test_extract_or_estimate_with_actual() {
        let output = r#"{"usage": {"input_tokens": 200, "output_tokens": 100}}"#;
        let prompt = "Test prompt";
        let estimator = super::super::TokenEstimator::default();

        let usage = extract_or_estimate(output, prompt, &estimator);
        assert!(usage.is_actual);
        assert_eq!(usage.input_tokens, Some(200));
    }

    #[test]
    fn test_extract_or_estimate_falls_back() {
        let output = "Plain text output";
        let prompt = "Test prompt";
        let estimator = super::super::TokenEstimator::default();

        let usage = extract_or_estimate(output, prompt, &estimator);
        assert!(!usage.is_actual);
        assert_eq!(usage.source, "estimated");
    }

    #[test]
    fn test_to_token_count() {
        let usage = ParsedTokenUsage::actual(1000, 500, "test");
        let count = usage.to_token_count();

        assert_eq!(count.input_tokens, 1000);
        assert_eq!(count.output_tokens, 500);
        assert_eq!(count.total(), 1500);
    }

    #[test]
    fn test_parse_embedded_usage() {
        // Simulates output where usage is embedded in a larger response
        let output = r#"{"id":"msg_123","type":"message","role":"assistant","content":[{"type":"text","text":"Hello!"}],"model":"claude-3","stop_reason":"end_turn","usage":{"input_tokens":42,"output_tokens":15}}"#;
        let parser = TokenUsageParser::new();
        let usage = parser.parse(output).unwrap();

        assert!(usage.is_actual);
        assert_eq!(usage.input_tokens, Some(42));
        assert_eq!(usage.output_tokens, Some(15));
    }
}
