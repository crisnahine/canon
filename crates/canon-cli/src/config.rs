//! Loading settings, layered and validated.
//!
//! Order, lowest precedence first: built-in defaults, then `.canon.toml` at the
//! repository root, then `CANON_*` environment variables.
//!
//! # Why an unknown key is fatal here and ignored in the hook payload
//!
//! They are opposite problems. A typo in `.canon.toml` is a user mistake they
//! want told about, because the setting they think is active never parsed. An
//! unknown field in the host's payload is the host evolving, and rejecting it
//! would take canon offline the day a new version ships.
//!
//! # Why invalid config still runs
//!
//! A hook cannot show an error. Exit code 2 blocks the tool call, stderr on
//! `PostToolUse` is fed to the model as if it were feedback, and stdout is the
//! protocol channel. There is no path from a hook to the user's eyes. So on
//! the hook path an invalid config degrades to defaults and is logged, and
//! `canon check` is where it fails loudly, because that is a command a human
//! ran and is waiting on.

use std::path::Path;

use canon_core::Settings;

/// Why loading failed.
#[derive(Debug)]
pub(crate) enum ConfigError {
    /// The file exists but is not valid TOML, or has a key canon does not know.
    Parse {
        /// Where the file was.
        path: String,
        /// What the parser said.
        detail: String,
    },
    /// The file parsed but a value is out of range.
    Invalid(canon_core::SettingsError),
    /// An environment variable could not be interpreted as its field's type.
    Env {
        /// Variable name.
        key: String,
        /// The value that would not parse.
        value: String,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse { path, detail } => write!(f, "{path}: {detail}"),
            Self::Invalid(e) => write!(f, "{e}"),
            Self::Env { key, value } => write!(f, "{key}=`{value}` is not a valid value"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// File name looked for at the repository root.
pub(crate) const CONFIG_FILE: &str = ".canon.toml";

/// Load settings for `root`, failing on anything wrong.
///
/// Used by `canon check`, where a human is waiting and wants the truth.
pub(crate) fn load(root: &Path) -> Result<Settings, ConfigError> {
    let mut settings = load_file(root)?;
    apply_env(&mut settings, &std::env::vars().collect::<Vec<_>>())?;
    settings.validate().map_err(ConfigError::Invalid)?;
    Ok(settings)
}

/// Load settings, falling back to defaults on any problem.
///
/// Used by every hook. Returns the reason alongside, so the caller can log it
/// to a file rather than pretending nothing happened.
#[must_use]
pub(crate) fn load_or_default(root: &Path) -> (Settings, Option<ConfigError>) {
    match load(root) {
        Ok(settings) => (settings, None),
        Err(e) => (Settings::default(), Some(e)),
    }
}

fn load_file(root: &Path) -> Result<Settings, ConfigError> {
    let path = root.join(CONFIG_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Settings::default());
    };
    toml::from_str(&text).map_err(|e| ConfigError::Parse {
        path: path.display().to_string(),
        detail: e.message().to_string(),
    })
}

/// Apply `CANON_*` overrides.
///
/// Takes the variables rather than reading the environment so the precedence
/// rules are testable without mutating process state, which is a data race in
/// a test binary that runs threads.
pub(crate) fn apply_env(
    settings: &mut Settings,
    vars: &[(String, String)],
) -> Result<(), ConfigError> {
    for (key, value) in vars {
        let parsed = match key.as_str() {
            "CANON_INJECTION_BUDGET" => {
                value.parse().map(|v| settings.injection_budget = v).map_err(|_| ())
            }
            "CANON_CONFIDENCE_FLOOR" => {
                value.parse().map(|v| settings.confidence_floor = v).map_err(|_| ())
            }
            "CANON_MIN_FILES" => value.parse().map(|v| settings.min_files = v).map_err(|_| ()),
            "CANON_RECENCY_HALF_LIFE_DAYS" => {
                value.parse().map(|v| settings.recency_half_life_days = v).map_err(|_| ())
            }
            "CANON_ENFORCE" => {
                match value.as_str() {
                    "1" | "true" | "yes" | "on" => settings.enforce = true,
                    "0" | "false" | "no" | "off" => settings.enforce = false,
                    _ => return Err(ConfigError::Env { key: key.clone(), value: value.clone() }),
                }
                Ok(())
            }
            "CANON_LOG" => {
                settings.log_level.clone_from(value);
                Ok(())
            }
            "CANON_SUPPRESS" => {
                settings.suppress = value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                Ok(())
            }
            _ => continue,
        };
        if parsed.is_err() {
            return Err(ConfigError::Env { key: key.clone(), value: value.clone() });
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn dir(name: &str, contents: Option<&str>) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("canon-config-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        if let Some(text) = contents {
            std::fs::write(root.join(CONFIG_FILE), text).unwrap();
        }
        root
    }

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect()
    }

    #[test]
    fn a_repository_with_no_config_file_gets_the_defaults() {
        let root = dir("none", None);
        assert_eq!(load(&root).unwrap(), Settings::default());
    }

    #[test]
    fn a_config_file_overrides_one_field_and_leaves_the_rest() {
        let root = dir("partial", Some("min_files = 9\n"));
        let settings = load(&root).unwrap();
        assert_eq!(settings.min_files, 9);
        assert_eq!(settings.injection_budget, Settings::default().injection_budget);
    }

    #[test]
    fn an_unknown_key_is_an_error_rather_than_a_silent_shrug() {
        let root = dir("typo", Some("min_fils = 9\n"));
        let err = load(&root).unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }), "got {err:?}");
    }

    #[test]
    fn a_value_out_of_range_is_rejected_with_the_field_named() {
        let root = dir("range", Some("confidence_floor = 0.1\n"));
        let err = load(&root).unwrap_err();
        assert!(err.to_string().contains("confidence_floor"), "got {err}");
    }

    #[test]
    fn malformed_toml_is_reported_with_its_path() {
        let root = dir("broken", Some("min_files = = 9\n"));
        let err = load(&root).unwrap_err();
        assert!(err.to_string().contains(CONFIG_FILE), "got {err}");
    }

    #[test]
    fn the_environment_outranks_the_file() {
        let mut settings = Settings { min_files: 9, ..Settings::default() };
        apply_env(&mut settings, &vars(&[("CANON_MIN_FILES", "12")])).unwrap();
        assert_eq!(settings.min_files, 12);
    }

    #[test]
    fn an_unparseable_environment_value_is_an_error_not_a_silent_default() {
        let mut settings = Settings::default();
        let err = apply_env(&mut settings, &vars(&[("CANON_MIN_FILES", "lots")])).unwrap_err();
        assert!(err.to_string().contains("CANON_MIN_FILES"), "got {err}");
    }

    #[test]
    fn suppression_is_a_comma_separated_list() {
        let mut settings = Settings::default();
        apply_env(&mut settings, &vars(&[("CANON_SUPPRESS", "naming.*, shape.base.app.rb ,")]))
            .unwrap();
        assert_eq!(settings.suppress, vec!["naming.*", "shape.base.app.rb"]);
    }

    #[test]
    fn enforcement_can_be_turned_off_from_the_environment() {
        // On by default: advising is the channel a model may decline, and a
        // tool that only advises is followed when convenient. What keeps that
        // safe is which rules qualify, not whether the switch is thrown.
        let mut settings = Settings::default();
        assert!(settings.enforce, "on by default");
        apply_env(&mut settings, &vars(&[("CANON_ENFORCE", "off")])).unwrap();
        assert!(!settings.enforce);
        apply_env(&mut settings, &vars(&[("CANON_ENFORCE", "1")])).unwrap();
        assert!(settings.enforce);
    }

    #[test]
    fn an_unreadable_enforcement_value_is_an_error_not_a_silent_no() {
        // Silently reading `maybe` as off would leave someone believing
        // enforcement is on when it never was.
        let mut settings = Settings::default();
        assert!(apply_env(&mut settings, &vars(&[("CANON_ENFORCE", "maybe")])).is_err());
    }

    #[test]
    fn unrelated_environment_variables_are_ignored() {
        let mut settings = Settings::default();
        apply_env(&mut settings, &vars(&[("PATH", "/usr/bin"), ("CANONICAL", "x")])).unwrap();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn a_broken_config_degrades_to_defaults_on_the_hook_path() {
        // A hook has no way to show an error, so it must keep working.
        let root = dir("degrade", Some("min_fils = 9\n"));
        let (settings, err) = load_or_default(&root);
        assert_eq!(settings, Settings::default());
        assert!(err.is_some(), "the reason must survive for the log");
    }
}
