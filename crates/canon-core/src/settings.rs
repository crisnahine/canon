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
    /// Off by default. Refusing is the only channel a model cannot decline,
    /// and that is exactly why the first release of it is opt-in: a rule that
    /// refuses correct code is worse than no rule, and the failure lands on
    /// someone mid-edit who cannot tell a real rule from a bad inference.
    ///
    /// Even when on, only a rule with total agreement and a check that cannot
    /// be wrong about a legitimate file will ever refuse.
    pub enforce: bool,
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
            enforce: false,
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
    /// `confidence_floor` sits outside 0.5 to 1.0.
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
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BudgetTooLarge { got, cap } => {
                write!(f, "injection_budget {got} exceeds the host's {cap}-byte hook output cap")
            }
            Self::FloorOutOfRange { got_times_100 } => {
                write!(f, "confidence_floor {got_times_100}% must be between 50% and 100%")
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
        if !(0.5..=1.0).contains(&self.confidence_floor) {
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
}
