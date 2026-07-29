//! Strict Codex 0.144.5 `Stop` hook normalization.

use std::collections::BTreeMap;

use codex_notifier_core::{
    CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Urgency,
};
use serde::{Deserialize, Deserializer};
use time::OffsetDateTime;

use crate::{CodexCliVersion, CodexInterface, SourceContext, SourceError, hash_source_id};

/// Maximum accepted bytes for one Codex task-completion hook payload.
pub const MAX_TASK_COMPLETED_INPUT_BYTES: usize = 32_768;
const MAX_SOURCE_ID_BYTES: usize = 256;
const MAX_MODEL_BYTES: usize = 128;

/// Fixture-gated task-completion source adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskCompletedAdapter {
    version: CodexCliVersion,
}

impl TaskCompletedAdapter {
    /// Selects an adapter only for a real-fixture-supported version/interface.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::UnsupportedInterface`] for non-hook surfaces.
    pub const fn new(
        version: CodexCliVersion,
        interface: CodexInterface,
    ) -> Result<Self, SourceError> {
        match interface {
            CodexInterface::CliHook => Ok(Self { version }),
            CodexInterface::AppServer => Err(SourceError::UnsupportedInterface),
        }
    }

    /// Returns the exact Codex version selected by this adapter.
    #[must_use]
    pub const fn version(self) -> CodexCliVersion {
        self.version
    }

    /// Converts one verified `Stop` payload into a privacy-safe canonical event.
    ///
    /// # Errors
    ///
    /// Returns a size, shape, value, context, or canonical validation failure.
    pub fn normalize(
        self,
        input: &[u8],
        context: &SourceContext,
        event_id: EventId,
        received_at: OffsetDateTime,
    ) -> Result<CanonicalEvent, SourceError> {
        if input.len() > MAX_TASK_COMPLETED_INPUT_BYTES {
            return Err(SourceError::PayloadTooLarge);
        }
        let payload: StopHookV0144 =
            serde_json::from_slice(input).map_err(|_| SourceError::IncompatiblePayload)?;
        payload.validate()?;

        let source = EventSource::new(
            context.host_label.clone(),
            context.project_label.clone(),
            Some(hash_source_id(&payload.session_id)),
        )
        .map_err(|_| SourceError::EventBuildFailed)?;
        let presentation = Presentation::new(
            "Codex task finished",
            "Open Codex to review the result.",
            Urgency::Normal,
            Privacy::Private,
        )
        .map_err(|_| SourceError::EventBuildFailed)?;
        CanonicalEvent::new(
            event_id,
            EventKind::TaskCompleted,
            received_at,
            source,
            presentation,
            context.routing.clone(),
            Extensions::new(BTreeMap::new()).map_err(|_| SourceError::EventBuildFailed)?,
            received_at,
        )
        .map_err(|_| SourceError::EventBuildFailed)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StopHookV0144 {
    session_id: String,
    #[allow(dead_code)]
    #[serde(deserialize_with = "deserialize_optional_string")]
    transcript_path: Option<String>,
    cwd: String,
    hook_event_name: String,
    model: String,
    turn_id: String,
    permission_mode: PermissionMode,
    #[allow(dead_code)]
    stop_hook_active: bool,
    #[allow(dead_code)]
    #[serde(deserialize_with = "deserialize_optional_string")]
    last_assistant_message: Option<String>,
}

impl StopHookV0144 {
    fn validate(&self) -> Result<(), SourceError> {
        if self.hook_event_name != "Stop"
            || !valid_source_id(&self.session_id)
            || !valid_source_id(&self.turn_id)
            || self.cwd.is_empty()
            || self.cwd.chars().any(char::is_control)
            || self.model.is_empty()
            || self.model.len() > MAX_MODEL_BYTES
            || self.model.chars().any(char::is_control)
        {
            return Err(SourceError::IncompatiblePayload);
        }
        let _ = self.permission_mode;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
enum PermissionMode {
    #[serde(rename = "default")]
    Default,
    #[serde(rename = "acceptEdits")]
    AcceptEdits,
    #[serde(rename = "plan")]
    Plan,
    #[serde(rename = "dontAsk")]
    DontAsk,
    #[serde(rename = "bypassPermissions")]
    BypassPermissions,
}

fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)
}

fn valid_source_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_SOURCE_ID_BYTES && !value.chars().any(char::is_control)
}
