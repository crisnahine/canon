//! Logging: off by default, and never to a stream.
//!
//! # The invariant that matters
//!
//! Both standard streams are already spoken for. stdout is the hook protocol
//! channel, so a stray line there is parsed as canon's answer. stderr on
//! `PostToolUse` is fed to the model, so a debug line there arrives inside the
//! user's conversation looking like considered feedback about their code.
//!
//! So logs go to a file or nowhere. There is no verbose flag that changes
//! this, because the flag would eventually be set by someone debugging, and
//! the failure it caused would be baffling.
//!
//! Off by default because a tool that runs before every write and silently
//! grows a file in the background is a tool that fills a disk.

use std::io::Write as _;
use std::sync::OnceLock;

/// Severity, ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Level {
    /// Nothing is written.
    Off,
    /// Something failed and the user may notice.
    Error,
    /// Something was degraded but handled.
    Warn,
    /// Lifecycle events.
    Info,
    /// Enough to reconstruct a decision.
    Debug,
    /// Everything.
    Trace,
}

impl Level {
    fn parse(name: &str) -> Self {
        match name {
            "error" => Self::Error,
            "warn" => Self::Warn,
            "info" => Self::Info,
            "debug" => Self::Debug,
            "trace" => Self::Trace,
            _ => Self::Off,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }
}

struct Sink {
    level: Level,
    path: std::path::PathBuf,
}

static SINK: OnceLock<Sink> = OnceLock::new();

/// Size past which the log is truncated rather than appended to.
///
/// A hook that runs before every write produces a lot of lines. Rotating in
/// place keeps the file bounded without a background task or a second file.
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

/// Point logging at a file, at a level. Call once, early.
///
/// Later calls are ignored rather than an error: a second call means two code
/// paths both tried to configure logging, and the first one won for a reason.
pub(crate) fn init(level_name: &str, path: std::path::PathBuf) {
    let _ = SINK.set(Sink { level: Level::parse(level_name), path });
}

/// Write a line, if the level allows it.
pub(crate) fn log(level: Level, message: &str) {
    let Some(sink) = SINK.get() else { return };
    if sink.level == Level::Off || level > sink.level {
        return;
    }
    write(sink, level, message);
}

fn write(sink: &Sink, level: Level, message: &str) {
    if let Some(parent) = sink.path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let oversized = std::fs::metadata(&sink.path).is_ok_and(|m| m.len() > MAX_LOG_BYTES);
    let opened = std::fs::OpenOptions::new()
        .create(true)
        .append(!oversized)
        .write(true)
        .truncate(oversized)
        .open(&sink.path);
    // Nothing to do if the log cannot be opened. Reporting it would mean
    // writing to a stream the protocol owns, which is the exact thing this
    // module exists to prevent.
    if let Ok(mut file) = opened {
        let _ = writeln!(file, "[{}] {message}", level.label());
    }
}

/// Log an error.
pub(crate) fn error(message: &str) {
    log(Level::Error, message);
}

/// Log a warning.
pub(crate) fn warn(message: &str) {
    log(Level::Warn, message);
}

/// Log a lifecycle event.
pub(crate) fn info(message: &str) {
    log(Level::Info, message);
}

/// Log a decision detail.
pub(crate) fn debug(message: &str) {
    log(Level::Debug, message);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_ordered_from_quiet_to_loud() {
        assert!(Level::Off < Level::Error);
        assert!(Level::Error < Level::Warn);
        assert!(Level::Warn < Level::Info);
        assert!(Level::Info < Level::Debug);
        assert!(Level::Debug < Level::Trace);
    }

    #[test]
    fn an_unknown_level_name_means_off_rather_than_verbose() {
        // Failing open into silence, not into noise.
        assert_eq!(Level::parse("verbose"), Level::Off);
        assert_eq!(Level::parse(""), Level::Off);
        assert_eq!(Level::parse("OFF"), Level::Off);
    }

    #[test]
    fn every_level_has_a_label() {
        for level in
            [Level::Off, Level::Error, Level::Warn, Level::Info, Level::Debug, Level::Trace]
        {
            assert!(!level.label().is_empty());
        }
    }

    #[test]
    fn writing_before_init_does_nothing_and_does_not_panic() {
        // Every log call in a hook runs before init on at least one path.
        error("no sink configured yet");
        warn("still nothing");
        debug("still nothing");
    }

    #[test]
    fn a_configured_sink_writes_the_level_and_the_message() {
        let path = std::env::temp_dir().join("canon-log-test/out.log");
        let _ = std::fs::remove_file(&path);
        let sink = Sink { level: Level::Debug, path: path.clone() };
        write(&sink, Level::Warn, "degraded to defaults");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[WARN] degraded to defaults"), "got {text}");
    }

    #[test]
    fn an_unwritable_path_is_survived_silently() {
        let sink = Sink { level: Level::Debug, path: "/nonexistent-root/x/y.log".into() };
        write(&sink, Level::Error, "message");
    }
}
