// get_queue_status MCP tool implementation
// This tool returns the current queue status for MCP executions

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Request parameters for the get_queue_status tool.
/// This tool takes no parameters.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetQueueStatusRequest {}

/// Response from the get_queue_status tool.
#[derive(Debug, Serialize)]
pub struct GetQueueStatusResponse {
    /// Number of queued stories waiting to run
    pub queue_size: usize,
    /// Maximum queue capacity
    pub queue_capacity: usize,
    /// Story ID currently running (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running_story_id: Option<String>,
    /// Unix timestamp when the running story started
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running_started_at: Option<u64>,
    /// Elapsed seconds for the running story
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running_elapsed_secs: Option<u64>,
    /// Average duration in seconds from recent completions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_duration_secs: Option<u64>,
    /// Estimated wait time in seconds for the queue to drain
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_wait_secs: Option<u64>,
}
