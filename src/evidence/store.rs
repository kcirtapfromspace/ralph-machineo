use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use thiserror::Error;

use crate::evidence::config::EvidenceStoreConfig;
use crate::evidence::record::{EvidenceRecord, EvidenceRunMetadata};

const RALPH_DIR_NAME: &str = ".ralph";
const EVIDENCE_DIR_NAME: &str = "evidence";
const RUNS_DIR_NAME: &str = "runs";
const MANIFEST_FILE_NAME: &str = "run.json";
const EVENTS_FILE_NAME: &str = "events.jsonl";

/// Errors that can occur during evidence storage operations.
#[derive(Error, Debug)]
pub enum EvidenceError {
    /// IO error during file operations.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Invalid run identifier.
    #[error("Invalid run ID")]
    InvalidRunId,
}

/// Result type for evidence storage operations.
pub type EvidenceResult<T> = Result<T, EvidenceError>;

/// Evidence store backed by the local filesystem.
#[derive(Debug, Clone)]
pub struct EvidenceStore {
    root_dir: PathBuf,
    retention_days: u64,
}

impl EvidenceStore {
    /// Create a new evidence store rooted at the given base directory.
    pub fn new(base_dir: impl Into<PathBuf>, config: EvidenceStoreConfig) -> EvidenceResult<Self> {
        let root_dir = base_dir.into().join(RALPH_DIR_NAME).join(EVIDENCE_DIR_NAME);
        let runs_dir = root_dir.join(RUNS_DIR_NAME);
        fs::create_dir_all(&runs_dir)?;
        Ok(Self {
            root_dir,
            retention_days: config.retention_days,
        })
    }

    /// Append a single evidence record for a run.
    pub fn append_record(&self, record: &EvidenceRecord) -> EvidenceResult<()> {
        if record.run_id.trim().is_empty() {
            return Err(EvidenceError::InvalidRunId);
        }

        let run_dir = self.run_dir(&record.run_id);
        fs::create_dir_all(&run_dir)?;

        let events_path = run_dir.join(EVENTS_FILE_NAME);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&events_path)?;

        let json = serde_json::to_string(record)?;
        writeln!(file, "{}", json)?;
        file.sync_all()?;

        let mut metadata = self.load_or_create_metadata(&run_dir, record)?;
        metadata.record(record.recorded_at);
        self.write_metadata(&run_dir, &metadata)?;

        Ok(())
    }

    /// Delete all evidence for a specific run.
    pub fn delete_run(&self, run_id: &str) -> EvidenceResult<()> {
        if run_id.trim().is_empty() {
            return Err(EvidenceError::InvalidRunId);
        }

        let run_dir = self.run_dir(run_id);
        match fs::remove_dir_all(&run_dir) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(EvidenceError::Io(err)),
        }
    }

    /// Apply retention rules and delete expired runs.
    pub fn enforce_retention(&self) -> EvidenceResult<usize> {
        if self.retention_days == 0 {
            return Ok(0);
        }

        let runs_dir = self.root_dir.join(RUNS_DIR_NAME);
        if !runs_dir.exists() {
            return Ok(0);
        }

        let cutoff = Utc::now() - Duration::days(self.retention_days as i64);
        let mut deleted = 0;

        for entry in fs::read_dir(&runs_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }

            let run_dir = entry.path();
            let manifest_path = run_dir.join(MANIFEST_FILE_NAME);
            let Some(metadata) = self.read_metadata(&manifest_path)? else {
                continue;
            };

            if metadata.created_at < cutoff {
                fs::remove_dir_all(&run_dir)?;
                deleted += 1;
            }
        }

        Ok(deleted)
    }

    /// Get the evidence root directory path.
    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    fn run_dir(&self, run_id: &str) -> PathBuf {
        self.root_dir.join(RUNS_DIR_NAME).join(run_id)
    }

    fn load_or_create_metadata(
        &self,
        run_dir: &Path,
        record: &EvidenceRecord,
    ) -> EvidenceResult<EvidenceRunMetadata> {
        let manifest_path = run_dir.join(MANIFEST_FILE_NAME);
        if let Some(metadata) = self.read_metadata(&manifest_path)? {
            Ok(metadata)
        } else {
            Ok(EvidenceRunMetadata::new(
                record.run_id.clone(),
                record.recorded_at,
            ))
        }
    }

    fn read_metadata(&self, manifest_path: &Path) -> EvidenceResult<Option<EvidenceRunMetadata>> {
        match fs::read_to_string(manifest_path) {
            Ok(content) => {
                let metadata = serde_json::from_str(&content)?;
                Ok(Some(metadata))
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(EvidenceError::Io(err)),
        }
    }

    fn write_metadata(&self, run_dir: &Path, metadata: &EvidenceRunMetadata) -> EvidenceResult<()> {
        let json = serde_json::to_string_pretty(metadata)?;
        let temp_path = run_dir.join(format!("{}.tmp", MANIFEST_FILE_NAME));
        let manifest_path = run_dir.join(MANIFEST_FILE_NAME);

        let mut file = fs::File::create(&temp_path)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp_path, &manifest_path)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn test_append_record_writes_files() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store =
            EvidenceStore::new(temp_dir.path(), EvidenceStoreConfig::new(30)).expect("store");
        let record = EvidenceRecord::new("run-123", "lifecycle", json!({"event": "start"}));

        store.append_record(&record).expect("append");

        let run_dir = store.root_dir().join(RUNS_DIR_NAME).join("run-123");
        assert!(run_dir.join(EVENTS_FILE_NAME).exists());
        assert!(run_dir.join(MANIFEST_FILE_NAME).exists());
    }

    #[test]
    fn test_delete_run_removes_evidence() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store =
            EvidenceStore::new(temp_dir.path(), EvidenceStoreConfig::new(30)).expect("store");
        let record = EvidenceRecord::new("run-999", "metrics", json!({"count": 1}));

        store.append_record(&record).expect("append");
        store.delete_run("run-999").expect("delete");

        let run_dir = store.root_dir().join(RUNS_DIR_NAME).join("run-999");
        assert!(!run_dir.exists());
    }

    #[test]
    fn test_enforce_retention_deletes_expired_runs() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store =
            EvidenceStore::new(temp_dir.path(), EvidenceStoreConfig::new(30)).expect("store");

        let record = EvidenceRecord::new("run-old", "lifecycle", json!({"event": "start"}));
        store.append_record(&record).expect("append");

        let run_dir = store.root_dir().join(RUNS_DIR_NAME).join("run-old");
        let manifest_path = run_dir.join(MANIFEST_FILE_NAME);
        let mut metadata: EvidenceRunMetadata =
            serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest"))
                .expect("metadata");
        metadata.created_at = Utc::now() - Duration::days(45);
        metadata.updated_at = metadata.created_at;
        let json = serde_json::to_string_pretty(&metadata).expect("serialize");
        fs::write(&manifest_path, json).expect("write");

        let deleted = store.enforce_retention().expect("retention");
        assert_eq!(deleted, 1);
        assert!(!run_dir.exists());
    }

    #[test]
    fn test_enforce_retention_keeps_recent_runs() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store =
            EvidenceStore::new(temp_dir.path(), EvidenceStoreConfig::new(30)).expect("store");
        let record = EvidenceRecord::new("run-new", "lifecycle", json!({"event": "start"}));

        store.append_record(&record).expect("append");
        let deleted = store.enforce_retention().expect("retention");

        let run_dir = store.root_dir().join(RUNS_DIR_NAME).join("run-new");
        assert_eq!(deleted, 0);
        assert!(run_dir.exists());
    }

    #[test]
    fn test_enforce_retention_disabled() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store =
            EvidenceStore::new(temp_dir.path(), EvidenceStoreConfig::new(0)).expect("store");
        let record = EvidenceRecord::new("run-keep", "lifecycle", json!({"event": "start"}));

        store.append_record(&record).expect("append");
        let deleted = store.enforce_retention().expect("retention");

        let run_dir = store.root_dir().join(RUNS_DIR_NAME).join("run-keep");
        assert_eq!(deleted, 0);
        assert!(run_dir.exists());
    }
}
