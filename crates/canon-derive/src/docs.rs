//! Conventions for prose files, derived without a grammar.
//!
//! Documentation was the largest class canon could say nothing about. Measured
//! across eight unrelated repositories, `.md` was the most common extension
//! with no rule covering it, and a repository holding 359 markdown files
//! answered `no conventions match` for every one of them. Teams hold their
//! documents to a house style as firmly as their code, and the parts a
//! newcomer gets wrong are positional: whether a file opens with front matter,
//! what heading level it starts at, and whether fenced blocks name a language.
//!
//! No grammar is linked for this. Markdown has several and none of them agree,
//! and all three facts here are answered by reading the first lines of a file
//! and scanning for fences. The cost is one read per document, on the same
//! cold path that already reads every source file.

use std::collections::HashMap;

use canon_core::{Confidence, Convention, Enforcement, Settings};

use crate::{
    Reach,
    tier0::{evidence_of, exemplar_of, id_fragment, roots_of},
    vocabulary,
    walk::FileEntry,
};

/// Extensions read as prose.
///
/// Deliberately short. A file canon cannot recognise as a document is left
/// alone rather than guessed at, and the three facts below mean nothing in a
/// format that has no front matter and no fences.
const PROSE: &[&str] = &["md", "mdx", "markdown", "mdown"];

/// The largest prefix of a document read to answer the opening questions.
///
/// Front matter and the first heading are both near the top by definition, and
/// a generated document can be megabytes. Fences are scanned over the whole
/// file, which is why this bounds only the opening pass.
const OPENING_BYTES: usize = 4 * 1024;

/// What one document opens with, and how it tags its fenced blocks.
struct DocFacts {
    /// Whether the file opens with a `---` front matter block.
    front_matter: bool,
    /// The level of the first ATX heading, when there is one.
    first_heading: Option<usize>,
    /// The language tag on each opening fence, empty string when untagged.
    fence_tags: Vec<String>,
}

/// Read the three facts from one document's text.
fn facts_of(text: &str) -> DocFacts {
    let opening: String = text.chars().take(OPENING_BYTES).collect();
    let mut lines = opening.lines();
    // Front matter is only front matter at the very top. A `---` further down
    // is a horizontal rule, and counting one as front matter would make every
    // document with a rule in it vote the wrong way.
    let front_matter = lines.next().is_some_and(|l| l.trim_end() == "---");

    let first_heading = opening
        .lines()
        .find(|l| l.starts_with('#'))
        .map(|l| l.chars().take_while(|c| *c == '#').count());

    // Fences are counted over the whole file, and only the opening one of each
    // pair carries a tag. Toggling on every fence line is what keeps the
    // closing fence of a tagged block from being read as an untagged one.
    let mut fence_tags = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("```") && !trimmed.starts_with("~~~") {
            continue;
        }
        if inside {
            inside = false;
            continue;
        }
        inside = true;
        let tag = trimmed.trim_start_matches(['`', '~']).trim();
        // A tag can carry attributes, as in ```rust,ignore. The language is the
        // first word, which is the part a reader and a highlighter both use.
        let tag = tag.split([',', ' ', '{']).next().unwrap_or("").trim();
        fence_tags.push(tag.to_ascii_lowercase());
    }

    DocFacts { front_matter, first_heading, fence_tags }
}

/// One document and the three facts read from it.
type Doc<'a> = (&'a FileEntry, DocFacts);

/// Documents grouped by the scope key every other rule votes over.
type Groups<'a> = HashMap<(String, String, Reach), Vec<&'a Doc<'a>>>;

/// One group per `(directory, extension, reach)` over the documents.
fn grouped<'a>(docs: &'a [Doc<'a>]) -> Groups<'a> {
    let mut groups: Groups<'a> = HashMap::new();
    for doc in docs {
        for (dir, reach) in crate::group_keys(&doc.0.dir) {
            groups.entry((dir, doc.0.ext.clone(), reach)).or_default().push(doc);
        }
    }
    groups
}

/// Every prose convention this repository holds.
pub(crate) fn derive(
    files: &[FileEntry],
    root: &std::path::Path,
    settings: &Settings,
) -> Vec<Convention> {
    let docs: Vec<Doc<'_>> = files
        .iter()
        .filter(|f| PROSE.contains(&f.ext.as_str()))
        .filter_map(|f| {
            let text = crate::read_indexable(&root.join(&f.rel))?;
            Some((f, facts_of(&text)))
        })
        .collect();
    if docs.is_empty() {
        return Vec::new();
    }

    let groups = grouped(&docs);
    let mut out = Vec::new();
    for ((dir, ext, reach), members) in groups {
        if members.len() < settings.min_files {
            continue;
        }
        out.extend(front_matter_rule(&dir, &ext, reach, &members));
        out.extend(heading_rule(&dir, &ext, reach, &members));
        out.extend(fence_rule(&dir, &ext, reach, &members));
    }
    out
}

/// "Documents here open with YAML front matter."
///
/// Both directions are stated. A docs tree whose files all carry front matter
/// and a repository whose top-level documents deliberately carry none are the
/// same fact read two ways, and a newcomer gets each of them wrong.
fn front_matter_rule(
    dir: &str,
    ext: &str,
    reach: Reach,
    members: &[&Doc<'_>],
) -> Option<Convention> {
    let with = members.iter().filter(|d| d.1.front_matter).count();
    let (agreeing, statement) = if with * 2 > members.len() {
        (with, vocabulary::FRONTMATTER_PRESENT)
    } else {
        (members.len() - with, vocabulary::FRONTMATTER_ABSENT)
    };
    let confidence = Confidence::derive(agreeing, members.len())?;
    Some(Convention {
        id: format!("{}{}.{ext}", vocabulary::FRONTMATTER.id, id_fragment(dir)),
        statement: vocabulary::FRONTMATTER.spelling.say(statement),
        scope: crate::scope_reaching(dir, ext, reach),
        confidence,
        agreeing,
        total: members.len(),
        exemplar: exemplar_of(&refs(members)),
        evidence: evidence_of(&refs(members)),
        sample_roots: roots_of(&refs(members)),
        // Advisory in both directions. A document legitimately gains front
        // matter the day it needs a title, and refusing that is the check
        // being wrong about a correct file.
        enforcement: Enforcement::Advisory,
    })
}

/// "Documents here open at heading level 1."
///
/// A file that starts at `##` inside a tree that starts at `#` renders with no
/// title, and no linter says so.
fn heading_rule(dir: &str, ext: &str, reach: Reach, members: &[&Doc<'_>]) -> Option<Convention> {
    let mut counts: HashMap<usize, usize> = HashMap::new();
    for d in members {
        if let Some(level) = d.1.first_heading {
            *counts.entry(level).or_default() += 1;
        }
    }
    // Voted over documents that have a heading at all, not over every document
    // in the directory. A file with no heading is not evidence for a level, and
    // counting it as a dissenter is what buries the rule in a tree that holds
    // one stub.
    let voting: usize = counts.values().sum();
    if voting == 0 {
        return None;
    }
    let (level, agreeing) = counts.into_iter().max_by_key(|(l, n)| (*n, std::cmp::Reverse(*l)))?;
    let confidence = Confidence::derive(agreeing, voting)?;
    Some(Convention {
        id: format!("{}{}.{ext}", vocabulary::HEADING.id, id_fragment(dir)),
        statement: vocabulary::HEADING.spelling.say(&level.to_string()),
        scope: crate::scope_reaching(dir, ext, reach),
        confidence,
        agreeing,
        total: voting,
        exemplar: exemplar_of(&refs(members)),
        evidence: evidence_of(&refs(members)),
        sample_roots: roots_of(&refs(members)),
        enforcement: Enforcement::Advisory,
    })
}

/// "Code blocks here are tagged with a language, such as `bash`."
///
/// Stated only where the directory actually holds fenced blocks, and only when
/// the tagged ones win. An untagged fence renders without highlighting and
/// tells a reader nothing about what they are looking at.
fn fence_rule(dir: &str, ext: &str, reach: Reach, members: &[&Doc<'_>]) -> Option<Convention> {
    let mut tagged = 0usize;
    let mut total = 0usize;
    let mut vocab: HashMap<&str, usize> = HashMap::new();
    let mut documents_with_fences = 0usize;
    for d in members {
        if d.1.fence_tags.is_empty() {
            continue;
        }
        documents_with_fences += 1;
        for tag in &d.1.fence_tags {
            total += 1;
            if !tag.is_empty() {
                tagged += 1;
                *vocab.entry(tag.as_str()).or_default() += 1;
            }
        }
    }
    // The sample is documents, not fences, so a single document holding forty
    // blocks cannot carry a rule the rest of the directory never voted on.
    if documents_with_fences < members.len().min(3) || total == 0 || tagged * 2 <= total {
        return None;
    }
    let confidence = Confidence::derive(documents_with_fences, members.len())?;
    // Ties break on the tag text, so an unchanged tree derives the same rule
    // twice rather than whichever the hash order visited last.
    let (common, _) = vocab.into_iter().max_by_key(|(t, n)| (*n, *t))?;
    Some(Convention {
        id: format!("{}{}.{ext}", vocabulary::FENCE.id, id_fragment(dir)),
        statement: vocabulary::FENCE.spelling.say(common),
        scope: crate::scope_reaching(dir, ext, reach),
        confidence,
        agreeing: documents_with_fences,
        total: members.len(),
        exemplar: exemplar_of(&refs(members)),
        evidence: evidence_of(&refs(members)),
        sample_roots: roots_of(&refs(members)),
        enforcement: Enforcement::Advisory,
    })
}

/// The file entries behind a group, for the helpers that only want paths.
fn refs<'a>(members: &[&'a Doc<'a>]) -> Vec<&'a FileEntry> {
    members.iter().map(|d| d.0).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_is_only_front_matter_at_the_top() {
        assert!(facts_of("---\ntitle: x\n---\n# H\n").front_matter);
        // A horizontal rule further down is not front matter, and reading it as
        // one would make every document carrying a rule vote the wrong way.
        assert!(!facts_of("# H\n\n---\n\nbody\n").front_matter);
        assert!(!facts_of("# H\n").front_matter);
    }

    #[test]
    fn the_first_heading_level_is_counted_from_its_hashes() {
        assert_eq!(facts_of("# one\n").first_heading, Some(1));
        assert_eq!(facts_of("intro\n\n## two\n").first_heading, Some(2));
        assert_eq!(facts_of("no heading at all\n").first_heading, None);
    }

    #[test]
    fn a_closing_fence_is_not_read_as_an_untagged_block() {
        // Two fences, one block. Counting the closing fence as a second,
        // untagged block would put every tagged document at 50% and sink the
        // rule in a directory that is entirely correct.
        let f = facts_of("```rust\nfn main() {}\n```\n");
        assert_eq!(f.fence_tags, vec!["rust"]);
    }

    #[test]
    fn a_fence_tag_carrying_attributes_keeps_only_the_language() {
        assert_eq!(facts_of("```rust,ignore\nx\n```\n").fence_tags, vec!["rust"]);
        assert_eq!(facts_of("```js {highlight}\nx\n```\n").fence_tags, vec!["js"]);
        assert_eq!(facts_of("```\nx\n```\n").fence_tags, vec![""]);
    }

    #[test]
    fn a_tilde_fence_counts_the_same_as_a_backtick_fence() {
        assert_eq!(facts_of("~~~python\nx\n~~~\n").fence_tags, vec!["python"]);
    }
}
