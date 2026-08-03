//! The payload the host writes to stdin.
//!
//! Every field is optional and unknown fields are ignored. That is the
//! opposite of the policy for user configuration, and deliberately so: a
//! config typo is a mistake the user wants told about, whereas a new field in
//! the host's payload is the host evolving, and a hook that rejects it breaks
//! the day the host ships a release.

use serde::Deserialize;

/// Tool arguments, flattened across the tools canon watches.
///
/// `Write` carries `content`, `Edit` carries `new_string`, and `NotebookEdit`
/// carries `new_source` against `notebook_path`. Modelling them as one
/// optional-everything struct keeps the handlers from caring which tool fired.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ToolInput {
    /// Target path for `Write` and `Edit`.
    pub file_path: Option<String>,
    /// Target path for `NotebookEdit`.
    pub notebook_path: Option<String>,
    /// Full contents, for `Write`.
    pub content: Option<String>,
    /// Replacement text, for `Edit`.
    pub new_string: Option<String>,
    /// Replaced text, for `Edit`.
    pub old_string: Option<String>,
    /// Replacement cell source, for `NotebookEdit`.
    pub new_source: Option<String>,
}

impl ToolInput {
    /// The path this tool call targets, whichever field carries it.
    #[must_use]
    pub fn path(&self) -> Option<&str> {
        self.file_path.as_deref().or(self.notebook_path.as_deref())
    }

    /// The text this tool call is about to put on disk, whichever field
    /// carries it.
    ///
    /// For an `Edit` this is the replacement fragment, not the resulting file.
    /// Callers that need the whole file read it from disk after the write.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.content.as_deref().or(self.new_string.as_deref()).or(self.new_source.as_deref())
    }
}

/// One hook invocation.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct HookInput {
    /// Host session identifier.
    pub session_id: String,
    /// Working directory the hook was invoked in.
    pub cwd: Option<String>,
    /// Which event fired.
    pub hook_event_name: String,
    /// Tool that matched, for the tool events.
    pub tool_name: Option<String>,
    /// Tool arguments.
    pub tool_input: ToolInput,
    /// `startup`, `resume`, `clear` or `compact`, for `SessionStart`.
    pub source: Option<String>,
    /// The user's text, for `UserPromptSubmit`.
    pub prompt: Option<String>,
    /// Subagent identifier, for the subagent events.
    ///
    /// The reason hooks beat every other way of carrying context: a subagent
    /// starts with an empty context window, so a skill or a note in the
    /// conversation does not reach it, and a hook does.
    pub agent_id: Option<String>,
    /// Subagent type, for the subagent events.
    pub agent_type: Option<String>,
}

impl HookInput {
    /// Repository root for this invocation, falling back to the process
    /// working directory when the host did not supply one.
    #[must_use]
    pub fn root(&self) -> std::path::PathBuf {
        self.cwd.as_deref().map_or_else(
            || std::env::current_dir().unwrap_or_else(|_| ".".into()),
            std::path::PathBuf::from,
        )
    }

    /// Path targeted by this invocation, if any.
    #[must_use]
    pub fn target_path(&self) -> Option<&str> {
        self.tool_input.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> HookInput {
        serde_json::from_str(s).expect("parses")
    }

    #[test]
    fn a_write_payload_exposes_path_and_content() {
        let h = parse(
            r#"{"session_id":"a","cwd":"/r","hook_event_name":"PreToolUse","tool_name":"Write",
                "tool_input":{"file_path":"/r/app/a.rb","content":"class A; end"}}"#,
        );
        assert_eq!(h.target_path(), Some("/r/app/a.rb"));
        assert_eq!(h.tool_input.text(), Some("class A; end"));
    }

    #[test]
    fn an_edit_payload_exposes_the_replacement_text() {
        let h = parse(
            r#"{"hook_event_name":"PreToolUse","tool_name":"Edit",
                "tool_input":{"file_path":"a.ts","old_string":"x","new_string":"y"}}"#,
        );
        assert_eq!(h.tool_input.text(), Some("y"));
    }

    #[test]
    fn a_notebook_payload_uses_its_own_two_field_names() {
        let h = parse(
            r#"{"hook_event_name":"PreToolUse","tool_name":"NotebookEdit",
                "tool_input":{"notebook_path":"n.ipynb","new_source":"import x"}}"#,
        );
        assert_eq!(h.target_path(), Some("n.ipynb"));
        assert_eq!(h.tool_input.text(), Some("import x"));
    }

    #[test]
    fn an_unknown_field_from_a_newer_host_is_ignored_not_rejected() {
        // The host adding a field must never take canon offline.
        let h = parse(
            r#"{"hook_event_name":"PreToolUse","brand_new_field":{"nested":1},
                "tool_input":{"file_path":"a.rb","future_arg":true}}"#,
        );
        assert_eq!(h.hook_event_name, "PreToolUse");
        assert_eq!(h.target_path(), Some("a.rb"));
    }

    #[test]
    fn an_empty_object_parses_to_defaults() {
        let h = parse("{}");
        assert!(h.hook_event_name.is_empty());
        assert_eq!(h.target_path(), None);
    }

    #[test]
    fn a_subagent_payload_carries_its_identity() {
        let h = parse(
            r#"{"hook_event_name":"SubagentStart","agent_id":"ag_1","agent_type":"general-purpose"}"#,
        );
        assert_eq!(h.agent_id.as_deref(), Some("ag_1"));
        assert_eq!(h.agent_type.as_deref(), Some("general-purpose"));
    }

    #[test]
    fn a_wrongly_typed_field_fails_the_parse_so_the_harness_can_fail_open() {
        assert!(serde_json::from_str::<HookInput>(r#"{"tool_input":"a string"}"#).is_err());
    }
}
