//! Evidence storage module.

pub mod config;
pub mod export;
pub mod labels;
pub mod lifecycle;
pub mod record;
pub mod store;
pub mod writer;

pub use config::EvidenceStoreConfig;
pub use export::{EvidenceExporter, EvidenceRunExport, RunStatus};
pub use labels::error_category_label;
pub use lifecycle::{LifecycleEvent, LifecycleEventType};
pub use record::{EvidenceRecord, EvidenceRunMetadata, EVIDENCE_SCHEMA_VERSION};
pub use store::{EvidenceError, EvidenceResult, EvidenceStore};
pub use writer::{generate_run_id, EvidenceWriter};
