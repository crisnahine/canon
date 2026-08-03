//! Conventions derivable from paths alone.
//!
//! No grammar, so these work on any text repository in any language from the
//! moment canon is installed, including the ones with no extractor and the
//! ones nobody has heard of.
//!
//! Rules are derived at every ancestor directory, not just the leaf. A file
//! written into a folder that did not exist at index time still inherits the
//! nearest ancestor that has rules, which is the ordinary case when someone
//! adds a domain folder.

use std::collections::HashMap;

use canon_core::{Confidence, Convention, Enforcement, Evidence, Scope, Settings};

use crate::naming::{self, Style};
use crate::walk::FileEntry;

/// Ancestor depth past which grouping stops paying for itself.
const MAX_GROUP_DEPTH: usize = 4;

/// Evidence paths carried per convention. Enough to audit, small enough that a
/// snapshot of a large repository stays a few hundred kilobytes.
const MAX_EVIDENCE: usize = 12;

pub(crate) fn derive(files: &[FileEntry], settings: &Settings) -> Vec<Convention> {
    let mut out = Vec::new();
    out.extend(naming_conventions(files, settings));
    out.extend(test_suffix(files, settings));
    out.extend(colocation(files, settings));
    out
}

/// "Every file here has a test."
///
/// A path-level fact, needing no parser, and it prompts the file a model is
/// most likely to skip. Stated as a proportion because the interesting part is
/// the habit, not any individual pairing.
///
/// Matching is by stem: `charge_card.rb` pairs with `charge_card_spec.rb` or
/// `chargeCard.test.ts` wherever they live. Path-shaped mirroring is left
/// alone deliberately, because `spec/` mirrors `app/` in some repositories,
/// `__tests__/` sits beside the file in others, and guessing wrong produces a
/// rule that tells someone to create a file in a directory nobody uses.
fn colocation(files: &[FileEntry], settings: &Settings) -> Vec<Convention> {
    // Keyed by (name, extension). Matching on the name alone made every
    // `src/**/composer.json` in a PHP framework "tested" because one fixture
    // under `tests/` happened to be called `composer.json` too.
    let tested: std::collections::HashSet<(String, String)> = files
        .iter()
        .filter(|f| is_test(f))
        .filter_map(|f| {
            let base = subject_of_test(&f.stem, &f.ext);
            (!base.is_empty()).then(|| (base.to_lowercase(), f.ext.clone()))
        })
        .collect();

    let mut groups: HashMap<(String, String), Vec<&FileEntry>> = HashMap::new();
    for f in files.iter().filter(|f| !is_test(f) && testable(f)) {
        for dir in group_keys(f) {
            groups.entry((dir, f.ext.clone())).or_default().push(f);
        }
    }

    let mut out = Vec::new();
    for ((dir, ext), members) in groups {
        if members.len() < settings.min_files || dir.is_empty() {
            continue;
        }
        let agreeing = members
            .iter()
            .filter(|f| {
                tested.contains(&(naming::name_root(&f.stem).to_lowercase(), f.ext.clone()))
            })
            .count();
        let Some(confidence) = Confidence::derive(agreeing, members.len()) else { continue };

        out.push(Convention {
            id: format!("tests.colocation.{}.{ext}", id_fragment(&dir)),
            statement: "Every file here has a test of the same name".to_string(),
            scope: scope_for(&dir, &ext),
            confidence,
            agreeing,
            total: members.len(),
            exemplar: exemplar_of(&members),
            evidence: evidence_of(&members),
            // Path-shaped, but not exact: a file may legitimately be the one
            // thing in a directory that needs no test.
            enforcement: Enforcement::Advisory,
        });
    }
    out
}

/// Extensions whose names no model will ever choose.
///
/// A naming rule over 2,912 `.jpg` files is true and useless: it takes a slot
/// in a 1,500-character budget from a rule that would change the output. The
/// list is a deny-list rather than an allow-list of code extensions, because
/// Tier 0 is supposed to work on any text repository, including one written in
/// a language canon has never heard of.
const ASSET_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "svg", "webp", "avif", "ico", "bmp", "tiff", "pdf", "psd", "ai",
    "woff", "woff2", "ttf", "otf", "eot", "mp3", "mp4", "wav", "ogg", "webm", "mov", "avi", "zip",
    "gz", "tar", "bz2", "7z", "rar", "jar", "war", "so", "dylib", "dll", "exe", "bin", "wasm",
    "class", "pyc", "o", "a", "lib", "map", "lock", "min",
];

/// File names that describe their role rather than following a convention.
///
/// Five of these at a repository root derived "files here are named in
/// `SCREAMING_SNAKE_CASE`" for every `.md` in the project. The inference is
/// arithmetically correct and practically wrong: asked for a new document, a
/// model following it produces `MY_NEW_DOC.md`.
const CONVENTIONAL_NAMES: &[&str] = &[
    "readme",
    "changelog",
    "license",
    "licence",
    "contributing",
    "code_of_conduct",
    "security",
    "authors",
    "notice",
    "makefile",
    "dockerfile",
    "rakefile",
    "gemfile",
    "procfile",
    "vagrantfile",
    "brewfile",
    "justfile",
    "todo",
    "install",
    "upgrading",
    "history",
    "news",
    "copying",
    "version",
    "owners",
    "codeowners",
    "maintainers",
    "support",
    "governance",
    "roadmap",
];

/// Extensions that hold data rather than code, and therefore have no test.
///
/// Separate from [`ASSET_EXTENSIONS`], which is about names nobody chooses.
/// These do get naming rules — `docs/**/*.md` in kebab-case is a real and
/// useful convention — but "every file here has a test of the same name" is
/// not a claim that can be true of a `composer.json` or a `.gitattributes`,
/// and it was derived at total agreement for both.
const DATA_EXTENSIONS: &[&str] = &[
    "json",
    "yml",
    "yaml",
    "toml",
    "xml",
    "ini",
    "cfg",
    "conf",
    "env",
    "properties",
    "plist",
    "md",
    "mdx",
    "rst",
    "adoc",
    "txt",
    "csv",
    "tsv",
    "html",
    "htm",
    "css",
    "scss",
    "sass",
    "less",
    "styl",
    "snap",
    "gitattributes",
    "gitignore",
    "gitmodules",
    "editorconfig",
    "dockerignore",
    "npmrc",
    "nvmrc",
    "prettierrc",
    "eslintrc",
    "babelrc",
];

/// Whether a file may contribute to a naming rule.
///
/// A dotfile has an empty name root and nothing to classify. A dunder name
/// describes a role the language assigns rather than a name anyone chose:
/// `__init__.py` and `__main__.py` are in every Python package, and a leading
/// underscore is compatible with no style at all, so counting them meant no
/// Python directory could ever produce a naming rule.
fn nameable(entry: &FileEntry) -> bool {
    let root = naming::name_root(&entry.stem);
    !root.is_empty()
        && !is_dunder(root)
        && !ASSET_EXTENSIONS.contains(&entry.ext.as_str())
        && !CONVENTIONAL_NAMES.contains(&entry.stem.to_ascii_lowercase().as_str())
}

fn is_dunder(root: &str) -> bool {
    root.starts_with("__")
        && root.ends_with("__")
        && root.len() > 4
        && !root.trim_matches('_').is_empty()
}

/// Whether a file is the kind of thing that has a test.
fn testable(entry: &FileEntry) -> bool {
    nameable(entry) && !DATA_EXTENSIONS.contains(&entry.ext.as_str())
}

/// Whether a naming rule has anything to say about this path.
///
/// The same exclusions [`nameable`] and [`is_test`] apply when deriving, from
/// a path alone so the check can ask about a file that is about to be written.
///
/// Deriving and checking have to agree here or the rule is applied to files it
/// was never counted over, and it refuses them. Measured against fourteen real
/// repositories, that was every false positive enforcement produced:
/// `__init__.py` against a `snake_case` rule derived from files that excluded
/// it, `AUTHORS.rst` against a `kebab-case` one, and every
/// `__tests__/foo-test.ts`, `*.spec.ts` and `test/fixtures/**/*.vue` against a
/// rule derived only from the code they test.
pub(crate) fn counts_toward_naming(rel: &str) -> bool {
    let name = rel.rsplit_once('/').map_or(rel, |(_, n)| n);
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let ext = ext.to_ascii_lowercase();
    let root = naming::name_root(stem);
    !root.is_empty()
        && !is_dunder(root)
        && !ASSET_EXTENSIONS.contains(&ext.as_str())
        && !CONVENTIONAL_NAMES.contains(&stem.to_ascii_lowercase().as_str())
        && !is_test_path(rel)
}

/// Every `(directory prefix, extension)` group a file belongs to.
fn group_keys(entry: &FileEntry) -> Vec<String> {
    if entry.ext.is_empty() {
        return Vec::new();
    }
    let mut keys = vec![String::new()];
    let mut acc = String::new();
    for segment in entry.dir.split('/').filter(|s| !s.is_empty()).take(MAX_GROUP_DEPTH) {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(segment);
        keys.push(acc.clone());
    }
    keys
}

/// "Files in `app/services/` are named in `snake_case`."
fn naming_conventions(files: &[FileEntry], settings: &Settings) -> Vec<Convention> {
    let mut groups: HashMap<(String, String), Vec<&FileEntry>> = HashMap::new();
    for f in files.iter().filter(|f| !is_test(f) && nameable(f)) {
        for dir in group_keys(f) {
            groups.entry((dir, f.ext.clone())).or_default().push(f);
        }
    }

    let mut out = Vec::new();
    for ((dir, ext), members) in groups {
        if members.len() < settings.min_files {
            continue;
        }
        let roots: Vec<String> =
            members.iter().map(|f| naming::name_root(&f.stem).to_string()).collect();
        // Gated on distinct names, not on file count. A directory of identical
        // names is one observation however many files carry it.
        let shared = naming::shared_styles(&roots, settings.min_files);
        // More than one surviving style means the sample never distinguished
        // them; take the most specific witnessed, or say nothing.
        let Some(style) = pick_style(&shared) else { continue };

        let agreeing = members
            .iter()
            .filter(|f| naming::is_compatible(naming::name_root(&f.stem), style))
            .count();
        let Some(confidence) = Confidence::derive(agreeing, members.len()) else { continue };

        out.push(Convention {
            id: format!("naming.{}.{ext}", id_fragment(&dir)),
            statement: format!("Files here are named in {}", style.label()),
            scope: scope_for(&dir, &ext),
            confidence,
            agreeing,
            total: members.len(),
            exemplar: exemplar_of(&members),
            evidence: evidence_of(&members),
            enforcement: canon_core::enforcement_for("naming", confidence, settings),
        });
    }
    out
}

/// When several styles survive, prefer the one an actual multi-word name
/// witnessed rather than an alphabetical coin flip.
fn pick_style(shared: &[Style]) -> Option<Style> {
    // Exactly one, or nothing. Two surviving styles means the sample never
    // distinguished them, and picking either would be a coin flip presented
    // as a finding.
    match shared {
        [only] => Some(*only),
        _ => None,
    }
}

/// "Test files are named `*_spec.rb`."
///
/// Derived per extension across the whole repository, because a team picks one
/// suffix and uses it everywhere. Deriving it per directory would split the
/// sample until nothing cleared the gate.
fn test_suffix(files: &[FileEntry], settings: &Settings) -> Vec<Convention> {
    let mut by_ext: HashMap<String, Vec<&FileEntry>> = HashMap::new();
    for f in files.iter().filter(|f| is_test(f)) {
        by_ext.entry(f.ext.clone()).or_default().push(f);
    }

    let mut out = Vec::new();
    for (ext, members) in by_ext {
        if members.len() < settings.min_files {
            continue;
        }
        let mut counts: HashMap<Marker, usize> = HashMap::new();
        for f in &members {
            if let Some(marker) = test_marker(&f.stem, &f.ext) {
                *counts.entry(marker).or_default() += 1;
            }
        }
        // Ties break on the marker text so an unchanged tree derives the same
        // rule twice; `max_by_key` over a HashMap otherwise follows iteration
        // order, which is not stable.
        let Some((marker, agreeing)) = counts.into_iter().max_by_key(|(m, n)| (*n, m.glob(&ext)))
        else {
            continue;
        };
        let Some(confidence) = Confidence::derive(agreeing, members.len()) else { continue };

        out.push(Convention {
            id: format!("tests.suffix.{ext}"),
            statement: format!("Test files are named `{}`", marker.glob(&ext)),
            scope: Scope::Ext(ext.clone()),
            confidence,
            agreeing,
            total: members.len(),
            exemplar: exemplar_of(&members),
            evidence: evidence_of(&members),
            enforcement: Enforcement::Advisory,
        });
    }
    out
}

/// How a test file is named, relative to the thing it tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Marker {
    /// `charge_card_spec.rb`, `chargeCard.test.ts`.
    Suffix(&'static str),
    /// `test_charge_card.py`. Python's dominant convention, and invisible to a
    /// suffix-only match: `pytest` collects `test_*.py` by default, so a
    /// Python repository derived no test-naming rule at all.
    Prefix(&'static str),
}

impl Marker {
    /// The rule as a glob, for the statement a reader sees.
    fn glob(self, ext: &str) -> String {
        match self {
            Self::Suffix(m) => format!("*{m}.{ext}"),
            Self::Prefix(m) => format!("{m}*.{ext}"),
        }
    }
}

/// The marker a test file uses, when it uses one.
///
/// The prefix form is Python's and only Python's. `pytest` collects `test_*.py`
/// by default, so without it a Python repository derives no test-naming rule at
/// all. Applied everywhere it misfires: Go's `test_helpers.go` is a helper —
/// a Go test has to end in `_test.go` — and a Rake task called
/// `spec_runner.rake` is a task.
fn test_marker(stem: &str, ext: &str) -> Option<Marker> {
    const SUFFIXES: &[&str] = &["_spec", "_test", ".spec", ".test", "Test", "_tests"];
    if let Some(m) = SUFFIXES.iter().copied().find(|m| stem.ends_with(m)) {
        return Some(Marker::Suffix(m));
    }
    // A bare `test.py` is the test, not a test of something called `""`.
    (matches!(ext, "py" | "pyi") && stem.starts_with("test_") && stem.len() > "test_".len())
        .then_some(Marker::Prefix("test_"))
}

/// Qualifiers between dots that mean "this file is a test".
///
/// `componentInstance.test-d.tsx` is a type test and carries no marker the
/// suffix match recognises, because the qualifier is not last. Five of them
/// were the only `.tsx` files in a Vue repository, and derived an enforced
/// repository-wide rule that every `.tsx` is camelCase — which would have
/// refused an ordinary `MyComponent.tsx`.
const TEST_QUALIFIERS: &[&str] = &["test", "tests", "spec", "test-d", "e2e", "cy", "stories"];

/// The name of the thing a test file tests, stripped of its marker and of any
/// qualifier after the first dot.
fn subject_of_test<'s>(stem: &'s str, ext: &str) -> &'s str {
    let base = match test_marker(stem, ext) {
        Some(Marker::Suffix(m)) => stem.strip_suffix(m).unwrap_or(stem),
        Some(Marker::Prefix(m)) => stem.strip_prefix(m).unwrap_or(stem),
        None => stem,
    };
    naming::name_root(base)
}

fn is_test(f: &FileEntry) -> bool {
    is_test_path(&f.rel)
}

/// Whether a repository-relative path is a test file.
///
/// The same judgement the derivation makes, reachable from a path alone so
/// selection can ask it about a file that is about to be written and is not in
/// the index yet.
pub(crate) fn is_test_path(rel: &str) -> bool {
    let name = rel.rsplit_once('/').map_or(rel, |(_, n)| n);
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
    test_marker(stem, &ext.to_ascii_lowercase()).is_some()
        || stem.split('.').skip(1).any(|q| TEST_QUALIFIERS.contains(&q))
        || dir.split('/').any(|s| matches!(s, "spec" | "test" | "tests" | "__tests__"))
}

/// The most recently modified agreeing file.
///
/// Recency rather than any other tie-break: an old file encodes a convention
/// the team has already moved off, and pointing someone at one teaches the
/// shape they are supposed to be replacing.
fn exemplar_of(members: &[&FileEntry]) -> Option<String> {
    members.iter().max_by_key(|f| (f.modified_unix, f.rel.clone())).map(|f| f.rel.clone())
}

fn evidence_of(members: &[&FileEntry]) -> Vec<Evidence> {
    let mut sorted: Vec<&&FileEntry> = members.iter().collect();
    sorted.sort_by(|a, b| b.modified_unix.cmp(&a.modified_unix).then(a.rel.cmp(&b.rel)));
    sorted
        .into_iter()
        .take(MAX_EVIDENCE)
        .map(|f| Evidence { rel: f.rel.clone(), line: 0 })
        .collect()
}

pub(crate) fn scope_for(dir: &str, ext: &str) -> Scope {
    if dir.is_empty() {
        Scope::Ext(ext.to_string())
    } else {
        Scope::DirExt(dir.to_string(), ext.to_string())
    }
}

pub(crate) fn id_fragment(dir: &str) -> String {
    if dir.is_empty() { "repo".to_string() } else { dir.replace('/', ".") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use crate::walk::walk;

    fn derive_from(name: &str, files: &[(&str, &str)]) -> Vec<Convention> {
        let root = fixture::build(name, files);
        let settings = Settings::default();
        derive(&walk(&root, &settings), &settings)
    }

    fn statements(convs: &[Convention]) -> String {
        convs.iter().map(|c| format!("{}: {}", c.id, c.statement)).collect::<Vec<_>>().join(" | ")
    }

    #[test]
    fn a_snake_case_directory_produces_a_naming_convention() {
        let convs = derive_from(
            "t0-snake",
            &[
                ("app/services/create_enrolment.rb", "x"),
                ("app/services/update_enrolment.rb", "x"),
                ("app/services/cancel_enrolment.rb", "x"),
                ("app/services/refund_payment.rb", "x"),
                ("app/services/approve_payout.rb", "x"),
            ],
        );
        assert!(statements(&convs).contains("snake_case"), "got {}", statements(&convs));
    }

    #[test]
    fn a_directory_of_single_words_produces_no_naming_convention() {
        // Compatible with three styles at once, so it witnesses none.
        let convs = derive_from(
            "t0-ambiguous",
            &[
                ("app/a/create.rb", "x"),
                ("app/a/update.rb", "x"),
                ("app/a/cancel.rb", "x"),
                ("app/a/refund.rb", "x"),
                ("app/a/approve.rb", "x"),
            ],
        );
        assert!(!statements(&convs).contains("named in"), "got {}", statements(&convs));
    }

    #[test]
    fn a_mixed_directory_produces_no_naming_convention() {
        let convs = derive_from(
            "t0-mixed",
            &[
                ("src/CreateThing.tsx", "x"),
                ("src/update_thing.tsx", "x"),
                ("src/cancelThing.tsx", "x"),
                ("src/refund-thing.tsx", "x"),
                ("src/ApproveThing.tsx", "x"),
            ],
        );
        assert!(!statements(&convs).contains("named in"), "got {}", statements(&convs));
    }

    #[test]
    fn pascal_case_components_are_recognised() {
        let convs = derive_from(
            "t0-pascal",
            &[
                ("src/components/UserCard.tsx", "x"),
                ("src/components/OrderList.tsx", "x"),
                ("src/components/PayoutForm.tsx", "x"),
                ("src/components/LoginPanel.tsx", "x"),
                ("src/components/NavBar.tsx", "x"),
            ],
        );
        assert!(statements(&convs).contains("PascalCase"), "got {}", statements(&convs));
    }

    #[test]
    fn a_test_suffix_convention_is_derived_across_the_repository() {
        let convs = derive_from(
            "t0-tests",
            &[
                ("spec/a_spec.rb", "x"),
                ("spec/b_spec.rb", "x"),
                ("spec/c_spec.rb", "x"),
                ("spec/d_spec.rb", "x"),
                ("spec/e_spec.rb", "x"),
            ],
        );
        assert!(statements(&convs).contains("`*_spec.rb`"), "got {}", statements(&convs));
    }

    #[test]
    fn test_files_do_not_pollute_the_source_naming_sample() {
        // Otherwise `a_spec` drags every Ruby directory toward snake_case even
        // where the source files disagree.
        let convs = derive_from(
            "t0-separation",
            &[
                ("src/UserCard.tsx", "x"),
                ("src/OrderList.tsx", "x"),
                ("src/PayoutForm.tsx", "x"),
                ("src/LoginPanel.tsx", "x"),
                ("src/NavBar.tsx", "x"),
                ("src/user_card.test.tsx", "x"),
                ("src/order_list.test.tsx", "x"),
            ],
        );
        assert!(statements(&convs).contains("PascalCase"), "got {}", statements(&convs));
    }

    #[test]
    fn conventions_are_derived_at_ancestor_levels_too() {
        let convs = derive_from(
            "t0-ancestors",
            &[
                ("app/services/enrolments/create_a.rb", "x"),
                ("app/services/enrolments/create_b.rb", "x"),
                ("app/services/payouts/create_c.rb", "x"),
                ("app/services/payouts/create_d.rb", "x"),
                ("app/services/payouts/create_e.rb", "x"),
            ],
        );
        // The leaf groups are too small on their own; the ancestor carries it.
        let scopes: Vec<&Scope> = convs.iter().map(|c| &c.scope).collect();
        assert!(
            scopes.iter().any(|s| matches!(s, Scope::DirExt(d, _) if d == "app/services")),
            "got {scopes:?}"
        );
    }

    #[test]
    fn a_group_below_the_sample_gate_produces_nothing() {
        let convs = derive_from(
            "t0-small",
            &[("app/a_one.rb", "x"), ("app/b_two.rb", "x"), ("app/c_three.rb", "x")],
        );
        assert!(convs.is_empty(), "got {}", statements(&convs));
    }

    /// Exemplar choice is tested against constructed entries rather than real
    /// files: mtime has whole-second resolution, so a fixture written and
    /// rewritten inside one second carries no ordering to observe.
    fn entry(rel: &str, modified_unix: u64) -> FileEntry {
        FileEntry {
            rel: rel.into(),
            dir: "app".into(),
            ext: "rb".into(),
            stem: rel.rsplit('/').next().unwrap_or(rel).trim_end_matches(".rb").into(),
            bytes: 1,
            weight: 1.0,
            modified_unix,
        }
    }

    #[test]
    fn the_exemplar_is_the_most_recently_modified_agreeing_file() {
        let files = [
            entry("app/create_a.rb", 100),
            entry("app/create_b.rb", 300),
            entry("app/create_c.rb", 200),
        ];
        let refs: Vec<&FileEntry> = files.iter().collect();
        assert_eq!(exemplar_of(&refs).as_deref(), Some("app/create_b.rb"));
    }

    #[test]
    fn an_exemplar_tie_breaks_deterministically_rather_than_by_walk_order() {
        // Files written in the same second share an mtime, which is the common
        // case for a fresh checkout. The result must still be reproducible.
        let files = [entry("app/create_a.rb", 100), entry("app/create_z.rb", 100)];
        let refs: Vec<&FileEntry> = files.iter().collect();
        let first = exemplar_of(&refs);
        let reversed: Vec<&FileEntry> = files.iter().rev().collect();
        assert_eq!(first, exemplar_of(&reversed));
    }

    #[test]
    fn one_name_repeated_across_directories_is_not_a_naming_convention() {
        // Every crate has a `Cargo.toml`. Five of them derived "files here are
        // named in PascalCase" at total agreement, enforced, and that refused
        // an ordinary `deny.toml`. Reproduced in canon's own repository and in
        // ripgrep.
        let convs = derive_from(
            "t0-repeated",
            &[
                ("crates/a/Cargo.toml", "x"),
                ("crates/b/Cargo.toml", "x"),
                ("crates/c/Cargo.toml", "x"),
                ("crates/d/Cargo.toml", "x"),
                ("crates/e/Cargo.toml", "x"),
            ],
        );
        assert!(!statements(&convs).contains("PascalCase"), "got {}", statements(&convs));
    }

    #[test]
    fn a_qualifier_before_the_extension_is_not_part_of_the_name() {
        // Every Rails view is `*.html.erb` and every Angular service is
        // `*.service.ts`. Read to the last dot, the stem holds a `.`, which no
        // style accepts, so the whole directory produced nothing.
        let convs = derive_from(
            "t0-qualifier",
            &[
                ("app/views/charge_card.html.erb", "x"),
                ("app/views/refund_payment.html.erb", "x"),
                ("app/views/settle_batch.html.erb", "x"),
                ("app/views/send_receipt.html.erb", "x"),
                ("app/views/void_invoice.html.erb", "x"),
            ],
        );
        assert!(statements(&convs).contains("snake_case"), "got {}", statements(&convs));
    }

    #[test]
    fn one_type_declaration_file_does_not_silence_the_directory() {
        let convs = derive_from(
            "t0-dts",
            &[
                ("src/models/charge_card.ts", "x"),
                ("src/models/refund_payment.ts", "x"),
                ("src/models/settle_batch.ts", "x"),
                ("src/models/send_receipt.ts", "x"),
                ("src/models/void_invoice.ts", "x"),
                ("src/models/globals.d.ts", "x"),
            ],
        );
        assert!(statements(&convs).contains("snake_case"), "got {}", statements(&convs));
    }

    #[test]
    fn a_dunder_name_does_not_decide_a_python_package() {
        // `__init__.py` is in every package and matches no style, so counting
        // it meant no Python directory could produce a naming rule at all.
        let convs = derive_from(
            "t0-dunder",
            &[
                ("src/pkg/__init__.py", "x"),
                ("src/pkg/charge_card.py", "x"),
                ("src/pkg/refund_payment.py", "x"),
                ("src/pkg/settle_batch.py", "x"),
                ("src/pkg/send_receipt.py", "x"),
                ("src/pkg/void_invoice.py", "x"),
            ],
        );
        assert!(statements(&convs).contains("snake_case"), "got {}", statements(&convs));
    }

    #[test]
    fn a_data_file_is_never_asked_whether_it_has_a_test() {
        // `src/**/composer.json` came out at "every file here has a test of
        // the same name (37/37)" because a fixture under `tests/` shared the
        // name, and `.gitattributes` at 37/37 because empty matched empty.
        let convs = derive_from(
            "t0-colocation-data",
            &[
                ("src/a/composer.json", "{}"),
                ("src/b/composer.json", "{}"),
                ("src/c/composer.json", "{}"),
                ("src/d/composer.json", "{}"),
                ("src/e/composer.json", "{}"),
                ("tests/fixtures/app/composer.json", "{}"),
                ("tests/fixtures/.env", ""),
                ("src/a/.gitattributes", ""),
                ("src/b/.gitattributes", ""),
                ("src/c/.gitattributes", ""),
                ("src/d/.gitattributes", ""),
                ("src/e/.gitattributes", ""),
            ],
        );
        assert!(!statements(&convs).contains("has a test"), "got {}", statements(&convs));
    }

    #[test]
    fn python_names_its_tests_with_a_prefix_and_go_does_not() {
        let py = derive_from(
            "t0-py-tests",
            &[
                ("app/charge_card.py", "x"),
                ("app/refund_payment.py", "x"),
                ("tests/test_charge_card.py", "x"),
                ("tests/test_refund_payment.py", "x"),
                ("tests/test_settle_batch.py", "x"),
                ("tests/test_send_receipt.py", "x"),
                ("tests/test_void_invoice.py", "x"),
            ],
        );
        assert!(statements(&py).contains("test_*.py"), "got {}", statements(&py));

        // A Go test must end in `_test.go`; `test_helpers.go` is a helper.
        assert!(test_marker("test_helpers", "go").is_none());
        assert!(test_marker("test_charge_card", "py").is_some());
    }

    #[test]
    fn a_type_test_qualifier_marks_the_file_as_a_test() {
        // Five `*.test-d.tsx` were the only `.tsx` in Vue's repository and
        // derived an enforced rule that every `.tsx` is camelCase.
        for rel in [
            "packages/dts-test/appDirective.test-d.tsx",
            "src/Button.stories.tsx",
            "e2e/checkout.cy.ts",
            "spec/models/user_spec.rb",
            "src/utils/__tests__/base64-test.ts",
        ] {
            assert!(is_test_path(rel), "{rel} is a test");
        }
        assert!(!is_test_path("app/services/charge_card.rb"));
        assert!(!is_test_path("src/latest.ts"), "`latest` merely contains `test`");
    }

    #[test]
    fn files_without_an_extension_are_not_grouped() {
        let convs = derive_from(
            "t0-noext",
            &[
                ("bin/one_a", "x"),
                ("bin/two_b", "x"),
                ("bin/three_c", "x"),
                ("bin/four_d", "x"),
                ("bin/five_e", "x"),
            ],
        );
        assert!(convs.is_empty(), "got {}", statements(&convs));
    }
}
