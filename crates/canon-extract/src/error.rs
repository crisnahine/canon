//! Extraction failures, kept distinct so callers can decide to degrade or skip.

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, ExtractError>;

/// Why extraction did not produce facts.
///
/// The variants are separate because the correct response differs. A grammar
/// that will not load is a build problem and should be reported once, loudly. A
/// file that will not parse is normal in a working tree and should be skipped
/// silently. A language with no grammar wired is a capability gap the caller
/// may want to report to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractError {
    /// The grammar refused to load, which means the binary and the grammar
    /// crate disagree on ABI version. Not recoverable at runtime.
    Grammar {
        /// Language whose grammar failed.
        language: &'static str,
    },
    /// The parser returned no tree at all. Distinct from a tree containing
    /// error nodes, which is normal and is not an error here.
    Parse {
        /// Repository-relative path.
        path: String,
    },
    /// No grammar is wired for this language yet.
    Unsupported {
        /// Language name.
        language: &'static str,
    },
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Grammar { language } => {
                write!(
                    f,
                    "the {language} grammar failed to load; grammar and tree-sitter ABI disagree"
                )
            }
            Self::Parse { path } => write!(f, "{path}: parser returned no tree"),
            Self::Unsupported { language } => write!(f, "no grammar wired for {language}"),
        }
    }
}

impl std::error::Error for ExtractError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_variant_names_itself_in_its_message() {
        assert!(ExtractError::Grammar { language: "ruby" }.to_string().contains("ruby"));
        assert!(ExtractError::Parse { path: "a/b.rb".into() }.to_string().contains("a/b.rb"));
        assert!(ExtractError::Unsupported { language: "Vue SFC" }.to_string().contains("Vue SFC"));
    }
}
