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
    let tested: std::collections::HashSet<String> = files
        .iter()
        .filter(|f| is_test(f))
        .map(|f| {
            let stem = f.stem.as_str();
            let base = test_marker(stem).map_or(stem, |m| stem.trim_end_matches(m));
            base.to_ascii_lowercase()
        })
        .collect();

    let mut groups: HashMap<(String, String), Vec<&FileEntry>> = HashMap::new();
    for f in files.iter().filter(|f| !is_test(f) && nameable(f)) {
        for dir in group_keys(f) {
            groups.entry((dir, f.ext.clone())).or_default().push(f);
        }
    }

    let mut out = Vec::new();
    for ((dir, ext), members) in groups {
        if members.len() < settings.min_files || dir.is_empty() {
            continue;
        }
        let agreeing =
            members.iter().filter(|f| tested.contains(&f.stem.to_ascii_lowercase())).count();
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

/// Whether a file may contribute to a naming rule.
fn nameable(entry: &FileEntry) -> bool {
    !ASSET_EXTENSIONS.contains(&entry.ext.as_str())
        && !CONVENTIONAL_NAMES.contains(&entry.stem.to_ascii_lowercase().as_str())
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
        let stems: Vec<String> = members.iter().map(|f| f.stem.clone()).collect();
        let shared = naming::shared_styles(&stems);
        // More than one surviving style means the sample never distinguished
        // them; take the most specific witnessed, or say nothing.
        let Some(style) = pick_style(&shared) else { continue };

        let agreeing = members.iter().filter(|f| naming::is_compatible(&f.stem, style)).count();
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
            enforcement: crate::semantic::enforcement_for("naming", confidence, settings),
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
        let mut counts: HashMap<&'static str, usize> = HashMap::new();
        for f in &members {
            if let Some(marker) = test_marker(&f.stem) {
                *counts.entry(marker).or_default() += 1;
            }
        }
        let Some((marker, agreeing)) = counts.into_iter().max_by_key(|(_, n)| *n) else { continue };
        let Some(confidence) = Confidence::derive(agreeing, members.len()) else { continue };

        out.push(Convention {
            id: format!("tests.suffix.{ext}"),
            statement: format!("Test files are named `*{marker}.{ext}`"),
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

/// The suffix a test file uses, when it uses one.
fn test_marker(stem: &str) -> Option<&'static str> {
    ["_spec", "_test", ".spec", ".test", "Test", "_tests"]
        .into_iter()
        .find(|&marker| stem.ends_with(marker))
        .map(|v| v as _)
}

fn is_test(f: &FileEntry) -> bool {
    test_marker(&f.stem).is_some()
        || f.dir.split('/').any(|s| matches!(s, "spec" | "test" | "tests" | "__tests__"))
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
