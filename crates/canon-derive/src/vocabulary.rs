//! Every sentence canon can state, and what has to be true to check one.
//!
//! Two contracts used to live in two places and drift apart, and both had
//! already shipped a bug.
//!
//! The first is between a family and the extraction pass its facts come from.
//! [`crate::verify`] takes the cheap structural pass unless a rule in scope
//! asks for more, and a family whose facts come only from the query pass does
//! not go quiet when it is left off that list — `extract_structure` leaves the
//! field empty, so the check reports a violation against every file it sees.
//! `FileFacts::raises` is extracted, has a pattern in every language's query
//! and is read by no rule; the next family to use it would walk straight into
//! this.
//!
//! The second is between the words a rule is stated in and the words its check
//! splits that statement on. Deriving formats a sentence and checking parses
//! one, and when the two disagree by a single character the rule derives,
//! reaches the model, and is silently never checked. That failure shipped
//! three times, once per prefix added, each time with nothing to say so.
//!
//! Both become one thing here: a family declares its words once, deriving
//! formats from them and checking parses with them, and the pass it needs is a
//! field rather than a list somewhere else.

use crate::naming;

/// How a family's statement spells the value it carries.
///
/// The parse and the format are the same knowledge read in two directions, so
/// they are one value rather than a `format!` in one file and a `split_once`
/// in another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Spelling {
    /// A backticked name after these words: "Types here inherit from `X`".
    ///
    /// The words stop short of the opening backtick, which is what keeps a
    /// family whose prefix is a strict prefix of another's from swallowing it.
    Named(&'static str),
    /// The same, where the words carry the opening backtick and a character of
    /// their own through: "Files here carry `@X`".
    Prefixed(&'static str),
    /// A count after these words: "Types here expose exactly 1 public method".
    Counted(&'static str),
    /// A naming style's label after these words, which is neither backticked
    /// nor a number: "Files here are named in `snake_case`".
    Labelled(&'static str),
    /// The whole statement, one of a closed set: "Files here export a
    /// default".
    OneOf(&'static [&'static str]),
}

impl Spelling {
    /// The backticked name this statement carries, when it carries one.
    pub(crate) fn name(self, statement: &str) -> Option<String> {
        match self {
            Self::Named(p) => backticked(statement, p),
            Self::Prefixed(p) => Some(statement.strip_prefix(p)?.strip_suffix('`')?.to_string()),
            _ => None,
        }
    }

    /// The count this statement carries, when it carries one.
    pub(crate) fn count(self, statement: &str) -> Option<usize> {
        match self {
            Self::Counted(p) => trailing_count(statement, p),
            _ => None,
        }
    }

    /// Whether this is a statement of the family that spells them this way.
    ///
    /// What the guard below is written against: a statement that answers yes
    /// to two families is a statement one of them will read the other's value
    /// out of.
    pub(crate) fn parses(self, statement: &str) -> bool {
        match self {
            Self::Named(_) | Self::Prefixed(_) => self.name(statement).is_some(),
            Self::Counted(_) => self.count(statement).is_some(),
            Self::Labelled(p) => statement
                .strip_prefix(p)
                .is_some_and(|rest| naming::Style::ALL.iter().any(|s| s.label() == rest)),
            Self::OneOf(whole) => whole.contains(&statement),
        }
    }

    /// A statement of this family naming `value`.
    ///
    /// The format side of [`Spelling::parses`]. Deriving writes one of these
    /// and checking reads it back, so they are two directions of one value
    /// rather than a `format!` in one file and a `split_once` in another.
    pub(crate) fn say(self, value: &str) -> String {
        match self {
            Self::Named(p) => format!("{p}`{value}`"),
            Self::Prefixed(p) => format!("{p}{value}`"),
            Self::Counted(p) | Self::Labelled(p) => format!("{p}{value}"),
            Self::OneOf(_) => value.to_string(),
        }
    }

    /// Statements a real rule of this family would carry.
    ///
    /// Built from the spelling rather than written out, so the guard cannot
    /// pass against a literal the derivation stopped producing.
    #[cfg(test)]
    pub(crate) fn samples(self) -> Vec<String> {
        match self {
            Self::Named(p) => vec![format!("{p}`Example`")],
            Self::Prefixed(p) => vec![format!("{p}Example`")],
            Self::Counted(p) => vec![format!("{p}1 thing")],
            Self::Labelled(p) => {
                naming::Style::ALL.iter().map(|s| format!("{p}{}", s.label())).collect()
            }
            Self::OneOf(whole) => whole.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

/// Which reading of a file a family's facts come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reads {
    /// Nothing in the file at all. A path is enough.
    Path,
    /// The declarations `extract_structure` resolves.
    Structure,
    /// Facts only the tree-sitter query pass records: calls, imports,
    /// annotations, raises. Checking one of these against the structural pass
    /// compares two different readings of the same file, and the field it
    /// wants is empty in the cheap one.
    Query,
}

/// One family: what its rules are called, how they are worded, and what has to
/// have been read for a check to answer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Family {
    /// The prefix every id of this family carries.
    pub(crate) id: &'static str,
    pub(crate) spelling: Spelling,
    pub(crate) reads: Reads,
}

pub(crate) const NAMING_PREFIX: &str = "Files here are named in ";
pub(crate) const FORMAT_PREFIX: &str = "Files here are named ";
pub(crate) const SUFFIX_PREFIX: &str = "Test files are named ";
pub(crate) const COLOCATION: &str = "Every file here has a test of the same name";
pub(crate) const IMPORT_PREFIX: &str = "Files here import from ";
pub(crate) const ANNOTATION_PREFIX: &str = "Files here carry `@";
pub(crate) const MACRO_PREFIX: &str = "Files here use ";
pub(crate) const COLLABORATOR_PREFIX: &str = "Files here call ";
pub(crate) const DEFAULT_EXPORT: &str = "Files here export a default";
pub(crate) const NAMED_EXPORTS: &str = "Files here use named exports";
pub(crate) const NAMESPACE_PREFIX: &str = "Files here declare namespace ";
pub(crate) const MODULE_ARITY_PREFIX: &str = "Files here export exactly ";
pub(crate) const PUBLIC_ARITY_PREFIX: &str = "Types here expose exactly ";
pub(crate) const ENTRYPOINT_PREFIX: &str = "That public method is named ";
/// A strict prefix of [`FAMILY_PREFIX`]. Safe only because [`backticked`]
/// requires a backtick immediately after whatever it split on, so the family
/// statement's `a ` stops it; the guard below is what holds that.
pub(crate) const BASE_PREFIX: &str = "Types here inherit from ";
pub(crate) const FAMILY_PREFIX: &str = "Types here inherit from a ";
pub(crate) const MIXIN_PREFIX: &str = "Types here include ";
pub(crate) const CONTRACT_PREFIX: &str = "Types here implement ";

/// One family per const, so the derivation and the checker name the same row
/// rather than two copies of it.
pub(crate) const NAMING: Family =
    Family { id: "naming.", spelling: Spelling::Labelled(NAMING_PREFIX), reads: Reads::Path };
pub(crate) const SUFFIX: Family =
    Family { id: "tests.suffix", spelling: Spelling::Named(SUFFIX_PREFIX), reads: Reads::Path };
pub(crate) const FORMAT: Family =
    Family { id: "format.", spelling: Spelling::Named(FORMAT_PREFIX), reads: Reads::Path };
/// Answered outside `verify_source` entirely: the one check that has to look
/// for a sibling that does not exist yet. See [`crate::verify::missing_test`].
pub(crate) const COLOCATION_FAMILY: Family =
    Family { id: "tests.colocation", spelling: Spelling::OneOf(&[COLOCATION]), reads: Reads::Path };
pub(crate) const IMPORT: Family =
    Family { id: "shape.import", spelling: Spelling::Named(IMPORT_PREFIX), reads: Reads::Query };
pub(crate) const EXPORT: Family = Family {
    id: "shape.export",
    spelling: Spelling::OneOf(&[DEFAULT_EXPORT, NAMED_EXPORTS]),
    reads: Reads::Structure,
};
pub(crate) const NAMESPACE: Family = Family {
    id: "shape.namespace",
    spelling: Spelling::Named(NAMESPACE_PREFIX),
    reads: Reads::Structure,
};
pub(crate) const ANNOTATION: Family = Family {
    id: "shape.annotation",
    spelling: Spelling::Prefixed(ANNOTATION_PREFIX),
    reads: Reads::Query,
};
pub(crate) const MACROS: Family =
    Family { id: "shape.macros", spelling: Spelling::Named(MACRO_PREFIX), reads: Reads::Query };
/// Derived and deliberately unchecked: a layering rule states who a directory
/// talks to, and a file that talks to someone else is describing a different
/// job rather than breaking one. Listed all the same, so its words are held
/// apart from every other family's.
pub(crate) const COLLABORATOR: Family = Family {
    id: "shape.collaborator",
    spelling: Spelling::Named(COLLABORATOR_PREFIX),
    reads: Reads::Query,
};
pub(crate) const MODULE_ARITY: Family = Family {
    id: "shape.module-arity",
    spelling: Spelling::Counted(MODULE_ARITY_PREFIX),
    reads: Reads::Structure,
};
pub(crate) const PUBLIC_ARITY: Family = Family {
    id: "shape.public-arity",
    spelling: Spelling::Counted(PUBLIC_ARITY_PREFIX),
    reads: Reads::Structure,
};
pub(crate) const ENTRYPOINT: Family = Family {
    id: "shape.entrypoint",
    spelling: Spelling::Named(ENTRYPOINT_PREFIX),
    reads: Reads::Structure,
};
pub(crate) const BASE: Family =
    Family { id: "shape.base", spelling: Spelling::Named(BASE_PREFIX), reads: Reads::Structure };
pub(crate) const BASE_FAMILY: Family = Family {
    id: "shape.family",
    spelling: Spelling::Named(FAMILY_PREFIX),
    reads: Reads::Structure,
};
pub(crate) const MIXIN: Family =
    Family { id: "shape.mixin", spelling: Spelling::Named(MIXIN_PREFIX), reads: Reads::Structure };
pub(crate) const CONTRACT: Family = Family {
    id: "shape.contract",
    spelling: Spelling::Named(CONTRACT_PREFIX),
    reads: Reads::Structure,
};

/// Every family, in the order [`crate::verify`] runs their checks.
///
/// Adding a row is what adding a family costs, and it is the whole cost: the
/// derivation formats its statement from the spelling here and names its rules
/// after the id here, the checker parses that statement with the same
/// spelling, `wants_query_pass` reads `reads`, and the guard in
/// [`crate::verify`] enumerates the lot.
pub(crate) const FAMILIES: &[Family] = &[
    NAMING,
    SUFFIX,
    FORMAT,
    COLOCATION_FAMILY,
    IMPORT,
    EXPORT,
    NAMESPACE,
    ANNOTATION,
    MACROS,
    COLLABORATOR,
    MODULE_ARITY,
    PUBLIC_ARITY,
    ENTRYPOINT,
    BASE,
    BASE_FAMILY,
    MIXIN,
    CONTRACT,
];

/// The family a rule belongs to, by its id.
pub(crate) fn family_of(id: &str) -> Option<&'static Family> {
    FAMILIES.iter().find(|f| id.starts_with(f.id))
}

/// The integer immediately after `prefix`, e.g. `1` in "expose exactly 1 ...".
pub(crate) fn trailing_count(statement: &str, prefix: &str) -> Option<usize> {
    let rest = statement.split_once(prefix)?.1;
    rest.split_whitespace().next()?.parse().ok()
}

/// The backticked identifier immediately after `prefix`.
pub(crate) fn backticked(statement: &str, prefix: &str) -> Option<String> {
    let rest = statement.split_once(prefix)?.1;
    let inner = rest.strip_prefix('`')?;
    inner.split_once('`').map(|(name, _)| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_statement_parses_under_another_familys_spelling() {
        // `Types here inherit from ` is a strict prefix of `Types here inherit
        // from a `, `Files here use ` of `Files here use named exports`, and
        // `Files here are named ` of `Files here are named in snake_case`.
        // Every one of those is safe by one character, and that character is
        // the whole separation between correct behaviour and a rule that
        // derives, reaches the model and is never checked.
        for family in FAMILIES {
            for statement in family.spelling.samples() {
                assert!(
                    family.spelling.parses(&statement),
                    "`{}` no longer reads its own `{statement}`",
                    family.id
                );
                for other in FAMILIES {
                    if other.id == family.id {
                        continue;
                    }
                    assert!(
                        !other.spelling.parses(&statement),
                        "`{statement}` parses as `{}` as well as `{}`",
                        other.id,
                        family.id
                    );
                }
            }
        }
    }

    #[test]
    fn no_id_prefix_claims_another_familys_rules() {
        // `family_of` takes the first row whose prefix matches, so two rows
        // where one prefix contains the other would route every rule of the
        // longer one to the shorter.
        for family in FAMILIES {
            for other in FAMILIES {
                assert!(
                    other.id == family.id || !family.id.starts_with(other.id),
                    "`{}` also matches every id of `{}`",
                    other.id,
                    family.id
                );
            }
        }
    }
}
