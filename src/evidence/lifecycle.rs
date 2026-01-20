use chrono::{SecondsFormat, Utc};
use serde::Serialize;

const SCHEMA_VERSION: &str = "v1";

/// Lifecycle event types for a run.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleEventType {
    RunStart,
    Step,
    RunComplete,
}

/// Lifecycle event payload stored as evidence.
#[derive(Debug, Serialize)]
pub struct LifecycleEvent {
    pub schema_version: &'static str,
    pub event_type: LifecycleEventType,
    pub timestamp: String,
    pub run_id: String,
    pub step_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl LifecycleEvent {
    pub fn new(event_type: LifecycleEventType, run_id: String, step_id: String) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            event_type,
            timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            run_id,
            step_id,
            status: None,
            error_type: None,
            error_message: None,
        }
    }
}
