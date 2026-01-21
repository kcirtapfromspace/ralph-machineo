use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::evidence::config::EvidenceStoreConfig;
use crate::evidence::lifecycle::{LifecycleEvent, LifecycleEventType};
use crate::evidence::record::EvidenceRecord;
use crate::evidence::store::EvidenceStore;

/// Evidence writer that records lifecycle events to durable storage.
pub struct EvidenceWriter {
    run_id: String,
    root_dir: PathBuf,
    store: EvidenceStore,
}

impl EvidenceWriter {
    pub fn try_new(base_dir: &Path, run_id: String) -> Result<Self, String> {
        let config = EvidenceStoreConfig::default();
        let store =
            EvidenceStore::new(base_dir, config).map_err(|e| format!("Evidence error: {}", e))?;
        Ok(Self {
            run_id,
            root_dir: store.root_dir().to_path_buf(),
            store,
        })
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    pub fn emit_run_start(&mut self) {
        let event = LifecycleEvent::new(
            LifecycleEventType::RunStart,
            self.run_id.clone(),
            "run".to_string(),
        );
        self.write_event(event);
    }

    pub fn emit_step(
        &mut self,
        step_id: impl Into<String>,
        status: impl Into<String>,
        error_type: Option<String>,
        error_message: Option<String>,
    ) {
        let mut event = LifecycleEvent::new(
            LifecycleEventType::Step,
            self.run_id.clone(),
            step_id.into(),
        );
        event.status = Some(status.into());
        event.error_type = error_type;
        event.error_message = error_message;
        self.write_event(event);
    }

    pub fn emit_run_complete(
        &mut self,
        status: impl Into<String>,
        error_type: Option<String>,
        error_message: Option<String>,
    ) {
        let mut event = LifecycleEvent::new(
            LifecycleEventType::RunComplete,
            self.run_id.clone(),
            "run".to_string(),
        );
        event.status = Some(status.into());
        event.error_type = error_type;
        event.error_message = error_message;
        self.write_event(event);
    }

    fn write_event(&mut self, event: LifecycleEvent) {
        let payload: Value = match serde_json::to_value(&event) {
            Ok(value) => value,
            Err(err) => {
                eprintln!("Warning: Failed to serialize evidence event: {}", err);
                return;
            }
        };

        let record = EvidenceRecord::new(self.run_id.clone(), "lifecycle", payload);
        if let Err(err) = self.store.append_record(&record) {
            eprintln!(
                "Warning: Failed to write evidence event to {}: {}",
                self.root_dir.display(),
                err
            );
        }
    }
}

pub fn generate_run_id() -> String {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let pid = std::process::id();
    format!("run-{}-{}", timestamp_ms, pid)
}
