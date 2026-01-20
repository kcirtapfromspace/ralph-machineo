use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::evidence::config::EvidenceStoreConfig;
use crate::evidence::lifecycle::{LifecycleEvent, LifecycleEventType};
use crate::evidence::record::{EvidenceRecord, EvidenceRunMetadata, EVIDENCE_SCHEMA_VERSION};
use crate::evidence::store::{EvidenceError, EvidenceResult, EvidenceStore};
use crate::metrics::{RunMetrics, RunMetricsStore};

/// Stable export status for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Success,
    Failed,
    Incomplete,
}

/// Exported evidence bundle for a single run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRunExport {
    pub schema_version: u32,
    pub run_id: String,
    pub status: RunStatus,
    pub metadata: Option<EvidenceRunMetadata>,
    pub metrics: Option<RunMetrics>,
    pub events: Vec<EvidenceRecord>,
}

/// Evidence exporter that assembles run metadata, events, and metrics.
#[derive(Debug, Clone)]
pub struct EvidenceExporter {
    evidence_store: EvidenceStore,
    metrics_store: RunMetricsStore,
}

impl EvidenceExporter {
    /// Create a new exporter rooted at the given base directory.
    pub fn new(base_dir: impl Into<PathBuf>) -> EvidenceResult<Self> {
        let base_dir = base_dir.into();
        let evidence_store = EvidenceStore::new(&base_dir, EvidenceStoreConfig::default())?;
        let metrics_store = RunMetricsStore::new(&base_dir).map_err(EvidenceError::Io)?;
        Ok(Self {
            evidence_store,
            metrics_store,
        })
    }

    /// Export a single run as a consolidated JSON document.
    pub fn export_run(&self, run_id: &str) -> EvidenceResult<EvidenceRunExport> {
        if run_id.trim().is_empty() {
            return Err(EvidenceError::InvalidRunId);
        }

        let metadata = self.evidence_store.load_metadata(run_id)?;
        let events = self.evidence_store.load_events(run_id)?;
        let metrics = self.metrics_store.load(run_id).map_err(EvidenceError::Io)?;
        let status = determine_run_status(&events, metrics.as_ref());

        Ok(EvidenceRunExport {
            schema_version: EVIDENCE_SCHEMA_VERSION,
            run_id: run_id.to_string(),
            status,
            metadata,
            metrics,
            events,
        })
    }
}

fn determine_run_status(events: &[EvidenceRecord], metrics: Option<&RunMetrics>) -> RunStatus {
    if let Some(metrics) = metrics {
        if metrics.expected_steps > 0 && metrics.completeness_percent < 100.0 {
            return RunStatus::Incomplete;
        }
    }

    for record in events.iter().rev() {
        if record.kind != "lifecycle" {
            continue;
        }
        if let Ok(event) = serde_json::from_value::<LifecycleEvent>(record.payload.clone()) {
            if matches!(event.event_type, LifecycleEventType::RunComplete) {
                return if event.status.as_deref() == Some("success") {
                    RunStatus::Success
                } else {
                    RunStatus::Failed
                };
            }
        }
    }

    RunStatus::Incomplete
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use serde_json::to_value;
    use tempfile::TempDir;

    use crate::evidence::record::EvidenceRecord;
    use crate::metrics::RunMetricsCollector;

    #[test]
    fn test_export_run_includes_metrics_and_events() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store = EvidenceStore::new(temp_dir.path(), EvidenceStoreConfig::new(30))
            .expect("evidence store");
        let run_id = "run-123";

        let start_event = LifecycleEvent::new(
            LifecycleEventType::RunStart,
            run_id.to_string(),
            "run".to_string(),
        );
        let mut complete_event = LifecycleEvent::new(
            LifecycleEventType::RunComplete,
            run_id.to_string(),
            "run".to_string(),
        );
        complete_event.status = Some("success".to_string());

        let start_record = EvidenceRecord::new(
            run_id,
            "lifecycle",
            to_value(start_event).expect("start payload"),
        );
        let complete_record = EvidenceRecord::new(
            run_id,
            "lifecycle",
            to_value(complete_event).expect("complete payload"),
        );

        store.append_record(&start_record).expect("append start");
        store
            .append_record(&complete_record)
            .expect("append complete");

        let metrics_collector = RunMetricsCollector::new(run_id, 1);
        metrics_collector.start_step("step-1");
        metrics_collector.complete_step("step-1", true, 1, Duration::from_secs(1), None);
        let metrics = metrics_collector.finish();
        let metrics_store = RunMetricsStore::new(temp_dir.path()).expect("metrics store");
        metrics_store.save(&metrics).expect("save metrics");

        let exporter = EvidenceExporter::new(temp_dir.path()).expect("exporter");
        let export = exporter.export_run(run_id).expect("export run");

        assert_eq!(export.run_id, run_id);
        assert_eq!(export.status, RunStatus::Success);
        assert!(export.metrics.is_some());
        assert_eq!(export.events.len(), 2);
    }

    #[test]
    fn test_export_run_marks_incomplete_when_evidence_missing() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store = EvidenceStore::new(temp_dir.path(), EvidenceStoreConfig::new(30))
            .expect("evidence store");
        let run_id = "run-456";

        let start_event = LifecycleEvent::new(
            LifecycleEventType::RunStart,
            run_id.to_string(),
            "run".to_string(),
        );
        let mut complete_event = LifecycleEvent::new(
            LifecycleEventType::RunComplete,
            run_id.to_string(),
            "run".to_string(),
        );
        complete_event.status = Some("success".to_string());

        let start_record = EvidenceRecord::new(
            run_id,
            "lifecycle",
            to_value(start_event).expect("start payload"),
        );
        let complete_record = EvidenceRecord::new(
            run_id,
            "lifecycle",
            to_value(complete_event).expect("complete payload"),
        );

        store.append_record(&start_record).expect("append start");
        store
            .append_record(&complete_record)
            .expect("append complete");

        let metrics_collector = RunMetricsCollector::new(run_id, 2);
        metrics_collector.record_evidence_step("step-1");
        let metrics = metrics_collector.finish();
        let metrics_store = RunMetricsStore::new(temp_dir.path()).expect("metrics store");
        metrics_store.save(&metrics).expect("save metrics");

        let exporter = EvidenceExporter::new(temp_dir.path()).expect("exporter");
        let export = exporter.export_run(run_id).expect("export run");

        assert_eq!(export.status, RunStatus::Incomplete);
    }
}
