//! The JSON document canon writes to stdout.
//!
//! The envelope is small on purpose. Everything canon needs is one of three
//! shapes: say nothing, add context, or block with a reason. Constructors
//! rather than public fields, so the `hookEventName` can never disagree with
//! the event that fired, which the host rejects outright.

use serde::Serialize;

/// The hook events canon registers for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Session opened, resumed, cleared or compacted.
    SessionStart,
    /// A subagent is starting, with its own empty context window.
    SubagentStart,
    /// The user submitted a prompt.
    UserPromptSubmit,
    /// A tool is about to run.
    PreToolUse,
    /// A tool finished.
    PostToolUse,
    /// A batch of tool calls resolved.
    PostToolBatch,
    /// The main agent is about to conclude its response.
    Stop,
    /// A subagent is about to conclude its response.
    SubagentStop,
}

impl Event {
    /// The wire name. Must match the event that fired: the host rejects output
    /// whose `hookEventName` disagrees with the invocation.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::SubagentStart => "SubagentStart",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::PostToolBatch => "PostToolBatch",
            Self::Stop => "Stop",
            Self::SubagentStop => "SubagentStop",
        }
    }

    /// Parse the name the host sent.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        [
            Self::SessionStart,
            Self::SubagentStart,
            Self::UserPromptSubmit,
            Self::PreToolUse,
            Self::PostToolUse,
            Self::PostToolBatch,
            Self::Stop,
            Self::SubagentStop,
        ]
        .into_iter()
        .find(|e| e.wire_name() == name)
    }

    /// Whether `additionalContext` from this event reaches the model.
    ///
    /// True for every event canon registers for. Stated as a function rather
    /// than assumed, so that adding an event where it is false cannot silently
    /// produce a hook that runs, costs time, and is never read.
    #[must_use]
    pub fn context_reaches_model(self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct HookSpecific {
    hook_event_name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    additional_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    permission_decision: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    permission_decision_reason: Option<String>,
}

/// What canon tells the host.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    decision: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hook_specific_output: Option<HookSpecific>,
}

impl HookOutput {
    /// Say nothing. Serialises to `{}`.
    ///
    /// The default for every path canon is not confident about, and the value
    /// the harness falls back to on any internal failure.
    #[must_use]
    pub fn silent() -> Self {
        Self::default()
    }

    /// Add context for the model to read.
    #[must_use]
    pub fn context(event: Event, text: impl Into<String>) -> Self {
        Self {
            hook_specific_output: Some(HookSpecific {
                hook_event_name: event.wire_name(),
                additional_context: Some(text.into()),
                permission_decision: None,
                permission_decision_reason: None,
            }),
            ..Self::default()
        }
    }

    /// Feed a reason back to the model after a tool has already run.
    ///
    /// Only for a violation of a rule the repository agrees on totally and
    /// that can be checked without a false positive. Everything derived by
    /// counting uses [`Self::context`] instead.
    #[must_use]
    pub fn block(event: Event, reason: impl Into<String>) -> Self {
        Self {
            decision: Some("block"),
            reason: Some(reason.into()),
            hook_specific_output: Some(HookSpecific {
                hook_event_name: event.wire_name(),
                additional_context: None,
                permission_decision: None,
                permission_decision_reason: None,
            }),
            ..Self::default()
        }
    }

    /// Refuse a tool call before it runs.
    ///
    /// Deliberately not reachable from any derived convention. It exists for
    /// the one case where canon knows something the model cannot: that its own
    /// index is mid-rebuild and the advice would be wrong. Kept here rather
    /// than omitted so the protocol stays complete and testable.
    #[must_use]
    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            hook_specific_output: Some(HookSpecific {
                hook_event_name: Event::PreToolUse.wire_name(),
                additional_context: None,
                permission_decision: Some("deny"),
                permission_decision_reason: Some(reason.into()),
            }),
            ..Self::default()
        }
    }

    /// Attach a line for the user's terminal. Never read by the model.
    #[must_use]
    pub fn with_system_message(mut self, text: impl Into<String>) -> Self {
        self.system_message = Some(text.into());
        self
    }

    /// Whether this output carries anything at all.
    #[must_use]
    pub fn is_silent(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(o: &HookOutput) -> String {
        serde_json::to_string(o).unwrap()
    }

    #[test]
    fn silence_is_an_empty_object_not_a_null() {
        assert_eq!(json(&HookOutput::silent()), "{}");
        assert!(HookOutput::silent().is_silent());
    }

    #[test]
    fn context_carries_the_event_name_the_host_requires() {
        let out = HookOutput::context(Event::PreToolUse, "rules");
        assert_eq!(
            json(&out),
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"rules"}}"#
        );
    }

    #[test]
    fn every_event_round_trips_through_its_wire_name() {
        for e in [
            Event::SessionStart,
            Event::SubagentStart,
            Event::UserPromptSubmit,
            Event::PreToolUse,
            Event::PostToolUse,
            Event::PostToolBatch,
            Event::Stop,
            Event::SubagentStop,
        ] {
            assert_eq!(Event::parse(e.wire_name()), Some(e));
            assert!(e.context_reaches_model(), "{} would run and never be read", e.wire_name());
        }
    }

    #[test]
    fn an_unknown_event_name_is_none_rather_than_a_guess() {
        assert_eq!(Event::parse("PreCompact"), None);
        assert_eq!(Event::parse(""), None);
    }

    #[test]
    fn a_block_carries_both_the_decision_and_the_reason() {
        let out = HookOutput::block(Event::PostToolUse, "arity 3, repo agrees on 1");
        let text = json(&out);
        assert!(text.contains(r#""decision":"block""#));
        assert!(text.contains("arity 3"));
        assert!(text.contains(r#""hookEventName":"PostToolUse""#));
    }

    #[test]
    fn a_deny_is_always_scoped_to_pretooluse() {
        let text = json(&HookOutput::deny("index rebuilding"));
        assert!(text.contains(r#""permissionDecision":"deny""#));
        assert!(text.contains(r#""hookEventName":"PreToolUse""#));
    }

    #[test]
    fn a_system_message_does_not_make_the_output_look_like_context() {
        let out = HookOutput::silent().with_system_message("reindexed");
        assert!(json(&out).contains(r#""systemMessage":"reindexed""#));
        assert!(!json(&out).contains("additionalContext"));
    }

    #[test]
    fn absent_fields_are_omitted_rather_than_serialised_as_null() {
        // A null in a field the host reads as optional is not the same as an
        // absent one, and the difference has been a real source of breakage.
        assert!(!json(&HookOutput::context(Event::Stop, "x")).contains("null"));
    }
}
