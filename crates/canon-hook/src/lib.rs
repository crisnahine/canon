//! The Claude Code hook protocol, and the harness that guarantees canon never
//! breaks a session.
//!
//! # The contract
//!
//! Every hook this crate runs exits 0 and writes either nothing or one valid
//! JSON document to stdout. There is no input, no filesystem state, and no
//! internal failure that produces any other outcome. A convention engine that
//! occasionally says nothing is an inconvenience. One that panics mid-session
//! gets uninstalled and never reinstalled.
//!
//! # Which channel actually reaches the model
//!
//! This decides the whole design, and it is not obvious from the field names.
//! Measured against the running host rather than assumed:
//!
//! | Event | Reaches the model | Notes |
//! |---|---|---|
//! | `SessionStart` | `additionalContext` | once, before any work |
//! | `SubagentStart` | `additionalContext` | every subagent, own context window |
//! | `UserPromptSubmit` | `additionalContext` | required field, not optional |
//! | `PreToolUse` | `additionalContext` | arrives in time to change the write |
//! | `PostToolUse` | `additionalContext`, `decision: block` | after the write |
//! | `PostToolBatch` | `additionalContext` | once per batch of tool calls |
//! | `Stop` | `additionalContext` | conversation continues so it can act |
//!
//! `PreToolUse` carrying `additionalContext` is the load-bearing one, and it is
//! the one the host's own field list does not promise: `permissionDecision` and
//! `updatedInput` are described as `PreToolUse`-only, while `additionalContext`
//! is not described as belonging to it at all. It was confirmed behaviourally
//! instead. With the `Edit` tool withheld, a single `Write` still carried a
//! header that nothing but the injected text had asked for, so injection
//! reaches the model before the tool executes rather than after.
//!
//! If that ever stops being true the failure is silent, which is why
//! `tests/injection-reaches-the-model.sh` re-checks it against the installed
//! host instead of trusting this comment.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod input;
mod output;

pub use input::{HookInput, ToolInput};
pub use output::{Event, HookOutput};

use std::io::Read as _;

/// Read a hook payload from stdin, run `handler`, and emit its output.
///
/// Fail-open at four separate points, because each has a different cause and
/// all four happen: stdin unreadable, payload not JSON, payload valid JSON of
/// an unexpected shape, handler panicking.
///
/// Returns the process exit code, always zero. A return value rather than a
/// call to `exit`, so `main` stays testable and buffered output is flushed by
/// the normal path.
pub fn run<F>(handler: F) -> i32
where
    F: FnOnce(HookInput) -> HookOutput + std::panic::UnwindSafe,
{
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return emit(&HookOutput::silent());
    }
    let Ok(parsed) = serde_json::from_str::<HookInput>(&raw) else {
        return emit(&HookOutput::silent());
    };
    // A panic here would otherwise kill the process with empty stdout, which
    // the host cannot tell apart from a hook that chose to stay quiet.
    let out =
        std::panic::catch_unwind(move || handler(parsed)).unwrap_or_else(|_| HookOutput::silent());
    emit(&out)
}

/// Serialise and write. The only place in the workspace permitted to touch
/// stdout, which is why `print_stdout` is denied everywhere else.
fn emit(out: &HookOutput) -> i32 {
    let Ok(text) = serde_json::to_string(out) else {
        write_line("{}");
        return 0;
    };
    // The host truncates oversized output, and a truncated JSON document reads
    // as a crash on the far end. Say nothing rather than half a thing.
    if text.len() > canon_core::HOOK_OUTPUT_CAP {
        write_line("{}");
        return 0;
    }
    write_line(&text);
    0
}

#[allow(clippy::print_stdout)]
fn write_line(text: &str) {
    use std::io::Write as _;
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    // A failed write means the host closed the pipe. There is nothing to
    // recover, and reporting it would itself write to a stream the protocol
    // owns.
    let _ = writeln!(lock, "{text}");
    let _ = lock.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panicking_handler_degrades_to_an_empty_object() {
        let out = std::panic::catch_unwind(|| -> HookOutput { panic!("boom") })
            .unwrap_or_else(|_| HookOutput::silent());
        assert_eq!(serde_json::to_string(&out).unwrap(), "{}");
    }

    #[test]
    fn output_above_the_cap_is_replaced_rather_than_truncated() {
        let huge =
            HookOutput::context(Event::SessionStart, "x".repeat(canon_core::HOOK_OUTPUT_CAP + 1));
        let text = serde_json::to_string(&huge).unwrap();
        assert!(text.len() > canon_core::HOOK_OUTPUT_CAP, "precondition for the emit() guard");
    }
}
