//! Time-format detection & extraction.
//!
//! One module per timestamp family (ISO-8601, `YYYY/MM/DD`, BSD syslog, Apache
//! CLF, epoch seconds/millis, logcat threadtime, glog). Each implements
//! [`TimeFormat`] and registers itself in [`TIME_FORMATS`]; [`TimeDetector`]
//! samples a few leading lines and picks the dominant family, then uses that
//! single fast path for the rest of the file. All values are normalized to
//! Unix epoch **milliseconds** (UTC).
//!
//! Adding a new timestamp family = add one `foo.rs` (struct + impl) and one
//! entry in [`TIME_FORMATS`]. Nothing else needs to change.

mod apache;
mod epoch;
mod glog;
mod iso;
mod logcat_threadtime;
mod slash;
mod syslog;

use std::ops::Range;

use chrono::{DateTime, NaiveDateTime};

pub use apache::Apache;
pub use epoch::Epoch;
pub use glog::Glog;
pub use iso::Iso;
pub use logcat_threadtime::LogcatThreadtime;
pub use slash::Slash;
pub use syslog::Syslog;

/// How far into a line we look for a timestamp. Timestamps virtually always
/// live at (or near) the start of a log line; capping the search window keeps
/// the per-line cost tiny on 50MB+ files.
pub(crate) const SEARCH_WINDOW: usize = 96;

/// A pluggable timestamp family: a recognizer ([`TimeFormat::matches`]) and an
/// extractor ([`TimeFormat::extract`]) that returns epoch millis + byte span.
pub trait TimeFormat: Send + Sync {
    /// Human-readable family name (used in tests and diagnostics).
    fn name(&self) -> &'static str;

    /// Cheap recognizer: does this line carry this timestamp family?
    fn matches(&self, line: &str) -> bool;

    /// Extract (epoch millis, byte span) from a line, if this family applies.
    fn extract(&self, line: &str) -> Option<(i64, Range<usize>)>;
}

/// All registered time formats, in detection priority order (most common first).
pub static TIME_FORMATS: &[&'static dyn TimeFormat] = &[
    &Iso,
    &Slash,
    &Syslog,
    &Apache,
    &Epoch,
    &LogcatThreadtime,
    &Glog,
];

/// Trim a line to the timestamp search window (zero-cost when already short).
pub(crate) fn window(line: &str) -> &str {
    if line.len() > SEARCH_WINDOW {
        line.get(..SEARCH_WINDOW).unwrap_or(line)
    } else {
        line
    }
}

/// Samples lines and picks the dominant timestamp family.
pub struct TimeDetector;

impl TimeDetector {
    /// Detect among all registered families. Returns `None` for timeless text.
    pub fn detect<S: AsRef<str>>(sample: impl Iterator<Item = S>) -> Option<&'static dyn TimeFormat> {
        Self::detect_among(sample, TIME_FORMATS)
    }

    /// Detect among a restricted set of families (used by log formats that
    /// only allow a specific timestamp shape, e.g. RFC 5424 → ISO only).
    pub fn detect_among<S: AsRef<str>>(
        sample: impl Iterator<Item = S>,
        candidates: &'static [&'static dyn TimeFormat],
    ) -> Option<&'static dyn TimeFormat> {
        let lines: Vec<S> = sample.take(512).collect();
        if lines.is_empty() {
            return None;
        }
        let mut best: Option<(&'static dyn TimeFormat, usize)> = None;
        for &fmt in candidates {
            let hits = lines
                .iter()
                .filter(|l| fmt.extract(l.as_ref()).is_some())
                .count();
            if best.map_or(true, |(_, b)| hits > b) {
                best = Some((fmt, hits));
            }
        }
        // Require a meaningful hit rate so we don't latch onto noise.
        let (fmt, hits) = best?;
        let needed = (lines.len() / 4).max(2).min(lines.len());
        if hits >= needed {
            Some(fmt)
        } else {
            None
        }
    }
}

/// Render epoch millis as a compact UTC timestamp for display / MCP output.
pub fn format_ms(ms: i64) -> String {
    match chrono::DateTime::from_timestamp_millis(ms) {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
        None => format!("{ms}"),
    }
}

/// Parse a user-supplied time value (RFC3339, "YYYY-MM-DD HH:MM:SS",
/// "YYYY-MM-DD", or epoch millis) into epoch millis. Used by the MCP server
/// and by field-based timestamps (e.g. JSON `time` fields).
pub fn parse_time_param(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Ok(n) = s.parse::<i64>() {
        return Some(if s.len() <= 11 { n * 1000 } else { n });
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    let normalized = s.replace('T', " ");
    for fmt in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(&normalized, fmt) {
            return Some(n.and_utc().timestamp_millis());
        }
    }
    // Date-only form parses as NaiveDate.
    if let Ok(d) = chrono::NaiveDate::parse_from_str(&normalized, "%Y-%m-%d") {
        return d.and_hms_milli_opt(0, 0, 0, 0).map(|n| n.and_utc().timestamp_millis());
    }
    None
}


#[cfg(test)]
mod tests {
    use super::*;

    fn detect_name(line: &str) -> Option<&'static str> {
        TimeDetector::detect(std::iter::once(line)).map(|f| f.name())
    }

    #[test]
    fn detector_picks_intended_format() {
        assert_eq!(detect_name("2026-07-19T10:15:30.123Z INFO hello"), Some("ISO-8601"));
        assert_eq!(detect_name("2026/07/19 10:15:30 INFO slash"), Some("YYYY/MM/DD"));
        assert_eq!(detect_name("Jan  5 03:22:11 host sshd[1]: hi"), Some("BSD syslog"));
        assert_eq!(
            detect_name("127.0.0.1 - - [10/Oct/2024:13:55:36 +0000] \"GET /\""),
            Some("Apache CLF")
        );
        assert_eq!(detect_name("1784158530123 | INFO | event"), Some("epoch"));
        assert_eq!(detect_name("07-15 22:00:01.123  1234  5678 I Tag: hello"), Some("logcat threadtime"));
        assert_eq!(detect_name("I0715 22:00:01.123456 12345 server.cc:42] started"), Some("glog"));
    }

    #[test]
    fn rejects_timeless_text() {
        assert_eq!(detect_name("just a plain line of text, no time here"), None);
    }

    #[test]
    fn detect_among_restricts_candidates() {
        // A logcat-threadtime line should NOT be detected when only ISO is allowed.
        let allowed: &'static [&'static dyn TimeFormat] = &[&Iso];
        let result = TimeDetector::detect_among(
            std::iter::once("07-15 22:00:01.123  1234  5678 I Tag: hello"),
            allowed,
        );
        assert!(result.is_none());
        // But ISO still detects from the same candidate set.
        let result = TimeDetector::detect_among(
            std::iter::once("2026-07-19T10:15:30.123Z INFO hello"),
            allowed,
        );
        assert_eq!(result.map(|f| f.name()), Some("ISO-8601"));
    }

    #[test]
    fn parses_time_params() {
        assert_eq!(parse_time_param("1784158530123"), Some(1784158530123));
        assert_eq!(
            parse_time_param("2026-07-19T10:15:30Z"),
            parse_time_param("2026-07-19 10:15:30")
        );
        assert!(parse_time_param("not a time").is_none());
    }
}
