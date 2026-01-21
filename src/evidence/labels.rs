use crate::error::classification::ErrorCategory;

/// Convert an error category into a stable evidence label.
pub fn error_category_label(category: &ErrorCategory) -> &'static str {
    match category {
        ErrorCategory::Transient(_) => "transient",
        ErrorCategory::UsageLimit(_) => "usage_limit",
        ErrorCategory::Fatal(_) => "fatal",
        ErrorCategory::Timeout(_) => "timeout",
    }
}
