//! Settings as pure data.
//!
//! Loading lives in the binary, the only crate permitted to touch files and
//! environment. This module owns the shape, the defaults, and the validation,
//! so config policy stays unit-testable without a filesystem.

use serde::{Deserialize, Serialize};

/// Every knob canon exposes.
///
/// The defaults are the supported configuration. `.canon.toml` is optional and
/// most repositories should never need one; the settings exist so that a wrong
/// derivation can be silenced without uninstalling the tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// Bytes of convention text injected per write.
    pub injection_budget: usize,
    /// Minimum agreement ratio before a convention is reported at all.
    pub confidence_floor: f32,
    /// Minimum number of files before agreement means anything.
    pub min_files: usize,
    /// Recency half-life in days. Lower makes current style dominate harder.
    pub recency_half_life_days: f32,
    /// Directory names never scanned, matched on a single path segment.
    pub exclude_dirs: Vec<String>,
    /// Convention ids to suppress, for when a derivation is wrong. A trailing
    /// `*` matches by prefix.
    pub suppress: Vec<String>,
    /// One of `off`, `error`, `warn`, `info`, `debug`, `trace`.
    pub log_level: String,
    /// Whether a convention may refuse a write, rather than only advise.
    ///
    /// On. Refusing is the only channel a model cannot decline: context before
    /// the write steers it, a report after the write is read and the turn ends
    /// anyway, and blocking the turn holds it open without producing the edit.
    /// A tool that only advises is a tool that is followed when convenient.
    ///
    /// The safety is in what qualifies, not in whether it is switched on. A
    /// rule may only refuse when every single file agrees — not 51 of 52 — and
    /// when its check cannot be wrong about a legitimate file. That pairing is
    /// what makes a refusal a statement about the repository rather than a
    /// guess about the code.
    ///
    /// Set `enforce = false` in `.canon.toml` to make everything advisory.
    pub enforce: bool,
    /// Reading the branch's ticket, and the thread that superseded it.
    pub context: ContextSettings,
}

/// Everything the ticket-and-thread gather reads.
///
/// Off by default, and off in two independent places. `enabled` is off
/// because this is the only part of canon that reads from a system outside
/// the repository, and that should be a decision someone makes rather than
/// something that starts happening after an update. `slack` is off
/// separately because the Jira half has been measured against 24 pull
/// requests and the thread search has not been scored at all.
///
/// Credentials are deliberately absent. They are read from the environment,
/// so that a token cannot end up in a repository file, in a snapshot, or in
/// `canon check`'s output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContextSettings {
    /// Whether canon reads the branch's ticket at all.
    pub enabled: bool,
    /// Whether the Slack half runs.
    pub slack: bool,
    /// How a ticket key is spelled in a branch name.
    ///
    /// Not a regular expression. canon links no regex engine, so this is
    /// matched against a small documented subset: `[A-Z]`, `[A-Z0-9]`,
    /// `[0-9]`, `\d`, a literal character, each optionally followed by `+`.
    /// A pattern outside the subset leaves the gather silent and is reported
    /// by `canon check`.
    pub key_pattern: String,
    /// Jira custom field ids read alongside the description.
    ///
    /// Empty by default. A custom field id belongs to one Jira instance and
    /// means nothing on another, so the two ids that carry the measured
    /// answers on the instance this was built against go in that
    /// repository's `.canon.toml`, not in this binary.
    pub jira_fields: Vec<String>,
    /// Channels searched before the search widens to the workspace.
    pub slack_channels: Vec<String>,
    /// Channel names never read. A leading or trailing `*` matches by suffix
    /// or prefix.
    ///
    /// The default two are the measured worst failure in the set: a search
    /// that returns the bot mirroring the pull request, and so hands back the
    /// review comment it was trying to predict.
    pub exclude_channels: Vec<String>,
    /// People whose newer statement outranks an older one, beyond the
    /// ticket's reporter and assignee. Empty by default.
    pub deciders: Vec<String>,
    /// Characters of digest, not bytes.
    ///
    /// The host's cap on the field this lands in is stated in characters,
    /// the selection fills characters, and a ticket carrying a currency
    /// symbol or an accent makes the two diverge.
    pub digest_chars: usize,
    /// Minutes a gathered digest is served without going back to the network.
    pub freshness_minutes: u64,
}

impl Default for ContextSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            slack: false,
            key_pattern: "[A-Z][A-Z0-9]+-\\d+".to_string(),
            jira_fields: Vec::new(),
            slack_channels: Vec::new(),
            exclude_channels: vec!["*-bitbucket".to_string(), "*-github".to_string()],
            deciders: Vec::new(),
            digest_chars: 2_000,
            freshness_minutes: 240,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            injection_budget: crate::INJECTION_BUDGET,
            confidence_floor: crate::Confidence::FLOOR,
            min_files: crate::Confidence::MIN_FILES,
            recency_half_life_days: 365.0,
            exclude_dirs: [
                ".git",
                // Agent and editor caches. These are why the fallback walk
                // needs a cap as well as a list: one of them reached 24 GB in a
                // real checkout, and no hand-maintained list stays ahead of
                // whatever tool a team installs next.
                ".claude",
                ".chameleon",
                ".cache",
                ".turbo",
                ".bundle",
                ".mypy_cache",
                ".ruff_cache",
                ".pytest_cache",
                ".tox",
                ".svelte-kit",
                ".nuxt",
                ".output",
                ".vercel",
                ".serverless",
                "Pods",
                "DerivedData",
                "node_modules",
                "target",
                "vendor",
                "dist",
                "build",
                ".next",
                "__pycache__",
                ".venv",
                "venv",
                "coverage",
                ".terraform",
                "tmp",
                ".idea",
            ]
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
            suppress: Vec::new(),
            log_level: "off".to_string(),
            enforce: true,
            context: ContextSettings::default(),
        }
    }
}

/// One variant per invalid field, so a bad value names itself.
#[derive(Debug, PartialEq, Eq)]
pub enum SettingsError {
    /// `injection_budget` exceeds what the host will accept without truncating.
    BudgetTooLarge {
        /// The configured value.
        got: usize,
        /// The ceiling.
        cap: usize,
    },
    /// `confidence_floor` sits outside [`crate::Confidence::FLOOR`] to 1.0.
    FloorOutOfRange {
        /// The configured value, as a percentage, to keep the error comparable.
        got_times_100: i32,
    },
    /// `min_files` is small enough that agreement is coincidence.
    MinFilesTooLow {
        /// The configured value.
        got: usize,
    },
    /// `recency_half_life_days` is zero or negative.
    HalfLifeNotPositive,
    /// `log_level` is not one of the six accepted names.
    UnknownLogLevel {
        /// The configured value.
        got: String,
    },
    /// `context.digest_chars` is zero, or larger than the field it lands in.
    DigestTooLarge {
        /// The configured value.
        got: usize,
        /// The ceiling.
        cap: usize,
    },
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BudgetTooLarge { got, cap } => {
                write!(f, "injection_budget {got} exceeds the host's {cap}-byte hook output cap")
            }
            Self::FloorOutOfRange { got_times_100 } => {
                write!(
                    f,
                    "confidence_floor {got_times_100}% must be between 80% and 100%; below 80% the sample-size bar refuses the convention first and the setting does nothing"
                )
            }
            Self::MinFilesTooLow { got } => write!(
                f,
                "min_files {got} is below 3; smaller samples are coincidence, not convention"
            ),
            Self::HalfLifeNotPositive => write!(f, "recency_half_life_days must be > 0"),
            Self::UnknownLogLevel { got } => write!(
                f,
                "unknown log_level `{got}`; expected off, error, warn, info, debug or trace"
            ),
            Self::DigestTooLarge { got, cap } => write!(
                f,
                "context.digest_chars {got} must be between 1 and {cap}; the host caps this field at {} characters and degrades anything longer to a file preview",
                crate::HOST_CONTEXT_CHAR_CAP
            ),
        }
    }
}

impl std::error::Error for SettingsError {}

impl Settings {
    /// Reject nonsense loudly at load time rather than behaving oddly later.
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.injection_budget > crate::HOOK_OUTPUT_CAP {
            return Err(SettingsError::BudgetTooLarge {
                got: self.injection_budget,
                cap: crate::HOOK_OUTPUT_CAP,
            });
        }
        // From the hard floor, not from 0.5. A lower value passed validation,
        // was printed by `canon check`, and admitted nothing: every path into
        // a `Confidence` refuses agreement under `Confidence::FLOOR` before
        // the configured value is ever consulted.
        if !(crate::Confidence::FLOOR..=1.0).contains(&self.confidence_floor) {
            #[allow(clippy::cast_possible_truncation)]
            return Err(SettingsError::FloorOutOfRange {
                got_times_100: (self.confidence_floor * 100.0) as i32,
            });
        }
        if self.min_files < 3 {
            return Err(SettingsError::MinFilesTooLow { got: self.min_files });
        }
        if self.recency_half_life_days <= 0.0 {
            return Err(SettingsError::HalfLifeNotPositive);
        }
        if !matches!(self.log_level.as_str(), "off" | "error" | "warn" | "info" | "debug" | "trace")
        {
            return Err(SettingsError::UnknownLogLevel { got: self.log_level.clone() });
        }
        if self.context.digest_chars == 0 || self.context.digest_chars > crate::DIGEST_CHAR_CAP {
            return Err(SettingsError::DigestTooLarge {
                got: self.context.digest_chars,
                cap: crate::DIGEST_CHAR_CAP,
            });
        }
        Ok(())
    }

    /// Whether the user has silenced this convention id.
    #[must_use]
    pub fn is_suppressed(&self, convention_id: &str) -> bool {
        self.suppress.iter().any(|s| {
            s == convention_id || s.strip_suffix('*').is_some_and(|p| convention_id.starts_with(p))
        })
    }

    /// Whether a single path segment is excluded from the walk.
    #[must_use]
    pub fn excludes_segment(&self, segment: &str) -> bool {
        self.exclude_dirs.iter().any(|d| d == segment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        Settings::default().validate().expect("defaults must validate");
    }

    #[test]
    fn budget_above_the_host_cap_is_rejected() {
        let s = Settings { injection_budget: 50_000, ..Default::default() };
        assert!(matches!(s.validate(), Err(SettingsError::BudgetTooLarge { .. })));
    }

    #[test]
    fn absurd_floor_is_rejected() {
        let s = Settings { confidence_floor: 0.1, ..Default::default() };
        assert!(matches!(s.validate(), Err(SettingsError::FloorOutOfRange { .. })));
    }

    #[test]
    fn a_floor_the_hard_floor_already_covers_is_rejected_rather_than_ignored() {
        // Issue #20. Half the validated range did nothing at all:
        // `Confidence::derive_counted` refuses anything under `FLOOR` before
        // the configured value is ever read, so 0.5 through 0.79 were
        // accepted, printed by `canon check`, and changed no rule.
        for got in [0.5, 0.6, 0.75, 0.79] {
            let s = Settings { confidence_floor: got, ..Default::default() };
            assert!(
                matches!(s.validate(), Err(SettingsError::FloorOutOfRange { .. })),
                "{got} was accepted and cannot admit anything"
            );
        }
        for got in [crate::Confidence::FLOOR, 0.9, 1.0] {
            let s = Settings { confidence_floor: got, ..Default::default() };
            assert!(s.validate().is_ok(), "{got} is a value that does something");
        }
    }

    #[test]
    fn tiny_sample_gate_is_rejected() {
        let s = Settings { min_files: 1, ..Default::default() };
        assert!(matches!(s.validate(), Err(SettingsError::MinFilesTooLow { .. })));
    }

    #[test]
    fn unknown_log_level_is_rejected() {
        let s = Settings { log_level: "verbose".into(), ..Default::default() };
        assert!(matches!(s.validate(), Err(SettingsError::UnknownLogLevel { .. })));
    }

    #[test]
    fn suppression_supports_exact_and_prefix() {
        let s = Settings {
            suppress: vec!["size.*".into(), "tests.layout.rb".into()],
            ..Default::default()
        };
        assert!(s.is_suppressed("size.app.services.rb"));
        assert!(s.is_suppressed("tests.layout.rb"));
        assert!(!s.is_suppressed("naming.app.services.rb"));
    }

    #[test]
    fn an_unknown_key_is_an_error_not_a_silent_shrug() {
        // A typo in .canon.toml must fail loudly. Silently ignoring it leaves
        // the user believing a setting is active when it never parsed.
        let err = serde_json::from_str::<Settings>(r#"{"injectionBudget": 100}"#);
        assert!(err.is_err());
    }

    #[test]
    fn a_known_key_still_parses_with_the_rest_defaulted() {
        let s: Settings = serde_json::from_str(r#"{"injection_budget": 900}"#).unwrap();
        assert_eq!(s.injection_budget, 900);
        assert_eq!(s.min_files, Settings::default().min_files);
    }

    #[test]
    fn the_context_table_is_off_and_empty_by_default() {
        // The only part of canon that reads from a system outside the
        // repository. It should be a decision someone makes rather than
        // something that starts happening after an update.
        let c = Settings::default().context;
        assert!(!c.enabled, "reading a ticket must not start by itself");
        assert!(!c.slack, "the thread search has never been scored; it stays off separately");
        assert_eq!(c.key_pattern, "[A-Z][A-Z0-9]+-\\d+");
        assert!(c.jira_fields.is_empty(), "custom field ids belong to one Jira instance");
        assert!(c.slack_channels.is_empty());
        assert!(c.deciders.is_empty(), "no name belongs in the source");
        assert_eq!(c.exclude_channels, vec!["*-bitbucket", "*-github"]);
        assert_eq!(c.digest_chars, 2_000);
        assert_eq!(c.freshness_minutes, 240);
    }

    #[test]
    fn a_context_table_parses_without_disturbing_anything_else() {
        let s: Settings = serde_json::from_str(
            r#"{"min_files": 9, "context": {"enabled": true, "digest_chars": 1200}}"#,
        )
        .unwrap();
        assert!(s.context.enabled);
        assert_eq!(s.context.digest_chars, 1_200);
        assert_eq!(s.context.freshness_minutes, 240, "the rest of the table stays default");
        assert_eq!(s.min_files, 9);
    }

    #[test]
    fn an_unknown_key_inside_the_context_table_is_an_error_too() {
        // Same reason the outer table rejects one: a typo means a setting the
        // author thinks is active never parsed.
        assert!(serde_json::from_str::<Settings>(r#"{"context": {"enable": true}}"#).is_err());
    }

    #[test]
    fn a_digest_larger_than_the_field_it_lands_in_is_rejected() {
        // The host caps `additionalContext` at 10,000 characters and degrades
        // anything longer to a file preview, silently. The conventions manifest
        // shares the field, so the digest gets a bound well under it.
        let s = Settings {
            context: ContextSettings { digest_chars: 9_000, ..ContextSettings::default() },
            ..Settings::default()
        };
        assert!(matches!(s.validate(), Err(SettingsError::DigestTooLarge { .. })));
        assert!(s.validate().unwrap_err().to_string().contains("digest_chars"));
    }

    #[test]
    fn a_digest_of_zero_characters_is_rejected_rather_than_silently_saying_nothing() {
        let s = Settings {
            context: ContextSettings { digest_chars: 0, ..ContextSettings::default() },
            ..Settings::default()
        };
        assert!(s.validate().is_err());
    }
}
