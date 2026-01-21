use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current evidence schema version.
pub const EVIDENCE_SCHEMA_VERSION: u32 = 1;

/// Evidence record for a run event, metric, or artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    /// Evidence schema version.
    pub schema_version: u32,
    /// Run identifier for correlation.
    pub run_id: String,
    /// Timestamp when the record was captured.
    pub recorded_at: DateTime<Utc>,
    /// Type of evidence stored (e.g. "lifecycle", "metrics").
    pub kind: String,
    /// Arbitrary JSON payload describing the evidence.
    pub payload: Value,
}

impl EvidenceRecord {
    /// Create a new evidence record with the current timestamp.
    pub fn new(run_id: impl Into<String>, kind: impl Into<String>, payload: Value) -> Self {
        Self {
            schema_version: EVIDENCE_SCHEMA_VERSION,
            run_id: run_id.into(),
            recorded_at: Utc::now(),
            kind: kind.into(),
            payload,
        }
    }
}

/// Metadata stored alongside evidence records for a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRunMetadata {
    /// Evidence schema version.
    pub schema_version: u32,
    /// Run identifier.
    pub run_id: String,
    /// Timestamp when the run evidence was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp of the latest record for the run.
    pub updated_at: DateTime<Utc>,
    /// Total number of stored records.
    pub record_count: u64,
}

impl EvidenceRunMetadata {
    /// Create new run metadata.
    pub fn new(run_id: impl Into<String>, timestamp: DateTime<Utc>) -> Self {
        let run_id = run_id.into();
        Self {
            schema_version: EVIDENCE_SCHEMA_VERSION,
            run_id,
            created_at: timestamp,
            updated_at: timestamp,
            record_count: 0,
        }
    }

    /// Update metadata for a newly recorded event.
    pub fn record(&mut self, timestamp: DateTime<Utc>) {
        self.updated_at = timestamp;
        self.record_count = self.record_count.saturating_add(1);
    }
}
