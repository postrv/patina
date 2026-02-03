//! Session formatting utilities for display.
//!
//! Provides human-readable formatting for session metadata and timestamps.

use super::SessionMetadata;
use std::time::SystemTime;

/// Formats a single session entry for display with ID, working directory,
/// and timestamp.
#[must_use]
pub fn format_session_entry(metadata: &SessionMetadata) -> String {
    let updated = format_timestamp(metadata.updated_at);
    format!(
        "{} | {} | {} msgs | {}",
        metadata.id,
        metadata.working_dir.display(),
        metadata.message_count,
        updated
    )
}

/// Formats a list of session metadata for display.
///
/// Sessions are sorted by most recently updated first. If the list is empty,
/// returns a message indicating no sessions were found.
#[must_use]
pub fn format_session_list(sessions: &[SessionMetadata]) -> String {
    if sessions.is_empty() {
        return "No sessions found.".to_string();
    }

    // Sort by updated_at descending (most recent first)
    let mut sorted = sessions.to_vec();
    sorted.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    let mut output = String::from("Available sessions:\n\n");

    for metadata in &sorted {
        output.push_str(&format_session_entry(metadata));
        output.push('\n');
    }

    output.push_str("\nUse --resume <session-id> or --resume last to resume a session.");
    output
}

/// Formats a `SystemTime` as a human-readable timestamp.
fn format_timestamp(time: SystemTime) -> String {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => {
            let secs = duration.as_secs();
            // Simple UTC timestamp without chrono dependency
            // Format: seconds since epoch (or use chrono if available)
            let days = secs / 86400;
            let remaining = secs % 86400;
            let hours = remaining / 3600;
            let mins = (remaining % 3600) / 60;

            // Calculate approximate date from days since epoch (1970-01-01)
            let (year, month, day) = days_to_ymd(days);

            format!(
                "{:04}-{:02}-{:02} {:02}:{:02} UTC",
                year, month, day, hours, mins
            )
        }
        Err(_) => "unknown".to_string(),
    }
}

/// Converts days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Simplified algorithm for UTC date calculation
    // This is accurate for dates from 1970 onwards
    let mut remaining_days = days;
    let mut year = 1970u64;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    let is_leap = is_leap_year(year);
    let days_in_months: [u64; 12] = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u64;
    for days_in_month in days_in_months {
        if remaining_days < days_in_month {
            break;
        }
        remaining_days -= days_in_month;
        month += 1;
    }

    (year, month, remaining_days + 1)
}

/// Returns true if the given year is a leap year.
const fn is_leap_year(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_is_leap_year() {
        assert!(!is_leap_year(1970)); // Not a leap year
        assert!(is_leap_year(2000)); // Divisible by 400
        assert!(!is_leap_year(1900)); // Divisible by 100 but not 400
        assert!(is_leap_year(2024)); // Divisible by 4 but not 100
    }

    #[test]
    fn test_days_to_ymd_epoch() {
        // Day 0 should be 1970-01-01
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn test_days_to_ymd_known_date() {
        // 2024-01-01 is approximately 19724 days since epoch
        // Let's verify with a known date
        let (year, month, day) = days_to_ymd(365); // One year after epoch
        assert_eq!(year, 1971);
        assert_eq!(month, 1);
        assert_eq!(day, 1);
    }

    #[test]
    fn test_format_timestamp() {
        let time = std::time::UNIX_EPOCH;
        let formatted = format_timestamp(time);
        assert_eq!(formatted, "1970-01-01 00:00 UTC");
    }

    #[test]
    fn test_format_session_entry() {
        let metadata = SessionMetadata {
            id: "test-123".to_string(),
            working_dir: PathBuf::from("/test/project"),
            created_at: std::time::UNIX_EPOCH,
            updated_at: std::time::UNIX_EPOCH,
            message_count: 5,
        };

        let formatted = format_session_entry(&metadata);
        assert!(formatted.contains("test-123"));
        assert!(formatted.contains("/test/project"));
        assert!(formatted.contains("5 msgs"));
    }

    #[test]
    fn test_format_session_list_empty() {
        let sessions: Vec<SessionMetadata> = vec![];
        let formatted = format_session_list(&sessions);
        assert_eq!(formatted, "No sessions found.");
    }

    #[test]
    fn test_format_session_list_with_sessions() {
        let sessions = vec![
            SessionMetadata {
                id: "session-1".to_string(),
                working_dir: PathBuf::from("/project1"),
                created_at: std::time::UNIX_EPOCH,
                updated_at: std::time::UNIX_EPOCH,
                message_count: 3,
            },
            SessionMetadata {
                id: "session-2".to_string(),
                working_dir: PathBuf::from("/project2"),
                created_at: std::time::UNIX_EPOCH,
                updated_at: std::time::UNIX_EPOCH,
                message_count: 7,
            },
        ];

        let formatted = format_session_list(&sessions);
        assert!(formatted.contains("Available sessions:"));
        assert!(formatted.contains("session-1"));
        assert!(formatted.contains("session-2"));
        assert!(formatted.contains("--resume"));
    }
}
