use std::env;

/// Environment variable for evidence retention period (days).
pub const RETENTION_ENV_VAR: &str = "RALPH_EVIDENCE_RETENTION_DAYS";

/// Default retention period in days.
pub const DEFAULT_RETENTION_DAYS: u64 = 30;

/// Configuration for evidence storage.
#[derive(Debug, Clone)]
pub struct EvidenceStoreConfig {
    /// Retention period in days (0 disables retention pruning).
    pub retention_days: u64,
}

impl EvidenceStoreConfig {
    /// Create a new config with the specified retention period.
    pub fn new(retention_days: u64) -> Self {
        Self { retention_days }
    }

    /// Build config from environment variables.
    pub fn from_env() -> Self {
        let retention_days = env::var(RETENTION_ENV_VAR)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_RETENTION_DAYS);
        Self { retention_days }
    }
}

impl Default for EvidenceStoreConfig {
    fn default() -> Self {
        Self::from_env()
    }
}
