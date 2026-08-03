//! What a derived rule is, where it applies, and how far it is allowed to go.

use serde::{Deserialize, Serialize};

use crate::{Confidence, Settings};

/// Which files a convention speaks for.
///
/// Directory scopes match by path prefix, not by exact parent, because
/// conventions are derived at every ancestor level. A file written into a
/// directory that did not exist at index time still inherits the rules of the
/// nearest ancestor that has any, which is the common case when a developer
/// adds a new domain folder.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scope {
    /// Every file in the repository.
    Repo,
    /// Every file with this extension, anywhere.
    Ext(String),
    /// Every file under this directory prefix.
    Dir(String),
    /// Every file with this extension under this directory prefix.
    DirExt(String, String),
}

impl Scope {
    /// Whether this scope speaks for `rel`, a repository-relative path with
    /// forward slashes.
    #[must_use]
    pub fn matches(&self, rel: &str) -> bool {
        match self {
            Self::Repo => true,
            Self::Ext(ext) => has_ext(rel, ext),
            Self::Dir(dir) => under(rel, dir),
            Self::DirExt(dir, ext) => under(rel, dir) && has_ext(rel, ext),
        }
    }

    /// How narrowly this scope speaks, for ranking two conventions that both
    /// match the same file.
    ///
    /// Deeper directories win, and at equal depth an extension-qualified scope
    /// wins over a bare one. A rule derived from the twelve files beside the
    /// one being written describes it better than a rule derived from four
    /// thousand files across the repository.
    #[must_use]
    pub fn specificity(&self) -> usize {
        let depth = match self {
            Self::Repo | Self::Ext(_) => 0,
            Self::Dir(d) | Self::DirExt(d, _) => d.split('/').filter(|s| !s.is_empty()).count() * 2,
        };
        let qualified = usize::from(matches!(self, Self::Ext(_) | Self::DirExt(..)));
        depth + qualified
    }

    /// Human-facing description of the file set, for the header of an injected
    /// block.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Repo => "this repository".to_string(),
            Self::Ext(ext) => format!("**/*.{ext}"),
            Self::Dir(dir) => format!("{dir}/**"),
            Self::DirExt(dir, ext) => format!("{dir}/**/*.{ext}"),
        }
    }
}

fn has_ext(rel: &str, ext: &str) -> bool {
    rel.rsplit_once('.').is_some_and(|(_, e)| e == ext)
}

fn under(rel: &str, dir: &str) -> bool {
    if dir.is_empty() {
        return true;
    }
    rel.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/'))
}

/// How far a convention may go when the code disagrees with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Enforcement {
    /// Stated in the injected block; disagreement is reported, never refused.
    ///
    /// The default, and the setting for everything derived by counting. A
    /// heuristic that denies a tool call is worse than no heuristic, because
    /// the developer cannot tell a real rule from a bad inference at the moment
    /// they are interrupted.
    Advisory,
    /// May refuse a write.
    ///
    /// Requires total agreement *and* a check that cannot produce a false
    /// positive. Both halves matter: total agreement over a mis-specified check
    /// still blocks correct code.
    Blocking,
}

/// One file that supports a convention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// Repository-relative path.
    pub rel: String,
    /// Line the supporting construct starts on, when the fact came from a
    /// parser. Zero for facts derived from the path alone.
    pub line: usize,
}

/// A claim about the shape of the code, with the evidence that produced it.
///
/// Never a claim about correctness. A convention can say that the other
/// forty-seven service objects expose a single `call`; it cannot say that
/// doing so is right, and it has no opinion on whether the code works.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Convention {
    /// Stable identifier, e.g. `shape.public-arity.app.services.rb`.
    ///
    /// Stable across runs so a user can suppress one by id in `.canon.toml`
    /// and have the suppression survive re-indexing.
    pub id: String,
    /// The rule, phrased as a statement about this repository.
    pub statement: String,
    /// Which files it speaks for.
    pub scope: Scope,
    /// Strength of agreement.
    pub confidence: Confidence,
    /// Files that agree.
    pub agreeing: usize,
    /// Files considered.
    pub total: usize,
    /// The best live example, when one exists.
    ///
    /// Most recently modified among the agreeing files rather than the first
    /// found: old files encode conventions the team has already moved off, and
    /// pointing a developer at one teaches the wrong shape.
    pub exemplar: Option<String>,
    /// A sample of supporting files, capped at render time.
    pub evidence: Vec<Evidence>,
    /// How far it may go.
    pub enforcement: Enforcement,
}

/// How far a rule of this kind may go when the code disagrees.
///
/// Advisory is the default and stays the default. Refusing a write on a
/// heuristic is worse than not having the heuristic, because the developer
/// cannot tell a real rule from a bad inference at the moment they are
/// interrupted.
///
/// Two conditions have to hold together before a rule may refuse anything.
///
/// The repository has to agree *totally*. Not 51 of 52: every single file. One
/// counterexample already in the tree means the rule has an exception nobody
/// wrote down, and blocking a write that matches an existing file is the
/// fastest way to get a tool uninstalled.
///
/// And the check has to be one that cannot be wrong about a legitimate file.
/// Counting a type's public methods, reading its base, and reading the name of
/// its single public method are all exact. "Files here call `X`" is not: a new
/// file may legitimately not need that collaborator yet.
///
/// Decided from the convention itself rather than read off the snapshot, so a
/// `.canon.toml` written in response to a refusal takes effect on the next
/// write rather than on the next session.
#[must_use]
pub fn enforcement_for(id: &str, confidence: Confidence, settings: &Settings) -> Enforcement {
    /// Checks that cannot be wrong about a legitimate file.
    ///
    /// `naming` is a string comparison on the path, with no parsing and no
    /// question about which type in the file is the subject. It only qualifies
    /// because a naming rule is withheld unless the sample contains enough
    /// distinct names to have witnessed the style; see `canon-derive::naming`.
    /// The shape checks read one resolved subject, which is only true since
    /// they stopped judging every declared type.
    const EXACT: &[&str] = &["naming", "shape.public-arity", "shape.entrypoint", "shape.base"];
    // A rule assembled from other rules is never enforced. Its evidence is
    // real, but it generalises to sibling directories that never voted, and a
    // directory with no files yet is exactly where a refusal would be wrong.
    if id.ends_with(ROLLUP_SUFFIX) {
        return Enforcement::Advisory;
    }
    if settings.enforce
        && confidence.is_blocking_grade()
        && EXACT.iter().any(|kind| id.starts_with(kind))
    {
        Enforcement::Blocking
    } else {
        Enforcement::Advisory
    }
}

/// Marks an id as having been assembled from the rules of child directories.
pub const ROLLUP_SUFFIX: &str = ".rollup";

impl Convention {
    /// Whether this convention may refuse a write *under these settings*.
    ///
    /// The stored [`Convention::enforcement`] records the answer at the moment
    /// the snapshot was built. This recomputes it, which is what makes
    /// `enforce = false` and a fresh `suppress` entry work inside the session
    /// that hit the refusal instead of only after the next rebuild.
    #[must_use]
    pub fn enforcement_now(&self, settings: &Settings) -> Enforcement {
        if settings.is_suppressed(&self.id) {
            return Enforcement::Advisory;
        }
        enforcement_for(&self.id, self.confidence, settings)
    }

    /// Render as one line of an injected block: the statement, the count, and
    /// the confidence.
    #[must_use]
    pub fn render_line(&self) -> String {
        format!(
            "- {}. ({}/{}, {})",
            self.statement.trim_end_matches('.'),
            self.agreeing,
            self.total,
            self.confidence.render()
        )
    }

    /// Sort key for choosing what to inject when the budget is tight.
    ///
    /// Specificity first, then confidence, then evidence volume. Specificity
    /// leads because a narrow rule is more actionable than a strong one: every
    /// repository agrees that files end in a newline, and saying so wastes the
    /// budget.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn rank(&self) -> (usize, u32, usize) {
        (self.scope.specificity(), (self.confidence.value() * 1000.0) as u32, self.agreeing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(scope: Scope) -> Convention {
        Convention {
            id: "t".into(),
            statement: "Types expose exactly 1 public method".into(),
            scope,
            confidence: Confidence::derive(47, 52).unwrap(),
            agreeing: 47,
            total: 52,
            exemplar: None,
            evidence: vec![],
            enforcement: Enforcement::Advisory,
        }
    }

    #[test]
    fn a_directory_scope_matches_files_nested_below_it() {
        let s = Scope::Dir("app/services".into());
        assert!(s.matches("app/services/create.rb"));
        assert!(s.matches("app/services/enrolments/create.rb"), "new subfolder inherits");
        assert!(!s.matches("app/models/user.rb"));
    }

    #[test]
    fn a_directory_scope_does_not_match_a_sibling_with_a_shared_prefix() {
        let s = Scope::Dir("app/service".into());
        assert!(!s.matches("app/services/create.rb"), "`service` must not capture `services`");
    }

    #[test]
    fn an_extension_scope_matches_the_final_segment_only() {
        let s = Scope::Ext("rb".into());
        assert!(s.matches("app/services/create.rb"));
        assert!(!s.matches("app/services/create.rb.erb"));
        assert!(!s.matches("Rakefile"));
    }

    #[test]
    fn a_deeper_scope_outranks_a_shallower_one() {
        let deep = conv(Scope::DirExt("app/services/enrolments".into(), "rb".into()));
        let shallow = conv(Scope::DirExt("app".into(), "rb".into()));
        let ext = conv(Scope::Ext("rb".into()));
        let repo = conv(Scope::Repo);
        assert!(deep.rank() > shallow.rank());
        assert!(shallow.rank() > ext.rank());
        assert!(ext.rank() > repo.rank());
    }

    #[test]
    fn at_equal_depth_confidence_breaks_the_tie() {
        let mut strong = conv(Scope::Dir("app".into()));
        strong.confidence = Confidence::derive(52, 52).unwrap();
        let weak = conv(Scope::Dir("app".into()));
        assert!(strong.rank() > weak.rank());
    }

    #[test]
    fn the_repo_scope_matches_everything_including_dotfiles() {
        assert!(Scope::Repo.matches(".github/workflows/ci.yml"));
        assert!(Scope::Repo.matches("README.md"));
    }

    #[test]
    fn rendering_a_line_carries_the_evidence_count() {
        assert_eq!(
            conv(Scope::Repo).render_line(),
            "- Types expose exactly 1 public method. (47/52, 0.90)"
        );
    }

    #[test]
    fn a_convention_round_trips_through_json() {
        let c = conv(Scope::DirExt("app/services".into(), "rb".into()));
        let back: Convention = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(c, back);
    }
}
