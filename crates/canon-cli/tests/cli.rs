//! End-to-end tests against the built binary.
//!
//! These spawn the real `canon` executable and speak the real protocol over a
//! pipe. Everything below the binary is unit-tested in its own crate; what is
//! left to check is the part no library test can see: that the shipped
//! artifact reads stdin, writes one JSON document, and exits zero.

// `indexing_slicing` is denied in shipped code, where a panic inside a hook is
// the one failure the fail-open harness cannot hide. In a test a panic is the
// reporting mechanism, and `parsed["a"]["b"]` is how serde_json is read.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

    fn abs(&self, rel: &str) -> String {
        self.root.join(rel).display().to_string()
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

fn pretooluse(path: &str) -> String {
    format!(
        r#"{{"session_id":"s1","cwd":"{}","hook_event_name":"PreToolUse","tool_name":"Write","tool_input":{{"file_path":"{}","content":"class X\n  def call; end\nend\n"}}}}"#,
        Path::new(path)
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .unwrap()
            .display(),
        path
    )
}

#[test]
fn indexing_then_injecting_produces_the_conventions_for_the_target() {
    let f = Fixture::service_repo("inject");
    let payload = format!(
        r#"{{"session_id":"s1","cwd":"{}","hook_event_name":"PreToolUse","tool_name":"Write","tool_input":{{"file_path":"{}","content":"class New\n  def call; end\nend\n"}}}}"#,
        f.root.display(),
        f.abs("app/services/item_new.rb")
    );
    let (stdout, stderr, code) = f.run(&["inject"], &payload);

    assert_eq!(code, 0);
    assert!(stderr.is_empty(), "a hook must never write to stderr: {stderr}");

    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let block = parsed["hookSpecificOutput"]["additionalContext"].as_str().expect("a block");
    assert_eq!(parsed["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert!(block.contains("exactly 1 public method"), "got {block}");
    assert!(block.contains("named `call`"), "got {block}");
    assert!(block.contains("ApplicationService"), "got {block}");
}

#[test]
fn injecting_for_an_unrelated_path_says_nothing() {
    let f = Fixture::service_repo("inject-miss");
    let payload = format!(
        r#"{{"cwd":"{}","hook_event_name":"PreToolUse","tool_input":{{"file_path":"{}"}}}}"#,
        f.root.display(),
        f.abs("docs/readme.md")
    );
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
    let payload = format!(
        r#"{{"session_id":"s1","cwd":"{}","hook_event_name":"PostToolUse","tool_name":"Write","tool_input":{{"file_path":"{}"}}}}"#,
        f.root.display(),
        f.abs("app/services/item_new.rb")
    );
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
    let payload = format!(
        r#"{{"session_id":"s1","cwd":"{}","hook_event_name":"PostToolUse","tool_name":"Write","tool_input":{{"file_path":"{}"}}}}"#,
        f.root.display(),
        f.abs("app/services/item_new.rb")
    );
    let (stdout, stderr, code) = f.run(&["verify"], &payload);

    assert_eq!(code, 0, "a divergent file must not block the write");
    assert!(stderr.is_empty(), "PostToolUse stderr reaches the model: {stderr}");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(parsed["decision"].is_null(), "nothing derived by counting may block");
    let text = parsed["hookSpecificOutput"]["additionalContext"].as_str().expect("a report");
    assert!(text.contains("exposes 2 public method"), "got {text}");
    assert!(text.contains("/6"), "the evidence count must be present: {text}");
}

#[test]
fn session_start_states_what_the_repository_looks_like() {
    let f = Fixture::service_repo("session");
    let payload = format!(
        r#"{{"session_id":"s1","cwd":"{}","hook_event_name":"SessionStart","source":"startup"}}"#,
        f.root.display()
    );
    let (stdout, _, code) = f.run(&["session-start"], &payload);
    assert_eq!(code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["hookSpecificOutput"]["hookEventName"], "SessionStart");
    let text = parsed["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
    assert!(text.contains("canon:"), "got {text}");
    assert!(text.contains("Ruby"), "got {text}");
}

#[test]
fn a_subagent_receives_the_same_manifest() {
    // The reason canon is a hook: a subagent starts with an empty context
    // window, so nothing in the conversation reaches it.
    let f = Fixture::service_repo("subagent");
    let payload = format!(
        r#"{{"session_id":"s1","cwd":"{}","hook_event_name":"SubagentStart","agent_id":"ag1","agent_type":"general-purpose"}}"#,
        f.root.display()
    );
    let (stdout, _, code) = f.run(&["subagent-start"], &payload);
    assert_eq!(code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["hookSpecificOutput"]["hookEventName"], "SubagentStart");
    assert!(parsed["hookSpecificOutput"]["additionalContext"].as_str().unwrap().contains("canon:"));
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

    let post = format!(
        r#"{{"session_id":"s9","cwd":"{}","hook_event_name":"PostToolUse","tool_input":{{"file_path":"{}"}}}}"#,
        f.root.display(),
        f.abs("app/services/item_copy.rb")
    );
    f.run(&["verify"], &post);

    let stop =
        format!(r#"{{"session_id":"s9","cwd":"{}","hook_event_name":"Stop"}}"#, f.root.display());
    let (stdout, _, code) = f.run(&["reconcile"], &stop);
    assert_eq!(code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let text = parsed["hookSpecificOutput"]["additionalContext"].as_str().expect("a report");
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
    let payload = pretooluse(&f.abs("app/services/item_new.rb"));
    let (stdout, stderr, code) = f.run(&["inject"], &payload);
    assert_eq!(code, 0, "a config typo must not break the editor");
    assert!(stderr.is_empty());
    serde_json::from_str::<serde_json::Value>(&stdout).expect("still valid JSON");

    // ...but a human asking directly is told the truth.
    let (check_out, _, _) = f.run(&["check"], "");
    assert!(check_out.contains("INVALID"), "got {check_out}");
}
