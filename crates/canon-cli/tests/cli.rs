//! End-to-end tests against the built binary.
//!
//! These spawn the real `canon` executable and speak the real protocol over a
//! pipe. Everything below the binary is unit-tested in its own crate; what is
//! left to check is the part no library test can see: that the shipped
//! artifact reads stdin, writes one JSON document, and exits zero.
//!
//! Payloads are built with `serde_json`, never by interpolating strings. A
//! Windows path is `C:\Users\...`, and `\U` is not a valid JSON escape, so a
//! hand-rolled payload is malformed on one platform and fine on the other two.
//! canon then fails open exactly as designed and every assertion here reads as
//! a product bug. It cost a red Windows job to learn.

// `indexing_slicing` is denied in shipped code, where a panic inside a hook is
// the one failure the fail-open harness cannot hide. In a test a panic is the
// reporting mechanism, and `parsed["a"]["b"]` is how serde_json is read.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// The binary under test, as cargo built it for this run.
fn binary() -> PathBuf {
    let mut path = std::env::current_exe().expect("test binary path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join(format!("canon{}", std::env::consts::EXE_SUFFIX))
}

struct Fixture {
    root: PathBuf,
    data: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!("canon-e2e-{name}"));
        let _ = std::fs::remove_dir_all(&base);
        let fixture = Self { root: base.join("repo"), data: base.join("data") };
        std::fs::create_dir_all(&fixture.root).unwrap();
        std::fs::create_dir_all(&fixture.data).unwrap();
        fixture
    }

    fn write(&self, rel: &str, body: &str) -> &Self {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
        self
    }

    /// A tool payload targeting `rel`, with the host's own field names.
    fn tool_payload(&self, session: &str, event: &str, rel: &str, content: Option<&str>) -> String {
        let mut tool_input = json!({ "file_path": self.root.join(rel) });
        if let Some(text) = content {
            tool_input["content"] = json!(text);
        }
        json!({
            "session_id": session,
            "cwd": self.root,
            "hook_event_name": event,
            "tool_name": "Write",
            "tool_input": tool_input,
        })
        .to_string()
    }

    /// A payload for an event that names no tool.
    fn session_payload(&self, session: &str, event: &str, extra: &Value) -> String {
        let mut payload = json!({
            "session_id": session,
            "cwd": self.root,
            "hook_event_name": event,
        });
        if let Some(fields) = extra.as_object() {
            for (key, value) in fields {
                payload[key] = value.clone();
            }
        }
        payload.to_string()
    }

    /// Run a subcommand with a payload on stdin. Returns `(stdout, stderr, code)`.
    fn run(&self, args: &[&str], stdin: &str) -> (String, String, i32) {
        use std::io::Write as _;
        let mut child = Command::new(binary())
            .args(args)
            .current_dir(&self.root)
            .env("CANON_DATA_DIR", &self.data)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn canon");
        child.stdin.as_mut().unwrap().write_all(stdin.as_bytes()).unwrap();
        let out = child.wait_with_output().expect("wait");
        (
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
            out.status.code().unwrap_or(-1),
        )
    }

    /// Parse a hook's answer, failing loudly if it is not JSON.
    fn json(&self, args: &[&str], stdin: &str) -> Value {
        let (stdout, stderr, code) = self.run(args, stdin);
        assert_eq!(code, 0, "{args:?} exited {code}");
        assert!(stderr.is_empty(), "{args:?} wrote to stderr: {stderr}");
        serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("{args:?} produced invalid JSON ({e}): {stdout}"))
    }

    fn service_repo(name: &str) -> Self {
        let f = Self::new(name);
        for i in 0..6 {
            f.write(
                &format!("app/services/item_{i}.rb"),
                &format!("class Item{i} < ApplicationService\n  def call\n    :ok\n  end\n\n  private\n\n  def helper\n  end\nend\n"),
            );
        }
        f.run(&["index", "--rebuild"], "");
        f
    }
}

/// The injected text, or `None` when the hook chose silence.
fn context(parsed: &Value) -> Option<&str> {
    parsed["hookSpecificOutput"]["additionalContext"].as_str()
}

#[test]
fn indexing_then_injecting_produces_the_conventions_for_the_target() {
    let f = Fixture::service_repo("inject");
    // Content that already satisfies the rules the repository holds without
    // exception, so this exercises the advisory path rather than the refusal.
    let payload = f.tool_payload(
        "s1",
        "PreToolUse",
        "app/services/item_new.rb",
        Some("class ItemNew < ApplicationService\n  def call; end\nend\n"),
    );
    let parsed = f.json(&["inject"], &payload);

    assert_eq!(parsed["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    let block = context(&parsed).expect("a block");
    assert!(block.contains("exactly 1 public method"), "got {block}");
    assert!(block.contains("named `call`"), "got {block}");
    assert!(block.contains("ApplicationService"), "got {block}");
}

#[test]
fn refusing_can_be_switched_off() {
    // On by default, because advising is the channel a model may decline. The
    // switch exists for a repository that would rather never be interrupted.
    let f = Fixture::service_repo("refuse-default");
    f.write(".canon.toml", "enforce = false\n");
    f.run(&["index", "--rebuild"], "");
    let payload = f.tool_payload(
        "s1",
        "PreToolUse",
        "app/services/item_new.rb",
        Some("class ItemNew\n  def perform; end\n  def also; end\nend\n"),
    );
    let parsed = f.json(&["inject"], &payload);
    assert!(
        parsed["hookSpecificOutput"]["permissionDecision"].is_null(),
        "enforce = false must not refuse: {parsed}"
    );
    assert!(context(&parsed).is_some(), "it should still advise");
}

#[test]
fn a_write_that_breaks_a_rule_held_without_exception_is_refused() {
    // The only channel the model cannot decline. Advisory context steers a
    // write; a refusal prevents it.
    let f = Fixture::service_repo("refuse");
    let payload = f.tool_payload(
        "s1",
        "PreToolUse",
        "app/services/item_new.rb",
        Some("class ItemNew\n  def perform; end\n  def also; end\nend\n"),
    );
    let parsed = f.json(&["inject"], &payload);

    assert_eq!(parsed["hookSpecificOutput"]["permissionDecision"], "deny");
    let reason =
        parsed["hookSpecificOutput"]["permissionDecisionReason"].as_str().expect("a reason");
    assert!(reason.contains("without exception"), "got {reason}");
    assert!(reason.contains("/6"), "the counts have to be there: {reason}");
    assert!(reason.contains("canon explain"), "a refusal must be auditable: {reason}");
}

#[test]
fn a_rule_with_a_counterexample_never_refuses() {
    // One disagreeing file in the tree means the rule has an exception nobody
    // wrote down, and refusing a write that matches it would be indefensible.
    let f = Fixture::new("refuse-partial");
    for i in 0..6 {
        f.write(
            &format!("app/services/item_{i}.rb"),
            &format!("class Item{i} < ApplicationService\n  def call; end\nend\n"),
        );
    }
    // The counterexample: same directory, no base class.
    f.write("app/services/odd_one.rb", "class OddOne\n  def call; end\nend\n");
    f.run(&["index", "--rebuild"], "");

    let payload = f.tool_payload(
        "s1",
        "PreToolUse",
        "app/services/item_new.rb",
        Some("class ItemNew\n  def call; end\nend\n"),
    );
    let parsed = f.json(&["inject"], &payload);
    assert!(
        parsed["hookSpecificOutput"]["permissionDecision"].is_null(),
        "a rule with a counterexample must only advise: {parsed}"
    );
}

#[test]
fn injecting_for_an_unrelated_path_says_nothing() {
    let f = Fixture::service_repo("inject-miss");
    let payload = f.tool_payload("s1", "PreToolUse", "docs/readme.md", None);
    let (stdout, _, code) = f.run(&["inject"], &payload);
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "{}");
}

#[test]
fn verifying_a_conforming_file_says_nothing() {
    let f = Fixture::service_repo("verify-clean");
    f.write(
        "app/services/item_new.rb",
        "class ItemNew < ApplicationService\n  def call; end\nend\n",
    );
    let payload = f.tool_payload("s1", "PostToolUse", "app/services/item_new.rb", None);
    let (stdout, _, code) = f.run(&["verify"], &payload);
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "{}");
}

#[test]
fn verifying_a_divergent_file_reports_the_difference_with_its_evidence() {
    let f = Fixture::service_repo("verify-dirty");
    f.write(
        "app/services/item_new.rb",
        "class ItemNew\n  def perform; end\n  def also; end\nend\n",
    );
    let payload = f.tool_payload("s1", "PostToolUse", "app/services/item_new.rb", None);
    let parsed = f.json(&["verify"], &payload);

    assert!(parsed["decision"].is_null(), "nothing derived by counting may block");
    let text = context(&parsed).expect("a report");
    assert!(text.contains("exposes 2 public method"), "got {text}");
    assert!(text.contains("/6"), "the evidence count must be present: {text}");
}

#[test]
fn session_start_states_what_the_repository_looks_like() {
    let f = Fixture::service_repo("session");
    let payload = f.session_payload("s1", "SessionStart", &json!({ "source": "startup" }));
    let parsed = f.json(&["session-start"], &payload);

    assert_eq!(parsed["hookSpecificOutput"]["hookEventName"], "SessionStart");
    let text = context(&parsed).expect("a manifest");
    assert!(text.contains("canon:"), "got {text}");
    assert!(text.contains("Ruby"), "got {text}");
}

#[test]
fn a_config_that_will_not_load_is_said_out_loud_once() {
    // The setting range narrowed in 0.5.0, so a `.canon.toml` that loaded
    // yesterday can stop loading today. The fallback is every default plus
    // enforcement off, which means refusals and suppressions both stop, and
    // the only previous record was a log line at a level that defaults to
    // off. Silence is the one thing this must not be.
    // Its own fixture name. Tests run in parallel and `Fixture::new` clears the
    // directory it is given, so two tests sharing a name race on it.
    let f = Fixture::service_repo("badconfig-spoken");
    f.write(".canon.toml", "confidence_floor = 0.7\n");
    let payload = f.session_payload("s1", "SessionStart", &json!({ "source": "startup" }));
    let parsed = f.json(&["session-start"], &payload);

    let said = parsed["systemMessage"].as_str().unwrap_or_default();
    assert!(said.contains(".canon.toml"), "the file was not named: {parsed}");
    assert!(said.contains("confidence_floor"), "the reason was not given: {said}");
    assert!(
        said.contains("enforcement") || said.contains("refuse"),
        "the consequence was not stated: {said}"
    );
}

#[test]
fn a_subagent_receives_the_same_manifest() {
    // The reason canon is a hook: a subagent starts with an empty context
    // window, so nothing in the conversation reaches it.
    let f = Fixture::service_repo("subagent");
    let payload = f.session_payload(
        "s1",
        "SubagentStart",
        &json!({ "agent_id": "ag1", "agent_type": "general-purpose" }),
    );
    let parsed = f.json(&["subagent-start"], &payload);

    assert_eq!(parsed["hookSpecificOutput"]["hookEventName"], "SubagentStart");
    assert!(context(&parsed).expect("a manifest").contains("canon:"));
}

/// Long enough to clear the duplication floor.
///
/// A ten-line service object shares its whole shape with every sibling by
/// design, which is why the detector requires real overlap before it says
/// anything. The fixture has to be a file where copying is actually a choice.
const LONG_SERVICE: &str = "\
class PayoutProcessor < ApplicationService
  def call
    validate_account
    compute_fees
    transfer_funds
    notify_customer
    record_audit_entry
    reconcile_ledger
    emit_metrics
  end
end
";

#[test]
fn reconcile_reports_a_file_copied_from_its_sibling() {
    let f = Fixture::service_repo("reconcile");
    f.write("app/services/payout_processor.rb", LONG_SERVICE);
    f.run(&["index", "--rebuild"], "");
    f.write(
        "app/services/item_copy.rb",
        &LONG_SERVICE.replace("PayoutProcessor", "RefundProcessor"),
    );

    f.run(&["verify"], &f.tool_payload("s9", "PostToolUse", "app/services/item_copy.rb", None));
    let parsed = f.json(&["reconcile"], &f.session_payload("s9", "Stop", &json!({})));

    let text = context(&parsed).expect("a report");
    assert!(text.contains("already exists in"), "got {text}");
}

#[test]
fn check_reports_the_capability_table_from_the_binary_not_from_a_document() {
    let f = Fixture::service_repo("check");
    let (stdout, _, code) = f.run(&["check"], "");
    assert_eq!(code, 0);
    assert!(stdout.contains("Ruby"));
    assert!(stdout.contains("wired"));
    assert!(stdout.contains("Vue SFC"));
    assert!(stdout.contains("conventions from 6 files"));
}

#[test]
fn explain_shows_the_evidence_behind_a_rule() {
    let f = Fixture::service_repo("explain");
    let (stdout, _, code) = f.run(&["explain", "app/services"], "");
    assert_eq!(code, 0);
    assert!(stdout.contains("shape.entrypoint"), "got {stdout}");
    assert!(stdout.contains("evidence"), "got {stdout}");
    assert!(stdout.contains("item_"), "the actual files must be listed: {stdout}");
}

#[test]
fn an_unknown_subcommand_fails_loudly_for_a_human() {
    let f = Fixture::new("badargs");
    let (stdout, stderr, code) = f.run(&["inject-everything"], "");
    assert_eq!(code, 2);
    assert!(stdout.is_empty());
    assert!(stderr.contains("unknown subcommand"), "got {stderr}");
}

#[test]
fn a_broken_config_does_not_take_the_hooks_offline() {
    let f = Fixture::service_repo("badconfig");
    f.write(".canon.toml", "min_fils = 3\n");
    let payload =
        f.tool_payload("s1", "PreToolUse", "app/services/item_new.rb", Some("class X; end"));
    let (stdout, stderr, code) = f.run(&["inject"], &payload);
    assert_eq!(code, 0, "a config typo must not break the editor");
    assert!(stderr.is_empty());
    serde_json::from_str::<Value>(&stdout).expect("still valid JSON");

    // ...but a human asking directly is told the truth.
    let (check_out, _, _) = f.run(&["check"], "");
    assert!(check_out.contains("INVALID"), "got {check_out}");
}

#[test]
fn a_payload_carrying_a_windows_style_path_is_still_valid_json() {
    // The bug this file's header describes: a hand-built payload embedding
    // `C:\Users\...` is malformed, canon fails open, and every assertion above
    // reads as a product bug rather than a test bug.
    let f = Fixture::new("json-escaping");
    let payload = f.tool_payload("s1", "PreToolUse", "app/a.rb", None);
    let parsed: Value = serde_json::from_str(&payload).expect("payloads must be valid JSON");
    assert_eq!(parsed["hook_event_name"], "PreToolUse");
    assert!(parsed["tool_input"]["file_path"].is_string());
}

/// The plugin manifest, read from the repository rather than from memory.
fn hooks_manifest() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hooks/hooks.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is not readable: {e}", path.display()));
    serde_json::from_str(&text).expect("hooks.json is valid JSON")
}

#[test]
fn the_session_start_matcher_covers_every_documented_source() {
    // The docs list five, canon's matcher predates the fifth, and a forked
    // session (`--fork-session`, `/fork`, `/branch`) therefore gets no
    // conventions manifest at all. Before v2.1.214 a fork reported `resume`,
    // which is why this went unnoticed.
    let manifest = hooks_manifest();
    let matcher = manifest["hooks"]["SessionStart"][0]["matcher"]
        .as_str()
        .expect("SessionStart carries a matcher");
    for source in ["startup", "resume", "clear", "compact", "fork"] {
        assert!(matcher.contains(source), "`{source}` is missing from `{matcher}`");
    }
}
