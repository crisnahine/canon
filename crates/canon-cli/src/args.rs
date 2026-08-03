//! Argument parsing, by hand.
//!
//! Deliberately not a dependency. The surface is eight subcommands and three
//! flags, it is fully specified by the hook configuration this binary ships
//! with, and it runs before every write in an editor. Hand-rolling costs
//! eighty lines and removes a dependency tree from a binary that intends to
//! still build in a decade.
//!
//! The rule that keeps it honest: unknown input is an error, never a guess. A
//! typo in `hooks.json` must be visible, not silently treated as the default
//! subcommand.

/// What the binary was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    /// `SessionStart`: refresh the snapshot and state what was found.
    SessionStart,
    /// `SubagentStart`: state the same thing to a worker with an empty context.
    SubagentStart,
    /// `PreToolUse`: the conventions for the file about to be written.
    Inject,
    /// `PostToolUse`: compare what was written against them.
    Verify,
    /// `Stop`: cross-file duplication over everything touched this turn.
    Reconcile,
    /// Print live configuration, capability table, and snapshot state.
    Check,
    /// Rebuild the snapshot now.
    Index {
        /// Discard the existing snapshot first.
        rebuild: bool,
    },
    /// Show every convention for a path, or one by id, with its evidence.
    Explain {
        /// A path prefix, or `None` when `id` is given.
        path: Option<String>,
        /// A convention id.
        id: Option<String>,
    },
    /// Usage.
    Help,
    /// Version.
    Version,
}

/// Why the arguments could not be understood.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ArgError {
    /// No subcommand at all.
    Missing,
    /// A subcommand canon does not have.
    UnknownCommand(String),
    /// A flag the subcommand does not take.
    UnknownFlag(String),
    /// A flag that needs a value and did not get one.
    MissingValue(String),
}

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "expected a subcommand; try `canon help`"),
            Self::UnknownCommand(c) => write!(f, "unknown subcommand `{c}`; try `canon help`"),
            Self::UnknownFlag(flag) => write!(f, "unknown flag `{flag}`"),
            Self::MissingValue(flag) => write!(f, "`{flag}` needs a value"),
        }
    }
}

impl std::error::Error for ArgError {}

/// Parse arguments, excluding the program name.
pub(crate) fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command, ArgError> {
    let mut args = args.into_iter().peekable();
    let Some(first) = args.next() else { return Err(ArgError::Missing) };

    match first.as_str() {
        "session-start" => Ok(Command::SessionStart),
        "subagent-start" => Ok(Command::SubagentStart),
        "inject" => Ok(Command::Inject),
        "verify" => Ok(Command::Verify),
        "reconcile" => Ok(Command::Reconcile),
        "check" => Ok(Command::Check),
        "help" | "--help" | "-h" => Ok(Command::Help),
        "version" | "--version" | "-V" => Ok(Command::Version),
        "index" => {
            let mut rebuild = false;
            for arg in args {
                match arg.as_str() {
                    "--rebuild" => rebuild = true,
                    other => return Err(ArgError::UnknownFlag(other.to_string())),
                }
            }
            Ok(Command::Index { rebuild })
        }
        "explain" => {
            let mut path = None;
            let mut id = None;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--id" => {
                        id = Some(args.next().ok_or(ArgError::MissingValue("--id".into()))?);
                    }
                    other if other.starts_with('-') => {
                        return Err(ArgError::UnknownFlag(other.to_string()));
                    }
                    other => path = Some(other.to_string()),
                }
            }
            Ok(Command::Explain { path, id })
        }
        other => Err(ArgError::UnknownCommand(other.to_string())),
    }
}

/// Usage text.
pub(crate) const USAGE: &str = "\
canon — your repository already documents its own conventions

Hook commands (read a JSON payload on stdin; the plugin wires these for you):
  session-start     refresh the snapshot and state what was found
  subagent-start    state the same thing to a subagent with an empty context
  inject            conventions for the file about to be written
  verify            compare what was written against them
  reconcile         cross-file duplication over everything touched this turn

Commands for people:
  check             live configuration, language table, snapshot state
  index [--rebuild] rebuild the snapshot now
  explain <path>    every convention for a path, with its evidence
  explain --id <id> one convention, with its evidence
  help, version
";

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn parse_str(s: &str) -> Result<Command, ArgError> {
        parse(s.split_whitespace().map(String::from))
    }

    #[test]
    fn every_hook_subcommand_parses() {
        assert_eq!(parse_str("session-start").unwrap(), Command::SessionStart);
        assert_eq!(parse_str("subagent-start").unwrap(), Command::SubagentStart);
        assert_eq!(parse_str("inject").unwrap(), Command::Inject);
        assert_eq!(parse_str("verify").unwrap(), Command::Verify);
        assert_eq!(parse_str("reconcile").unwrap(), Command::Reconcile);
    }

    #[test]
    fn no_arguments_is_an_error_not_a_default_command() {
        assert_eq!(parse_str(""), Err(ArgError::Missing));
    }

    #[test]
    fn an_unknown_subcommand_is_reported_rather_than_guessed() {
        // A typo in hooks.json must be visible.
        assert_eq!(
            parse_str("inject-all"),
            Err(ArgError::UnknownCommand("inject-all".to_string()))
        );
    }

    #[test]
    fn an_unknown_flag_is_reported() {
        assert_eq!(parse_str("index --force"), Err(ArgError::UnknownFlag("--force".to_string())));
    }

    #[test]
    fn index_takes_an_optional_rebuild_flag() {
        assert_eq!(parse_str("index").unwrap(), Command::Index { rebuild: false });
        assert_eq!(parse_str("index --rebuild").unwrap(), Command::Index { rebuild: true });
    }

    #[test]
    fn explain_accepts_a_path_or_an_id() {
        assert_eq!(
            parse_str("explain app/services/").unwrap(),
            Command::Explain { path: Some("app/services/".into()), id: None }
        );
        assert_eq!(
            parse_str("explain --id shape.base.app.rb").unwrap(),
            Command::Explain { path: None, id: Some("shape.base.app.rb".into()) }
        );
    }

    #[test]
    fn a_flag_missing_its_value_is_an_error() {
        assert_eq!(parse_str("explain --id"), Err(ArgError::MissingValue("--id".to_string())));
    }

    #[test]
    fn help_and_version_have_the_conventional_aliases() {
        for form in ["help", "--help", "-h"] {
            assert_eq!(parse_str(form).unwrap(), Command::Help);
        }
        for form in ["version", "--version", "-V"] {
            assert_eq!(parse_str(form).unwrap(), Command::Version);
        }
    }

    #[test]
    fn the_usage_text_lists_every_subcommand_that_parses() {
        for name in [
            "session-start",
            "subagent-start",
            "inject",
            "verify",
            "reconcile",
            "check",
            "index",
            "explain",
        ] {
            assert!(USAGE.contains(name), "usage does not mention `{name}`");
        }
    }
}
