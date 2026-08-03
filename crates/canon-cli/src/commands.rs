//! What each subcommand does.
//!
//! The hook commands all share a shape: read the snapshot, decide, return a
//! [`HookOutput`]. None of them derive anything except `session-start`, and
//! none of them write to a stream. Every one of them returns silence rather
//! than an error, because there is no error channel from a hook to a human.

use std::path::Path;

use canon_core::Settings;
use canon_derive::{
    FileEntry, Snapshot, duplicates_against_siblings, for_path, render_block, verify_source,
};
use canon_hook::{Event, HookInput, HookOutput};

use crate::{config, git, logging, paths};

/// Refresh the snapshot if it is stale, and state what the repository looks
/// like.
///
/// The only command that walks the tree. It runs once per session, where a
/// second of work is invisible, rather than before every write, where fifty
/// milliseconds is not.
pub(crate) fn session_start(input: &HookInput) -> HookOutput {
    let root = input.root();
    let (settings, problem) = config::load_or_default(&root);
    if let Some(e) = problem {
        logging::warn(&format!("config unusable, running on defaults: {e}"));
    }

    // Housekeeping on the cold path: a snapshot for a repository that no
    // longer exists, and a touched-file list from a session that was killed
    // before its turn ended, are both invisible and both permanent otherwise.
    let swept = paths::sweep_stale(now_unix());
    if swept > 0 {
        logging::info(&format!("swept {swept} stale files"));
    }

    let snapshot = refresh(&root, &settings, false);
    if snapshot.conventions.is_empty() {
        logging::info("no conventions derived; staying quiet");
        return HookOutput::silent();
    }
    HookOutput::context(Event::SessionStart, manifest(&snapshot))
}

/// State the same thing to a subagent.
///
/// The reason canon is a hook rather than a skill. A subagent starts with an
/// empty context window, so nothing in the conversation reaches it, and seven
/// parallel workers otherwise invent seven house styles. Reads the existing
/// snapshot and never derives: subagents spawn in bursts, and a tree walk per
/// worker would be felt.
pub(crate) fn subagent_start(input: &HookInput) -> HookOutput {
    let root = input.root();
    let Some(snapshot) = Snapshot::load(&paths::snapshot_path(&root)) else {
        return HookOutput::silent();
    };
    if snapshot.conventions.is_empty() {
        return HookOutput::silent();
    }
    HookOutput::context(Event::SubagentStart, manifest(&snapshot))
}

/// The conventions for the file about to be written.
///
/// The hot path. One file read, one filter, no parsing and no subprocess.
pub(crate) fn inject(input: &HookInput) -> HookOutput {
    let root = input.root();
    let Some(rel) = input.target_path().and_then(|p| relative_to(&root, p)) else {
        return HookOutput::silent();
    };
    let Some(snapshot) = Snapshot::load(&paths::snapshot_path(&root)) else {
        logging::debug("no snapshot yet; session-start has not finished");
        return HookOutput::silent();
    };
    let (settings, _) = config::load_or_default(&root);
    let selected = for_path(&snapshot.conventions, &rel, settings.injection_budget);
    let Some(block) = render_block(&rel, &selected) else { return HookOutput::silent() };
    logging::debug(&format!("injected {} conventions for {rel}", selected.len()));
    HookOutput::context(Event::PreToolUse, block)
}

/// Compare what was written against the conventions that applied.
///
/// Reads the file from disk rather than trusting the payload: for an `Edit`
/// the payload carries the replacement fragment, not the resulting file, and
/// checking a fragment against a whole-file rule reports nonsense.
pub(crate) fn verify(input: &HookInput) -> HookOutput {
    let root = input.root();
    let Some(target) = input.target_path() else { return HookOutput::silent() };
    let Some(rel) = relative_to(&root, target) else { return HookOutput::silent() };

    record_touch(&root, &input.session_id, &rel);

    let Some(snapshot) = Snapshot::load(&paths::snapshot_path(&root)) else {
        return HookOutput::silent();
    };
    let Ok(source) = std::fs::read_to_string(root.join(&rel)) else {
        return HookOutput::silent();
    };

    let violations = verify_source(&rel, &source, &snapshot.conventions);
    if violations.is_empty() {
        return HookOutput::silent();
    }

    let mut text = format!("{rel} differs from the conventions here:\n\n");
    for v in violations.iter().take(6) {
        text.push_str(&format!("- {}\n", v.message));
    }
    text.push_str("\nThese are derived by counting, not written down. Follow them unless the change is deliberate.\n");
    HookOutput::context(Event::PostToolUse, text)
}

/// Cross-file duplication over everything touched this turn.
///
/// At the end of the turn rather than per write, because the question is
/// whether the finished work duplicates something, and a file halfway through
/// an edit always looks like a partial copy of its neighbour.
pub(crate) fn reconcile(input: &HookInput) -> HookOutput {
    let root = input.root();
    let touched = take_touched(&root, &input.session_id);
    if touched.is_empty() {
        return HookOutput::silent();
    }
    let (settings, _) = config::load_or_default(&root);
    let index = index_files(&root, &settings);

    let mut lines = Vec::new();
    for rel in touched.iter().take(20) {
        let Ok(source) = std::fs::read_to_string(root.join(rel)) else { continue };
        for hit in duplicates_against_siblings(&root, rel, &source, &index).into_iter().take(2) {
            lines.push(format!("- {}: {}", rel, hit.render()));
        }
    }
    if lines.is_empty() {
        return HookOutput::silent();
    }

    let text = format!(
        "Possible duplication in what was just written:\n\n{}\n\nWorth a look before this lands. Shared structure is not always wrong.\n",
        lines.join("\n")
    );
    HookOutput::context(Event::Stop, text)
}

/// Everything a human needs to know about this install, from the binary
/// itself.
///
/// The answer to documentation drift. A capability table in a README is a
/// claim; this is the binary reporting what it actually links.
pub(crate) fn check(root: &Path) -> String {
    let mut out = String::new();
    out.push_str(&format!("canon {}\n\n", env!("CARGO_PKG_VERSION")));

    match config::load(root) {
        Ok(settings) => {
            out.push_str("Configuration\n");
            out.push_str(&format!("  injection_budget         {}\n", settings.injection_budget));
            out.push_str(&format!("  confidence_floor         {}\n", settings.confidence_floor));
            out.push_str(&format!("  min_files                {}\n", settings.min_files));
            out.push_str(&format!(
                "  recency_half_life_days   {}\n",
                settings.recency_half_life_days
            ));
            out.push_str(&format!("  log_level                {}\n", settings.log_level));
            if !settings.suppress.is_empty() {
                out.push_str(&format!(
                    "  suppress                 {}\n",
                    settings.suppress.join(", ")
                ));
            }
        }
        Err(e) => {
            out.push_str(&format!(
                "Configuration\n  INVALID: {e}\n  hooks are running on defaults\n"
            ));
        }
    }

    out.push_str("\nLanguages\n");
    for language in canon_extract::Language::ALL {
        let provider = canon_extract::lang::provider(*language);
        out.push_str(&format!(
            "  {:<12} {:<8} {}\n",
            language.name(),
            if provider.grammar_ready { "wired" } else { "tier 0" },
            provider.extensions.join(", ")
        ));
    }

    out.push_str("\nSnapshot\n");
    let path = paths::snapshot_path(root);
    match Snapshot::load(&path) {
        Some(s) => {
            out.push_str(&format!("  {}\n", s.summary()));
            out.push_str(&format!(
                "  commit                   {}\n",
                s.git_sha.as_deref().unwrap_or("not a git repository")
            ));
            out.push_str(&format!(
                "  fresh                    {}\n",
                s.is_fresh(git::head_sha(root).as_deref(), &config::load_or_default(root).0)
            ));
        }
        None => out.push_str("  no snapshot yet; run `canon index`\n"),
    }
    out.push_str(&format!("  path                     {}\n", path.display()));
    out
}

/// Rebuild the snapshot now.
pub(crate) fn index(root: &Path, rebuild: bool) -> String {
    let (settings, problem) = config::load_or_default(root);
    let mut out = String::new();
    if let Some(e) = problem {
        out.push_str(&format!("config unusable, running on defaults: {e}\n"));
    }
    out.push_str(&format!("{}\n", refresh(root, &settings, rebuild).summary()));
    out
}

/// Every convention for a path, or one by id, with the files behind it.
///
/// The audit surface. When canon derives something wrong, this is where the
/// evidence is visible and where a suppression can be written from.
pub(crate) fn explain(root: &Path, path: Option<&str>, id: Option<&str>) -> String {
    let Some(snapshot) = Snapshot::load(&paths::snapshot_path(root)) else {
        return "no snapshot yet; run `canon index`\n".to_string();
    };

    let matching: Vec<&canon_core::Convention> = snapshot
        .conventions
        .iter()
        .filter(|c| match (path, id) {
            (_, Some(wanted)) => c.id == wanted,
            (Some(p), None) => relevant_to(&c.scope, p),
            (None, None) => true,
        })
        .collect();

    if matching.is_empty() {
        return "no conventions match\n".to_string();
    }

    let mut out = String::new();
    for c in matching {
        out.push_str(&format!("{}\n", c.id));
        out.push_str(&format!("  {}\n", c.statement));
        out.push_str(&format!(
            "  scope       {}\n  agreement   {}/{} ({})\n  enforcement {:?}\n",
            c.scope.render(),
            c.agreeing,
            c.total,
            c.confidence.render(),
            c.enforcement
        ));
        if let Some(exemplar) = &c.exemplar {
            out.push_str(&format!("  example     {exemplar}\n"));
        }
        if !c.evidence.is_empty() {
            out.push_str("  evidence\n");
            for e in &c.evidence {
                out.push_str(&format!("    {}\n", e.rel));
            }
        }
        out.push('\n');
    }
    out
}

/// Whether a convention is worth showing when someone asks about a directory.
///
/// A looser question than [`canon_core::Scope::matches`], which answers "does
/// this rule govern this exact file". Here the argument is a directory, and
/// both directions are interesting: rules for `app/` apply inside
/// `app/services/`, and rules for `app/services/enrolments/` are what someone
/// asking about `app/services/` wants to see.
fn relevant_to(scope: &canon_core::Scope, query: &str) -> bool {
    let query = query.trim_end_matches('/');
    match scope {
        canon_core::Scope::Repo | canon_core::Scope::Ext(_) => true,
        canon_core::Scope::Dir(d) | canon_core::Scope::DirExt(d, _) => {
            d.is_empty()
                || query.is_empty()
                || d == query
                || d.starts_with(&format!("{query}/"))
                || query.starts_with(&format!("{d}/"))
        }
    }
}

/// Derive and persist, reusing a fresh snapshot unless told not to.
fn refresh(root: &Path, settings: &Settings, force: bool) -> Snapshot {
    let path = paths::snapshot_path(root);
    let sha = git::head_sha(root);

    if !force
        && let Some(existing) = Snapshot::load(&path)
        && existing.is_fresh(sha.as_deref(), settings)
    {
        logging::debug("snapshot still fresh");
        return existing;
    }

    let files = index_files(root, settings);
    let conventions = canon_derive::derive_from(root, settings, &files);
    let languages = languages_in(&files);
    let snapshot = Snapshot::new(sha, settings, files.len(), languages, conventions);
    // A snapshot that cannot be persisted is still usable for this call. The
    // next session pays to derive again, which is the right trade against
    // failing the hook over a full disk.
    if let Err(e) = snapshot.save(&path) {
        logging::error(&format!("could not write snapshot: {e}"));
    } else {
        logging::info(&format!("snapshot rebuilt: {}", snapshot.summary()));
    }
    snapshot
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The files canon considers, from git when there is a git.
///
/// Falls back to walking the filesystem for a plain directory. The fallback
/// leans on an exclude list, which is why it is the fallback: on a real
/// repository that list missed a cache directory holding 909,661 files.
fn index_files(root: &Path, settings: &Settings) -> Vec<FileEntry> {
    if let Some(tracked) = git::tracked_files(root) {
        logging::debug(&format!("{} files tracked by git", tracked.len()));
        return canon_derive::entries_for(root, settings, &tracked);
    }
    logging::debug("not a git repository; walking the filesystem");
    canon_derive::walk(root, settings)
}

fn languages_in(files: &[FileEntry]) -> Vec<String> {
    let mut seen: Vec<String> = files
        .iter()
        .filter_map(|f| canon_extract::lang::from_extension(&f.ext))
        .filter(|l| canon_extract::lang::provider(*l).grammar_ready)
        .map(|l| l.name().to_string())
        .collect();
    seen.sort_unstable();
    seen.dedup();
    seen
}

/// Evidence below which a rule is too small to be worth summarising.
///
/// A repository has many tiny rules about obscure extensions. They are useful
/// when that exact file is being written and noise in an orientation block.
const MANIFEST_MIN_EVIDENCE: usize = 20;

/// The short block injected at session start and into every subagent.
///
/// Short on purpose. The per-file block carries the detail; this exists so the
/// model knows the detail is coming and does not go inferring house style from
/// five files it happened to read.
fn manifest(snapshot: &Snapshot) -> String {
    // Best supported first. Taking whatever came first alphabetically surfaced
    // the three most trivial rules in the repository, which is the opposite of
    // a summary.
    let mut widest: Vec<&canon_core::Convention> =
        snapshot.conventions.iter().filter(|c| c.total >= MANIFEST_MIN_EVIDENCE).collect();
    // Shape before naming, then evidence. Sorting on evidence alone put "the
    // 2,912 files in public/ are named in snake_case" at the top: true, and
    // nothing a reader could not have got from a directory listing. Shape is
    // the part that cannot be seen without parsing.
    widest.sort_by(|a, b| {
        let rank = |c: &canon_core::Convention| usize::from(c.id.starts_with("shape."));
        rank(b).cmp(&rank(a)).then(b.total.cmp(&a.total)).then(a.id.cmp(&b.id))
    });
    widest.truncate(4);

    let mut out = format!(
        "canon: {} derived from this repository. The rules for each file are supplied before it is written, so do not infer house style from the files you happen to read.\n",
        snapshot.summary()
    );
    if !widest.is_empty() {
        out.push_str("\nThe most widely held, for orientation only:\n");
        for c in widest {
            // Scoped, unlike the per-file block. There the header names the
            // scope, so a rule can say "files here"; in a summary nothing has
            // established what "here" refers to, and two rules for different
            // extensions read as one rule contradicting itself.
            out.push_str(&format!(
                "- {}: {} ({}/{}, {})\n",
                c.scope.render(),
                c.statement.trim_end_matches('.'),
                c.agreeing,
                c.total,
                c.confidence.render()
            ));
        }
    }
    out
}

/// A host-supplied absolute path, as a repository-relative one.
///
/// Returns `None` for anything outside the repository, which is how a write to
/// `/etc/hosts` or to a sibling checkout is declined rather than matched
/// against the wrong repository's conventions.
fn relative_to(root: &Path, target: &str) -> Option<String> {
    let target = Path::new(target);
    let rel = target.strip_prefix(root).ok().or_else(|| {
        // Fall through canonicalised, for a root reached by a symlink.
        target.strip_prefix(root.canonicalize().ok()?).ok()
    })?;
    let text = rel.to_str()?;
    if text.is_empty() {
        return None;
    }
    Some(if std::path::MAIN_SEPARATOR == '/' {
        text.to_string()
    } else {
        text.replace(std::path::MAIN_SEPARATOR, "/")
    })
}

fn record_touch(root: &Path, session_id: &str, rel: &str) {
    use std::io::Write as _;

    let path = paths::touched_path(root, session_id);
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{rel}");
    }
}

/// Read and clear the touched list for this session.
fn take_touched(root: &Path, session_id: &str) -> Vec<String> {
    let path = paths::touched_path(root, session_id);
    let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
    let _ = std::fs::remove_file(&path);
    let mut seen: Vec<String> =
        text.lines().map(str::to_string).filter(|s| !s.is_empty()).collect();
    seen.sort_unstable();
    seen.dedup();
    seen
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("canon-cmd-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_path_inside_the_repository_becomes_relative() {
        let root = Path::new("/work/repo");
        assert_eq!(relative_to(root, "/work/repo/app/a.rb").as_deref(), Some("app/a.rb"));
    }

    #[test]
    fn a_path_outside_the_repository_is_declined() {
        // Otherwise a write to a sibling checkout is matched against the wrong
        // repository's conventions.
        let root = Path::new("/work/repo");
        assert_eq!(relative_to(root, "/etc/hosts"), None);
        assert_eq!(relative_to(root, "/work/other/app/a.rb"), None);
        assert_eq!(relative_to(root, "/work/repo"), None);
    }

    #[test]
    fn the_touched_list_round_trips_and_clears_itself() {
        let root = temp("touched");
        record_touch(&root, "s1", "app/a.rb");
        record_touch(&root, "s1", "app/b.rb");
        record_touch(&root, "s1", "app/a.rb");

        let first = take_touched(&root, "s1");
        assert_eq!(first, vec!["app/a.rb", "app/b.rb"], "duplicates collapse");
        assert!(take_touched(&root, "s1").is_empty(), "reading must clear");
    }

    #[test]
    fn two_sessions_do_not_share_a_touched_list() {
        let root = temp("touched-sessions");
        record_touch(&root, "s1", "app/a.rb");
        record_touch(&root, "s2", "app/b.rb");
        assert_eq!(take_touched(&root, "s1"), vec!["app/a.rb"]);
        assert_eq!(take_touched(&root, "s2"), vec!["app/b.rb"]);
    }

    #[test]
    fn inject_without_a_snapshot_is_silent_rather_than_an_error() {
        let root = temp("inject-cold");
        let input = HookInput {
            cwd: Some(root.display().to_string()),
            hook_event_name: "PreToolUse".into(),
            tool_input: canon_hook::ToolInput {
                file_path: Some(root.join("app/a.rb").display().to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(inject(&input).is_silent());
    }

    #[test]
    fn check_reports_the_language_table_from_the_binary() {
        let root = temp("check");
        let text = check(&root);
        assert!(text.contains("Ruby"));
        assert!(text.contains("TypeScript"));
        assert!(text.contains("Vue SFC"));
        assert!(text.contains("tier 0"), "an unwired language must say so");
        assert!(text.contains("no snapshot yet"));
    }

    #[test]
    fn check_reports_an_invalid_config_loudly() {
        // The one place a human is waiting, so the one place it should shout.
        let root = temp("check-bad");
        std::fs::write(root.join(config::CONFIG_FILE), "min_fils = 3\n").unwrap();
        let text = check(&root);
        assert!(text.contains("INVALID"), "got {text}");
        assert!(text.contains("running on defaults"));
    }

    #[test]
    fn explain_without_a_snapshot_says_so() {
        let root = temp("explain-cold");
        assert!(explain(&root, Some("app/"), None).contains("run `canon index`"));
    }

    #[test]
    fn an_explain_query_matches_ancestors_and_descendants_of_the_directory() {
        use canon_core::Scope;
        let deep = Scope::DirExt("app/services/enrolments".into(), "rb".into());
        let mid = Scope::DirExt("app/services".into(), "rb".into());
        let other = Scope::DirExt("app/models".into(), "rb".into());

        // Asking about a directory shows the rules above and below it.
        assert!(relevant_to(&mid, "app/services"));
        assert!(relevant_to(&deep, "app/services"), "descendants are interesting");
        assert!(relevant_to(&mid, "app/services/enrolments"), "ancestors govern it");
        assert!(!relevant_to(&other, "app/services"), "siblings are not");

        // A trailing slash is how people type directories.
        assert!(relevant_to(&mid, "app/services/"));

        // Repository-wide rules are always relevant.
        assert!(relevant_to(&Scope::Repo, "app/services"));
        assert!(relevant_to(&Scope::Ext("rb".into()), "app/services"));
    }

    #[test]
    fn a_prefix_that_is_not_a_path_boundary_does_not_match() {
        use canon_core::Scope;
        let scope = Scope::DirExt("app/service".into(), "rb".into());
        assert!(!relevant_to(&scope, "app/services"), "`service` must not capture `services`");
    }

    // The end-to-end cycle lives in `tests/cli.rs`, which runs the real
    // binary. Setting an environment variable in-process would need `unsafe`
    // under edition 2024, and the workspace forbids it; spawning the binary
    // with its own environment tests the shipped artifact rather than a
    // library call that resembles it.
}
