//! Strict Codex 0.144.5 app-server approval-request normalization.

use std::collections::BTreeMap;

use codex_notifier_core::{
    CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Urgency,
};
use serde::Deserialize;
use serde_json::Value;
use time::OffsetDateTime;

use crate::{CodexCliVersion, CodexInterface, SourceContext, SourceError, hash_source_id};

/// Maximum accepted bytes for one Codex approval-request payload.
pub const MAX_APPROVAL_REQUESTED_INPUT_BYTES: usize = 32_768;
const MAX_SOURCE_ID_BYTES: usize = 256;
const MAX_OPTION_ENTRIES: usize = 32;
const APPROVAL_METHOD: &str = "item/commandExecution/requestApproval";

/// Fixture-gated approval-request source adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApprovalRequestedAdapter {
    version: CodexCliVersion,
}

impl ApprovalRequestedAdapter {
    /// Selects the real-fixture-supported app-server adapter.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::UnsupportedInterface`] for the unverified CLI
    /// lifecycle-hook surface.
    pub const fn new(
        version: CodexCliVersion,
        interface: CodexInterface,
    ) -> Result<Self, SourceError> {
        match interface {
            CodexInterface::AppServer => Ok(Self { version }),
            CodexInterface::CliHook => Err(SourceError::UnsupportedInterface),
        }
    }

    /// Returns the exact Codex version selected by this adapter.
    #[must_use]
    pub const fn version(self) -> CodexCliVersion {
        self.version
    }

    /// Converts one verified app-server request into a private canonical event.
    ///
    /// # Errors
    ///
    /// Returns a size, shape, value, context, timestamp, or canonical
    /// validation failure. Command and approval-decision data are discarded.
    pub fn normalize(
        self,
        input: &[u8],
        context: &SourceContext,
        event_id: EventId,
        received_at: OffsetDateTime,
    ) -> Result<CanonicalEvent, SourceError> {
        if input.len() > MAX_APPROVAL_REQUESTED_INPUT_BYTES {
            return Err(SourceError::PayloadTooLarge);
        }
        let request: ApprovalRequestV0144 =
            serde_json::from_slice(input).map_err(|_| SourceError::IncompatiblePayload)?;
        request.validate()?;
        let occurred_at = OffsetDateTime::from_unix_timestamp_nanos(
            i128::from(request.params.started_at_ms) * 1_000_000,
        )
        .map_err(|_| SourceError::IncompatiblePayload)?;

        let source = EventSource::new(
            context.host_label.clone(),
            context.project_label.clone(),
            Some(hash_source_id(&request.params.thread_id)),
        )
        .map_err(|_| SourceError::EventBuildFailed)?;
        let presentation = Presentation::new(
            "Codex needs approval",
            "Open Codex to review the request.",
            Urgency::High,
            Privacy::Private,
        )
        .map_err(|_| SourceError::EventBuildFailed)?;
        CanonicalEvent::new(
            event_id,
            EventKind::ApprovalRequested,
            occurred_at,
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
struct ApprovalRequestV0144 {
    id: RequestId,
    method: String,
    params: ApprovalParamsV0144,
}

impl ApprovalRequestV0144 {
    fn validate(&self) -> Result<(), SourceError> {
        if self.method != APPROVAL_METHOD
            || !self.id.valid()
            || !valid_source_id(&self.params.thread_id)
            || !valid_source_id(&self.params.turn_id)
            || !valid_source_id(&self.params.item_id)
            || self
                .params
                .available_decisions
                .as_ref()
                .is_some_and(|values| values.len() > MAX_OPTION_ENTRIES)
            || self
                .params
                .command_actions
                .as_ref()
                .is_some_and(|values| values.len() > MAX_OPTION_ENTRIES)
            || self
                .params
                .proposed_execpolicy_amendment
                .as_ref()
                .is_some_and(|values| values.len() > MAX_OPTION_ENTRIES)
            || self
                .params
                .proposed_network_policy_amendments
                .as_ref()
                .is_some_and(|values| values.len() > MAX_OPTION_ENTRIES)
        {
            return Err(SourceError::IncompatiblePayload);
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RequestId {
    String(String),
    Integer(i64),
}

impl RequestId {
    fn valid(&self) -> bool {
        match self {
            Self::String(value) => valid_source_id(value),
            Self::Integer(value) => value.to_string().len() <= 20,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApprovalParamsV0144 {
    thread_id: String,
    turn_id: String,
    item_id: String,
    started_at_ms: i64,
    #[serde(default, rename = "additionalPermissions")]
    _additional_permissions: Option<Value>,
    #[serde(default, rename = "approvalId")]
    _approval_id: Option<String>,
    #[serde(default)]
    available_decisions: Option<Vec<Value>>,
    #[serde(default, rename = "command")]
    _command: Option<String>,
    #[serde(default)]
    command_actions: Option<Vec<Value>>,
    #[serde(default, rename = "cwd")]
    _cwd: Option<String>,
    #[serde(default, rename = "environmentId")]
    _environment_id: Option<String>,
    #[serde(default, rename = "networkApprovalContext")]
    _network_approval_context: Option<Value>,
    #[serde(default)]
    proposed_execpolicy_amendment: Option<Vec<String>>,
    #[serde(default)]
    proposed_network_policy_amendments: Option<Vec<Value>>,
    #[serde(default, rename = "reason")]
    _reason: Option<String>,
}

fn valid_source_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_SOURCE_ID_BYTES && !value.chars().any(char::is_control)
}
