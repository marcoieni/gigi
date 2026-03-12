use crate::review::{parse_requires_code_changes, sanitize_review_markdown};

pub(super) fn normalize_review_storage(
    content_md: &str,
    stored_requires_code_changes: bool,
) -> (String, bool) {
    let sanitized = sanitize_review_markdown(content_md);
    let requires_code_changes =
        parse_requires_code_changes(&sanitized).unwrap_or(stored_requires_code_changes);
    (sanitized, requires_code_changes)
}

pub(super) fn bool_to_int(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

pub(super) fn unix_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
