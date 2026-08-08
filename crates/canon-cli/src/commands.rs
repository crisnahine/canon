//! What each subcommand does.
//!
//! The hook commands all share a shape: read the snapshot, decide, return a
//! [`HookOutput`]. None of them derive anything except `session-start`, and
//! none of them write to a stream. Every one of them returns silence rather
//! than an error, because there is no error channel from a hook to a human.

use std::path::Path;

use canon_core::Settings;
use canon_derive::{
    FileEntry, Snapshot, blocking_violations, duplicates_against_siblings, for_path, render_block,
    verify_source,
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
    // Said to the user, not only to a log whose level defaults to off. The
    // fallback is every default plus enforcement off, so a config that stops
    // loading takes refusals and suppressions with it, and the previous
    // behaviour was to do all of that in silence. A setting range can narrow
    // between releases, which turns a file that loaded yesterday into this
    // path; the one thing it must not be is invisible.
    let unusable = problem.map(|e| {
        logging::warn(&format!("config unusable, running on defaults: {e}"));
        format!(
            "canon could not load .canon.toml and is running on defaults, with enforcement off, so nothing will refuse a write: {e}"
        )
    });

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
        // Said to the user, once, rather than only to a log whose level
        // defaults to off. A repository under the floor is the ordinary first
        // run for a new or small project, and measured across eight unrelated
        // repositories it is common: a 14-file repository derived nothing at
        // all. Silence there is canon behaving correctly and looking broken,
        // which for a tool someone just installed is the same thing. Saying how
        // many files were seen, and what the floor is, turns a dead install
        // into a tool that has started.
        let said = format!(
            "canon indexed {} files in {} and derived no conventions yet. A rule needs {} files agreeing inside one directory, so a small or new repository states nothing until it grows. Nothing is wrong and nothing needs configuring.",
            snapshot.file_count,
            root.display(),
            settings.min_files,
        );
        return match unusable {
            // The unusable-config message wins. It reports a file that stopped
            // parsing, which is actionable now, where this one is a status.
            Some(problem) => HookOutput::silent().with_system_message(problem),
            None => HookOutput::silent().with_system_message(said),
        };
    }
    let out = HookOutput::context(Event::SessionStart, manifest(&snapshot));
    match unusable {
        Some(said) => out.with_system_message(said),
        None => out,
    }
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
    let cwd = input.root();
    let Some(target) = input.target_path() else { return HookOutput::silent() };
    let Some((root, snapshot)) = snapshot_root(&cwd) else {
        logging::debug("no snapshot for this root or any ancestor");
        // Once per session, to the user rather than the model. Silence here is
        // indistinguishable from "nothing applies here", which is how this
        // failure stayed invisible; saying it on every write would be worse
        // than saying nothing.
        if first_miss(&cwd, &input.session_id) {
            return HookOutput::silent().with_system_message(format!(
                "canon has no index for {}. Run `canon index` there, or start the session at the directory you want indexed.",
                cwd.display()
            ));
        }
        return HookOutput::silent();
    };
    let Some(rel) = relative_to(&root, target, &cwd) else { return HookOutput::silent() };
    let (settings, _) = config::load_or_default(&root);
    let conventions = live_conventions(&snapshot, &settings);

    // Refusing the write is the only channel the model cannot decline. Every
    // other one was measured: context before the write steers it, a
    // `PostToolUse` block delivers a reason and the turn ends anyway, and a
    // `Stop` block genuinely prevents the turn ending and still does not
    // compel the edit. So a rule the repository holds without exception is
    // enforced here, before anything is written, and everything else advises.
    let resulting = resulting_file(&root, &rel, input);
    // A file the index would have skipped never voted on anything, so no rule
    // here was counted over it and none may refuse it. That is the same bound
    // the walk uses, applied to the file about to exist rather than to the
    // files that already do: a 630 KB tracked service object was refused on
    // every edit by rules it had never been allowed to break.
    let violations = if indexable(&root, &rel, resulting.as_deref()) {
        blocking_violations(&rel, resulting, &conventions, &settings)
    } else {
        Vec::new()
    };
    if !violations.is_empty() {
        logging::info(&format!("refused a write to {rel}: {} violations", violations.len()));
        return HookOutput::deny(refusal(&rel, &violations));
    }

    // Anything able to refuse a write is stated, whatever the budget says. The
    // budget drops the least specific rules first and no real path has ever
    // come close to it, but the two are decided independently, so a rule could
    // in principle refuse a write it was silently dropped from warning about.
    // That is the worst available shape for a refusal, and it costs one line to
    // make impossible rather than merely unlikely.
    let mut selected = for_path(&conventions, &rel, settings.injection_budget);
    for c in conventions.iter().filter(|c| {
        c.scope.matches(&rel) && c.enforcement_now(&settings) == canon_core::Enforcement::Blocking
    }) {
        if !selected.iter().any(|s| s.statement == c.statement) {
            selected.push(c);
        }
    }
    let Some(block) = render_block(&rel, &selected) else { return HookOutput::silent() };
    logging::debug(&format!("injected {} conventions for {rel}", selected.len()));
    HookOutput::context(Event::PreToolUse, block)
}

/// Read one of canon's own state files, if it is a file and a sane size.
///
/// The same guard the repository reads get. A touched-file ledger is a few
/// kilobytes; anything larger is not one canon wrote.
fn read_state(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 4 * 1024 * 1024 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Whether the file this write produces is one the index would have kept.
///
/// Both directions matter. The content about to be written may be over the
/// bound, and the file already on disk may be, and either way it never
/// contributed a vote to any rule in the snapshot.
fn indexable(root: &Path, rel: &str, resulting: Option<&str>) -> bool {
    if resulting.is_some_and(|c| c.len() as u64 > canon_derive::MAX_FILE_BYTES) {
        return false;
    }
    match std::fs::symlink_metadata(root.join(rel)) {
        Ok(meta) => meta.is_file() && meta.len() <= canon_derive::MAX_FILE_BYTES,
        // Not there yet is the ordinary case for a new file.
        Err(_) => true,
    }
}

/// The file as it will exist once this tool call runs, when that is knowable.
///
/// `Write` carries the whole file, so it is the whole file. `Edit` carries a
/// fragment, and checking a whole-file rule against a fragment reports that a
/// class has no base type because the fragment does not contain the class — the
/// same reason `verify` re-reads from disk instead of trusting the payload. So
/// the result is reconstructed: read what is there and apply the replacement.
///
/// Without this, enforcement depended on which tool the model reached for
/// rather than on what landed on disk. Measured on a directory with a rule at
/// total agreement: `Write` of the file was refused, and an `Edit` producing
/// byte-identical content was accepted. `Edit` is also the tool a model uses
/// most once a file exists, so the guarded path was the rarer one.
///
/// `None` when the result cannot be known — a `NotebookEdit`, or an `Edit`
/// against a file that is not on disk. Naming rules still apply then, because
/// they read the path and never the content.
fn resulting_file(root: &Path, rel: &str, input: &HookInput) -> Option<String> {
    if let Some(content) = input.tool_input.content.clone() {
        return Some(content);
    }
    let (old, new) =
        (input.tool_input.old_string.as_deref()?, input.tool_input.new_string.as_deref()?);
    let current = canon_derive::read_indexable(&root.join(rel))?;
    // An `old_string` that is not there means the edit will not apply, and
    // guessing at the result would refuse a write that was never going to
    // happen in the shape we imagined.
    if !current.contains(old) {
        return None;
    }
    Some(if input.tool_input.replace_all {
        current.replace(old, new)
    } else {
        current.replacen(old, new, 1)
    })
}

/// Why the write was refused, and what would satisfy the rule.
///
/// The counts are load-bearing. A refusal without them reads as an arbitrary
/// gate; with them it reads as a fact about the repository the author can
/// check, and disagree with, in one command.
///
/// So are the ids. The refusal points at `.canon.toml` as the way out, and
/// `suppress` is keyed by id, so a refusal that names the rule only in prose
/// leaves the reader guessing at the key. Measured against the running host:
/// asked for a file that broke three rules, the model was refused, inferred
/// three plausible ids, wrote them into `.canon.toml`, and was refused again,
/// because the real ids carried the directory the rules were derived at.
/// The suppression block is therefore written out, ready to paste.
fn refusal(rel: &str, violations: &[canon_derive::Violation]) -> String {
    // "every file in this directory" was a claim the counts often contradicted:
    // a repository-wide `.txt` rule reported 8/8 while refusing a write into a
    // directory holding two files, neither of which followed it. Each line now
    // renders the scope it was counted over, so the sentence and the number
    // describe the same set.
    let mut text = format!(
        "canon refused this write to {rel}. Each of these is a rule that every file it was counted over follows, without exception:\n\n"
    );
    let shown: Vec<&canon_derive::Violation> = violations.iter().take(3).collect();
    for v in &shown {
        text.push_str(&format!("- {} [{}]\n", v.message, v.convention_id));
    }
    text.push_str("\nRewrite the file to match, or, if the rule is wrong, put this in .canon.toml at the repository root:\n\n");
    let ids: Vec<String> = shown.iter().map(|v| format!("\"{}\"", v.convention_id)).collect();
    text.push_str(&format!("    suppress = [{}]\n", ids.join(", ")));
    text.push_str(
        "\n`canon explain <path>` shows the files each rule was counted from. Suppression takes effect on the next write, not the next session.\n",
    );
    text
}

/// Compare what was written against the conventions that applied.
///
/// Reads the file from disk rather than trusting the payload: for an `Edit`
/// the payload carries the replacement fragment, not the resulting file, and
/// checking a fragment against a whole-file rule reports nonsense.
pub(crate) fn verify(input: &HookInput) -> HookOutput {
    let cwd = input.root();
    let Some(target) = input.target_path() else { return HookOutput::silent() };
    // Resolved before the ledger is written. Recording first left ledgers
    // accumulating for roots that have no snapshot and nothing ever reads.
    let Some((root, snapshot)) = snapshot_root(&cwd) else { return HookOutput::silent() };
    let Some(rel) = relative_to(&root, target, &cwd) else { return HookOutput::silent() };

    record_touch(&root, &input.session_id, &rel);

    // Guarded the same way the write path is. A FIFO at the target path never
    // returns, and this read had none of the checks its neighbour got.
    let Some(source) = canon_derive::read_indexable(&root.join(&rel)) else {
        return HookOutput::silent();
    };

    let (settings, _) = config::load_or_default(&root);
    let conventions = live_conventions(&snapshot, &settings);
    let mut violations = verify_source(&rel, &source, &conventions);
    // Asked last and separately, because it is the one check that looks for a
    // file rather than at one, and the file it looks for does not exist yet.
    violations.extend(canon_derive::missing_test(&root, &rel, &conventions, &settings));
    if violations.is_empty() {
        return HookOutput::silent();
    }

    let mut text = format!("{rel} differs from the conventions here:\n\n");
    for v in violations.iter().take(6) {
        text.push_str(&format!("- {}\n", v.message));
    }
    // Asking for the change moves compliance more than restating the rule
    // does, and costs nothing. The previous wording described a policy.
    text.push_str(
        "\nUpdate the file to match, unless the difference is deliberate. These are counted from the code, not written down; `canon explain` shows the files behind each one.\n",
    );
    HookOutput::context(Event::PostToolUse, text)
}

/// Cross-file duplication over everything touched this turn.
///
/// At the end of the turn rather than per write, because the question is
/// whether the finished work duplicates something, and a file halfway through
/// an edit always looks like a partial copy of its neighbour.
pub(crate) fn reconcile(input: &HookInput) -> HookOutput {
    let cwd = input.root();
    let root = snapshot_root(&cwd).map_or(cwd, |(r, _)| r);
    let touched = take_touched(&root, &input.session_id);
    if touched.is_empty() {
        return HookOutput::silent();
    }
    let (settings, _) = config::load_or_default(&root);

    // The index comes from git, so everything written this turn is untracked
    // and therefore invisible to it. Four similar services written in one turn
    // is exactly the case worth catching, and it was the one case that could
    // not be seen; copying an already-tracked file was the only one that could.
    let mut index = index_files(&root, &settings, History::Recent);
    let known: std::collections::HashSet<&str> = index.iter().map(|f| f.rel.as_str()).collect();
    let fresh: Vec<String> =
        touched.iter().filter(|r| !known.contains(r.as_str())).cloned().collect();
    if !fresh.is_empty() {
        // `fresh` is exactly the touched paths git does not know about yet, so
        // no commit time in the tree could apply to any of them; asking git
        // again here would only pay for a lookup that always misses.
        let commit_times = std::collections::HashMap::new();
        let entries = canon_derive::entries_for(&root, &settings, &fresh, &commit_times);
        index.extend(entries);
    }

    let mut lines = Vec::new();
    for rel in touched.iter().take(20) {
        let Some(source) = canon_derive::read_indexable(&root.join(rel)) else { continue };
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
            // Printed because a reader has to be able to tell whether canon
            // can refuse a write, and the answer is not visible anywhere else.
            out.push_str(&format!(
                "  enforce                  {}{}\n",
                settings.enforce,
                if settings.enforce {
                    "  (rules with total agreement may refuse a write)"
                } else {
                    "  (advisory only)"
                }
            ));
            if !settings.suppress.is_empty() {
                out.push_str(&format!(
                    "  suppress                 {}\n",
                    settings.suppress.join(", ")
                ));
            }
        }
        Err(e) => {
            // Not "the defaults": the default is `enforce = true`, and a
            // config that will not parse deliberately runs with enforcement
            // off. Saying "defaults" here would tell someone their write can
            // still be refused when it cannot.
            out.push_str(&format!(
                "Configuration\n  INVALID: {e}\n  hooks are running on defaults, with enforce = false until this parses\n"
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
    // Printed because the hooks and a terminal used to resolve different
    // directories, and there was no way to tell from the output which one you
    // were looking at.
    out.push_str(&format!("  data directory           {}\n", paths::data_dir().display()));
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

    let query = path.map(|p| normalise_query(root, p)).unwrap_or_default();
    let names_file = path.is_some() && names_a_file(&query, &snapshot);
    let mut matching: Vec<&canon_core::Convention> = snapshot
        .conventions
        .iter()
        .filter(|c| match (path, id) {
            (_, Some(wanted)) => c.id == wanted,
            (Some(_), None) => relevant_to(
                &c.scope,
                &query,
                &c.evidence,
                names_file,
                !names_file || canon_derive::offered_for_path(c, &query),
            ),
            (None, None) => true,
        })
        .collect();

    if matching.is_empty() {
        return "no conventions match\n".to_string();
    }

    // Enforcement as it would be applied to the next write, not as it was
    // recorded when the snapshot was built. The two differ the moment someone
    // edits `.canon.toml`, and this is the surface a refusal sends them to.
    let (settings, _) = config::load_or_default(root);

    // Anything that can refuse a write comes first. Someone reading this was
    // sent here by a refusal and has one question: which of these stopped me.
    matching.sort_by_key(|c| {
        let blocking = c.enforcement_now(&settings) == canon_core::Enforcement::Blocking
            && !settings.is_suppressed(&c.id);
        (!blocking, std::cmp::Reverse(c.scope.specificity()), c.id.clone())
    });

    let mut out = String::new();
    for c in matching {
        out.push_str(&format!("{}\n", c.id));
        out.push_str(&format!("  {}\n", c.statement));
        // "Suppressed" rather than "Advisory". A suppressed rule is not
        // downgraded, it is gone: neither injected nor checked. This is the
        // surface a refusal sends someone to in order to confirm the
        // suppression they just wrote took effect, and `Advisory` reads as
        // though it were still being stated.
        let enforcement = if settings.is_suppressed(&c.id) {
            "Suppressed".to_string()
        } else {
            format!("{:?}", c.enforcement_now(&settings))
        };
        out.push_str(&format!(
            "  scope       {}\n  agreement   {}/{} ({})\n  enforcement {enforcement}\n",
            c.scope.render(),
            c.agreeing,
            c.total,
            c.confidence.render(),
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
fn relevant_to(
    scope: &canon_core::Scope,
    query: &str,
    evidence: &[canon_core::Evidence],
    names_file: bool,
    speaks_here: bool,
) -> bool {
    let query = query.trim_end_matches('/');
    if query.is_empty() {
        return true;
    }
    // A file has exactly one right answer: the same filter injection uses.
    // Asking about a `.rake` file and being shown the rule for `**/*.csv` is
    // the whole of issue #15, and it lands on someone who was just refused and
    // told to run this.
    if names_file {
        // `speaks_here` carries the one filter a scope cannot express: a rule
        // that speaks for its own directory and no other. Without it this
        // surface named a namespace the injected block deliberately withholds
        // and the checker refuses to judge on — on the page a reader is sent
        // to after being told to run it.
        return speaks_here && scope.matches(query);
    }
    match scope {
        canon_core::Scope::Repo => true,
        // A repository-wide extension rule is about this directory only if it
        // was counted over something in it. The evidence is a sample rather
        // than the whole set, so this can understate — which is the right way
        // for an audit surface to be wrong.
        canon_core::Scope::Ext(ext) => evidence
            .iter()
            .any(|e| e.rel.starts_with(&format!("{query}/")) && has_extension(&e.rel, ext)),
        // A rule counted over one directory's own files is relevant to that
        // directory and to anyone asking about something above it. It is not
        // relevant downwards, unlike the prefix scopes: it says nothing about a
        // subdirectory, and an audit surface that showed it there would name a
        // rule the injected block withholds.
        canon_core::Scope::DirChildrenExt(d, _) => {
            d == query || d.starts_with(&format!("{query}/"))
        }
        canon_core::Scope::Dir(d) | canon_core::Scope::DirExt(d, _) => {
            d.is_empty()
                || d == query
                || d.starts_with(&format!("{query}/"))
                || query.starts_with(&format!("{d}/"))
        }
    }
}

/// Whether a query names a file rather than a directory.
///
/// Guessing from "the last segment has a dot" is wrong twice over: `.github`,
/// `.circleci` and `.storybook` are directories whose only dot is the first
/// character, and `api.v2` or `src/v1.2` are directories with a dot in the
/// middle. Both answered "no conventions match", which is the least helpful
/// thing this command can say.
///
/// So the snapshot decides where it can. A path that something in the index
/// sits *under* is a directory, whatever its punctuation. Only when the index
/// is silent does the shape of the name break the tie, and then a leading dot
/// does not count as an extension.
fn names_a_file(query: &str, snapshot: &Snapshot) -> bool {
    let prefix = format!("{query}/");
    let holds_files =
        snapshot.conventions.iter().any(|c| c.evidence.iter().any(|e| e.rel.starts_with(&prefix)));
    if holds_files {
        return false;
    }
    let name = query.rsplit_once('/').map_or(query, |(_, n)| n);
    name.trim_start_matches('.').contains('.')
}

/// A query as the snapshot spells paths: repository-relative, no `./`, no `..`.
///
/// `explain` compared the raw string, so `./app/services` and an absolute path
/// both missed every directory-scoped rule while the identical relative query
/// matched. The hook path has done this since it started resolving targets;
/// this is the same normalisation, one command later than it should have been.
fn normalise_query(root: &Path, query: &str) -> String {
    let trimmed = query.trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    let candidate = Path::new(trimmed);
    let absolute =
        if candidate.is_absolute() { candidate.to_path_buf() } else { root.join(candidate) };
    let normal = lexically_normal(&absolute);

    let relative = normal.strip_prefix(root).map(Path::to_path_buf).ok().or_else(|| {
        // Same fallback the hook path uses, for a root reached by a symlink.
        let real = root.canonicalize().ok()?;
        normal.strip_prefix(real).map(Path::to_path_buf).ok()
    });
    // A path outside the repository is left as typed rather than mangled into
    // something that would match the wrong rules.
    relative.as_deref().and_then(Path::to_str).map_or_else(
        || with_forward_slashes(trimmed.trim_start_matches("./")),
        with_forward_slashes,
    )
}

fn has_extension(rel: &str, ext: &str) -> bool {
    rel.rsplit_once('.').is_some_and(|(_, e)| e.eq_ignore_ascii_case(ext))
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

    let files = index_files(root, settings, History::Full);
    let conventions = canon_derive::derive_from(root, settings, &files);
    let languages = languages_in(&conventions);
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

/// Whether this is the first time this session has missed for this root.
///
/// A marker in the session directory, which is already swept, so the state
/// costs nothing to keep and nothing to clean up. Failing to write it means
/// the message repeats rather than being lost, which is the right way round.
fn first_miss(root: &Path, session_id: &str) -> bool {
    let marker = paths::touched_path(root, session_id).with_extension("missed");
    if marker.exists() {
        return false;
    }
    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&marker, "").is_ok()
}

/// The root a snapshot exists for, searching upward from `start`.
///
/// A session indexes the directory it started in. Anything that later moves the
/// working directory deeper — a nested repository, a shell that keeps its
/// directory between calls — makes `input.root()` resolve somewhere that was
/// never indexed, and every hook goes quiet for the rest of the session. Silence
/// is also what canon returns when nothing applies, so the failure is invisible
/// from inside the session.
///
/// Walking up finds the root the session actually indexed. Bounded, because an
/// unbounded walk would eventually find a snapshot for an unrelated ancestor.
fn snapshot_root(start: &Path) -> Option<(std::path::PathBuf, Snapshot)> {
    const MAX_ASCENT: usize = 6;
    let mut candidate = start.to_path_buf();
    for _ in 0..=MAX_ASCENT {
        if let Some(snapshot) = Snapshot::load(&paths::snapshot_path(&candidate)) {
            return Some((candidate, snapshot));
        }
        if !candidate.pop() {
            break;
        }
    }
    None
}

/// How much history an index reads commit times from.
///
/// [`refresh`] derives every convention, and a file's commit time becomes the
/// recency weight behind each vote and the exemplar the block points at, so it
/// reads the whole log — once per snapshot, on the cold path.
///
/// [`reconcile`] runs at the end of every turn that touched a file, and does
/// one thing with the answer: order a directory's files before truncating them
/// to a shortlist of siblings. Paying an unbounded history walk for that
/// ordering is the wrong trade, and it was bounded only by a twenty-second
/// timeout. A file older than the cap is simply absent and falls back to its
/// mtime, which is what every file did before commit times existed — and the
/// files an ordering by recency actually promotes are all inside the cap.
#[derive(Clone, Copy)]
enum History {
    Full,
    Recent,
}

/// The files canon considers, from git when there is a git.
///
/// Falls back to walking the filesystem for a plain directory. The fallback
/// leans on an exclude list, which is why it is the fallback: on a real
/// repository that list missed a cache directory holding 909,661 files.
fn index_files(root: &Path, settings: &Settings, history: History) -> Vec<FileEntry> {
    if let Some(tracked) = git::tracked_files(root) {
        logging::debug(&format!("{} files tracked by git", tracked.len()));
        // `unwrap_or_default` rather than propagating `None`: a repository too
        // large to walk in time, or one git can't answer for at all, still has
        // its tracked files and their mtimes, so it degrades to those instead
        // of losing the index entirely.
        let commit_times = match history {
            History::Full => git::commit_times(root),
            History::Recent => git::recent_commit_times(root),
        }
        .unwrap_or_default();
        return canon_derive::entries_for(root, settings, &tracked, &commit_times);
    }
    logging::debug("not a git repository; walking the filesystem");
    canon_derive::walk(root, settings)
}

/// The languages the conventions in hand actually came from.
///
/// Read off the scopes rather than off the files walked. Counting every wired
/// language with a file in the tree credited ERB, PHP and Python on a workspace
/// where all three derived nothing: the header said conventions were "derived
/// from" languages that had contributed none, which is the one claim a header
/// like that is making.
fn languages_in(conventions: &[canon_core::Convention]) -> Vec<String> {
    let mut seen: Vec<String> = conventions
        .iter()
        .filter_map(|c| match &c.scope {
            canon_core::Scope::Ext(ext)
            | canon_core::Scope::DirExt(_, ext)
            | canon_core::Scope::DirChildrenExt(_, ext) => Some(ext),
            canon_core::Scope::Repo | canon_core::Scope::Dir(_) => None,
        })
        .filter_map(|ext| canon_extract::lang::from_extension(ext))
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

/// The repository-relative path a tool call targets.
///
/// Returns `None` for anything outside the repository, which is how a write to
/// `/etc/hosts` or to a sibling checkout is declined rather than matched
/// against the wrong repository's conventions.
///
/// Two things this has to do beyond stripping a prefix.
///
/// A relative `file_path` is resolved against the invocation's working
/// directory. The host normally sends an absolute path; when it does not,
/// returning nothing meant the write got no conventions and no enforcement,
/// silently.
///
/// And `..` is resolved before matching. Left in, `app/services/../../vendor/x.rb`
/// still starts with `app/`, so it matched `app/**/*.rb` and would have had a
/// service object's rules applied to a vendored file.
fn relative_to(root: &Path, target: &str, cwd: &Path) -> Option<String> {
    let target = Path::new(target);
    let absolute = if target.is_absolute() { target.to_path_buf() } else { cwd.join(target) };
    let absolute = lexically_normal(&absolute);

    let rel = absolute.strip_prefix(root).ok().map(Path::to_path_buf).or_else(|| {
        // Fall through canonicalised, for a root reached by a symlink.
        let real = root.canonicalize().ok()?;
        absolute.strip_prefix(real).ok().map(Path::to_path_buf)
    })?;
    let text = rel.to_str()?;
    if text.is_empty() {
        return None;
    }
    Some(with_forward_slashes(text))
}

/// A path as the snapshot spells one: forward slashes on every platform.
///
/// Scopes, evidence and `sample_roots` are all stored with forward slashes,
/// because that is what `git ls-files` reports. A path that came back through
/// `Path` carries the platform separator, and on Windows comparing
/// `app\services` against the stored `app/services` matches nothing —
/// `canon explain app/services` answered "no conventions match", which is the
/// audit surface a refusal sends people to. Both callers go through here now,
/// so the next one cannot forget it.
fn with_forward_slashes(text: &str) -> String {
    if std::path::MAIN_SEPARATOR == '/' {
        text.to_string()
    } else {
        text.replace(std::path::MAIN_SEPARATOR, "/")
    }
}

/// Resolve `.` and `..` without touching the filesystem.
///
/// Lexical rather than `canonicalize`, because the target of a write does not
/// exist yet and `canonicalize` fails on a path that is not there.
fn lexically_normal(path: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(Component::ParentDir);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// The conventions in force for this invocation.
///
/// A snapshot records the enforcement decision made when it was built, and
/// suppression is applied at derivation. Both meant a `.canon.toml` written in
/// response to a refusal did nothing until the next session rebuilt the
/// snapshot, which is the one moment it has to work. Suppressed rules are
/// dropped here so they are neither injected nor enforced; `blocking_violations`
/// recomputes enforcement from the same settings.
///
/// Borrowed when nothing is suppressed, which is almost always, so the hot
/// path still does not copy a few hundred conventions.
fn live_conventions<'s>(
    snapshot: &'s Snapshot,
    settings: &Settings,
) -> std::borrow::Cow<'s, [canon_core::Convention]> {
    if settings.suppress.is_empty() {
        return std::borrow::Cow::Borrowed(&snapshot.conventions);
    }
    std::borrow::Cow::Owned(
        snapshot
            .conventions
            .iter()
            .filter(|c| !settings.is_suppressed(&c.id))
            .cloned()
            .collect::<Vec<_>>(),
    )
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
    // canon's own state directory is still a directory someone can put a FIFO
    // in, and `reconcile` blocks forever on one.
    let Some(text) = read_state(&path) else { return Vec::new() };
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
        assert_eq!(relative_to(root, "/work/repo/app/a.rb", root).as_deref(), Some("app/a.rb"));
    }

    #[test]
    fn a_path_outside_the_repository_is_declined() {
        // Otherwise a write to a sibling checkout is matched against the wrong
        // repository's conventions.
        let root = Path::new("/work/repo");
        assert_eq!(relative_to(root, "/etc/hosts", root), None);
        assert_eq!(relative_to(root, "/work/other/app/a.rb", root), None);
        assert_eq!(relative_to(root, "/work/repo", root), None);
    }

    #[test]
    fn a_relative_path_is_resolved_against_the_working_directory() {
        // The host normally sends an absolute path. When it does not, silence
        // cost the write both its conventions and its enforcement.
        let root = Path::new("/work/repo");
        let cwd = Path::new("/work/repo/app");
        assert_eq!(relative_to(root, "services/a.rb", cwd).as_deref(), Some("app/services/a.rb"));
        assert_eq!(relative_to(root, "./a.rb", cwd).as_deref(), Some("app/a.rb"));
    }

    #[test]
    fn a_parent_traversal_is_resolved_before_the_scope_is_matched() {
        // `app/services/../../vendor/x.rb` still starts with `app/`, so it
        // matched `app/**/*.rb` and would have been judged as a service.
        let root = Path::new("/work/repo");
        assert_eq!(
            relative_to(root, "/work/repo/app/services/../../vendor/x.rb", root).as_deref(),
            Some("vendor/x.rb")
        );
        assert_eq!(relative_to(root, "/work/repo/../other/x.rb", root), None);
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
    fn a_root_with_no_snapshot_is_reported_once_and_then_stays_quiet() {
        // Issue #3: silence for "no snapshot for this root" is
        // indistinguishable from silence for "nothing applies here", so the
        // failure was invisible from inside the session. Saying it on every
        // write would be worse than saying nothing.
        let root = temp("first-miss");
        // The marker lives in the data directory, keyed by root and session,
        // so it outlives the temporary root and would leak between runs.
        for session in ["s-once", "s-other"] {
            let _ =
                std::fs::remove_file(paths::touched_path(&root, session).with_extension("missed"));
        }
        assert!(first_miss(&root, "s-once"), "the first miss must report");
        assert!(!first_miss(&root, "s-once"), "the second must not");
        assert!(first_miss(&root, "s-other"), "a different session reports again");
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
        // The first miss carries a message for the user; every later one is
        // silent. Prime it so this asserts the steady state.
        let _ = first_miss(&root, "");
        let out = inject(&input);
        assert!(out.is_silent(), "a repeat miss must be silent");
    }

    #[test]
    fn check_reports_the_language_table_from_the_binary() {
        let root = temp("check");
        let text = check(&root);
        assert!(text.contains("Ruby"));
        assert!(text.contains("TypeScript"));
        assert!(text.contains("Vue SFC"));
        assert!(text.contains("ERB"), "every language canon knows must be listed");
        // Nothing is tier 0 any more: Vue and ERB are two grammars in one file
        // and are parsed through included ranges rather than declared
        // unsupported. If a language is ever added without a grammar, the
        // table has to keep saying so.
        for language in canon_extract::Language::ALL {
            let provider = canon_extract::lang::provider(*language);
            let expected = if provider.grammar_ready { "wired" } else { "tier 0" };
            assert!(
                text.contains(language.name()),
                "{} is missing from the table",
                language.name()
            );
            assert!(text.contains(expected), "{} is mislabelled", language.name());
        }
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
        assert!(relevant_to(&mid, "app/services", &[], false, true));
        assert!(
            relevant_to(&deep, "app/services", &[], false, true),
            "descendants are interesting"
        );
        assert!(
            relevant_to(&mid, "app/services/enrolments", &[], false, true),
            "ancestors govern it"
        );
        assert!(!relevant_to(&other, "app/services", &[], false, true), "siblings are not");

        // A trailing slash is how people type directories.
        assert!(relevant_to(&mid, "app/services/", &[], false, true));

        // The repository-wide rule governs every directory.
        assert!(relevant_to(&Scope::Repo, "app/services", &[], false, true));

        // An extension rule governs a directory only if it was counted over
        // something in it. Answering "yes, always" is how asking about a
        // `.rake` file listed the rule for `**/*.csv`.
        let ext = Scope::Ext("rb".into());
        let here = [ev("app/services/charge_card.rb")];
        let elsewhere = [ev("lib/tasks/backfill.rb")];
        assert!(relevant_to(&ext, "app/services", &here, false, true));
        assert!(!relevant_to(&ext, "app/services", &elsewhere, false, true));
        assert!(!relevant_to(&ext, "app/services", &[], false, true));
    }

    fn ev(rel: &str) -> canon_core::Evidence {
        canon_core::Evidence { rel: rel.to_string(), line: 0 }
    }

    fn rule(id: &str, scope: canon_core::Scope) -> canon_core::Convention {
        canon_core::Convention {
            id: id.into(),
            statement: "Files here are named in snake_case".into(),
            scope,
            confidence: canon_core::Confidence::derive(9, 10).expect("valid"),
            agreeing: 9,
            total: 10,
            exemplar: None,
            evidence: vec![],
            sample_roots: vec![],
            enforcement: canon_core::Enforcement::Advisory,
        }
    }

    #[test]
    fn explain_withholds_from_a_file_exactly_what_injection_withholds() {
        use canon_core::Scope;
        // Someone stopped by a refusal runs this to find the rule that stopped
        // them. Listing a rule the injected block withheld, and the checker
        // refuses to judge on, sends them to a sentence that governs nothing.
        let suffix = rule("tests.suffix.rb", Scope::Ext("rb".into()));
        assert!(
            !canon_derive::offered_for_path(&suffix, "app/services/charge_card.rb"),
            "how tests are named is not about a file that is not a test"
        );
        assert!(canon_derive::offered_for_path(&suffix, "spec/services/charge_card_spec.rb"));

        let parent = rule(
            "shape.namespace.src.Services.Billing.php",
            Scope::DirExt("src/Services/Billing".into(), "php".into()),
        );
        assert!(
            !canon_derive::offered_for_path(&parent, "src/Services/Billing/Invoices/Void.php"),
            "a namespace rule speaks for one directory"
        );
        assert!(canon_derive::offered_for_path(&parent, "src/Services/Billing/Charge.php"));

        // Everything else a scope matches is still offered.
        let naming = rule("naming.src.php", Scope::DirExt("src".into(), "php".into()));
        assert!(canon_derive::offered_for_path(&naming, "src/Services/Billing/Invoices/Void.php"));
    }

    #[test]
    fn a_normalised_query_is_spelled_the_way_the_snapshot_spells_paths() {
        // Windows-only in effect, and invisible on a Unix runner unless the
        // assertion is about the separator rather than about the literal. The
        // separator step was written into `relative_to` and left out of
        // `normalise_query`, so `canon explain app/services` matched nothing on
        // Windows — the audit surface a refusal points at.
        let root = Path::new("/work/repo");
        for query in ["app/services", "./app/services", "app/services/", "/work/repo/app/services"]
        {
            let got = normalise_query(root, query);
            assert!(
                !got.contains(std::path::MAIN_SEPARATOR) || std::path::MAIN_SEPARATOR == '/',
                "{query} kept a platform separator: {got}"
            );
        }
        assert_eq!(normalise_query(root, "./app/services"), "app/services");
        assert_eq!(normalise_query(root, "app/services/"), "app/services");
        // A path outside the repository is left as typed, and still normalised.
        assert!(!normalise_query(root, "/elsewhere/x").is_empty());
    }

    #[test]
    fn a_relative_path_and_a_query_agree_on_how_a_path_is_spelled() {
        // The two used to normalise separately and one of them forgot a step.
        let root = Path::new("/work/repo");
        assert_eq!(
            relative_to(root, "/work/repo/app/services/a.rb", root).as_deref(),
            Some(normalise_query(root, "app/services/a.rb").as_str())
        );
    }

    #[test]
    fn an_explain_query_naming_a_file_answers_only_for_that_file() {
        use canon_core::Scope;
        // The same predicate injection uses. Someone stopped by a refusal runs
        // this to find the rule that stopped them, and the whole snapshot is
        // not an answer.
        let rake = "lib/tasks/backfill.rake";
        assert!(relevant_to(&Scope::Ext("rake".into()), rake, &[], true, true));
        assert!(!relevant_to(&Scope::Ext("csv".into()), rake, &[ev("data/a.csv")], true, true));
        assert!(!relevant_to(&Scope::DirExt("app".into(), "rb".into()), rake, &[], true, true));
        assert!(relevant_to(
            &Scope::DirExt("lib/tasks".into(), "rake".into()),
            rake,
            &[],
            true,
            true
        ));
        assert!(relevant_to(&Scope::Repo, rake, &[], true, true));
    }

    #[test]
    fn a_prefix_that_is_not_a_path_boundary_does_not_match() {
        use canon_core::Scope;
        let scope = Scope::DirExt("app/service".into(), "rb".into());
        assert!(
            !relevant_to(&scope, "app/services", &[], false, true),
            "`service` must not capture `services`"
        );
    }

    // The end-to-end cycle lives in `tests/cli.rs`, which runs the real
    // binary. Setting an environment variable in-process would need `unsafe`
    // under edition 2024, and the workspace forbids it; spawning the binary
    // with its own environment tests the shipped artifact rather than a
    // library call that resembles it.
}
