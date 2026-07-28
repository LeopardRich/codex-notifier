//! Structured bounded acknowledgement model.

use codex_notifier_core::EventId;
use serde::{Deserialize, Deserializer, Serialize, de};

use crate::IpcError;

pub(crate) const MAX_ACK_BYTES: usize = 2_048;
const MAX_ERROR_CODE_BYTES: usize = 64;
const MAX_ERROR_MESSAGE_SCALARS: usize = 160;

/// Local submission acknowledgement status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AckStatus {
    /// The agent accepted a new event.
    Accepted,
    /// The agent already has a durable record for the event.
    Duplicate,
    /// The desktop completed delivery.
    Delivered,
    /// The event was rejected permanently or transiently.
    Rejected,
}

/// Bounded safe acknowledgement failure.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AckError {
    code: String,
    retryable: bool,
    message: String,
}

impl AckError {
    /// Creates a safe failure from a stable code and fixed message.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::MalformedAcknowledgement`] for invalid identifiers,
    /// controls, multiline text, or excessive length.
    pub fn new(
        code: impl Into<String>,
        retryable: bool,
        message: impl Into<String>,
    ) -> Result<Self, IpcError> {
        let value = Self {
            code: code.into(),
            retryable,
            message: message.into(),
        };
        value.validate()?;
        Ok(value)
    }

    /// Returns the stable machine-readable error code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns whether retrying may succeed.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    /// Returns the safe fixed diagnostic message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    fn validate(&self) -> Result<(), IpcError> {
        if !valid_identifier(&self.code)
            || self.message.is_empty()
            || self.message.chars().count() > MAX_ERROR_MESSAGE_SCALARS
            || self.message.chars().any(char::is_control)
        {
            return Err(IpcError::MalformedAcknowledgement);
        }
        Ok(())
    }
}

/// Protocol version 1 compact acknowledgement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Acknowledgement {
    schema_version: u16,
    #[serde(deserialize_with = "deserialize_event_id")]
    event_id: EventId,
    status: AckStatus,
    error: Option<AckError>,
}

impl Acknowledgement {
    /// Creates an acknowledgement for a newly accepted event.
    #[must_use]
    pub const fn accepted(event_id: EventId) -> Self {
        Self {
            schema_version: 1,
            event_id,
            status: AckStatus::Accepted,
            error: None,
        }
    }

    /// Creates an acknowledgement for an already durable event.
    #[must_use]
    pub const fn duplicate(event_id: EventId) -> Self {
        Self {
            schema_version: 1,
            event_id,
            status: AckStatus::Duplicate,
            error: None,
        }
    }

    /// Creates an acknowledgement for completed desktop delivery.
    #[must_use]
    pub const fn delivered(event_id: EventId) -> Self {
        Self {
            schema_version: 1,
            event_id,
            status: AckStatus::Delivered,
            error: None,
        }
    }

    /// Creates a non-rejected acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::MalformedAcknowledgement`] for `Rejected`, which
    /// requires a safe error object.
    pub fn success(event_id: EventId, status: AckStatus) -> Result<Self, IpcError> {
        if status == AckStatus::Rejected {
            return Err(IpcError::MalformedAcknowledgement);
        }
        Ok(Self {
            schema_version: 1,
            event_id,
            status,
            error: None,
        })
    }

    /// Creates a rejected acknowledgement with a safe error.
    #[must_use]
    pub const fn rejected(event_id: EventId, error: AckError) -> Self {
        Self {
            schema_version: 1,
            event_id,
            status: AckStatus::Rejected,
            error: Some(error),
        }
    }

    /// Returns the matching event identifier.
    #[must_use]
    pub const fn event_id(&self) -> EventId {
        self.event_id
    }

    /// Returns the acknowledgement status.
    #[must_use]
    pub const fn status(&self) -> AckStatus {
        self.status
    }

    /// Returns the optional safe rejection information.
    #[must_use]
    pub const fn error(&self) -> Option<&AckError> {
        self.error.as_ref()
    }

    pub(crate) fn to_bytes(&self) -> Result<Vec<u8>, IpcError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| IpcError::MalformedAcknowledgement)?;
        if bytes.len() > MAX_ACK_BYTES {
            return Err(IpcError::FrameTooLarge);
        }
        Ok(bytes)
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, IpcError> {
        if bytes.len() > MAX_ACK_BYTES {
            return Err(IpcError::FrameTooLarge);
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|_| IpcError::MalformedAcknowledgement)?;
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), IpcError> {
        if self.schema_version != 1 || (self.status == AckStatus::Rejected) != self.error.is_some()
        {
            return Err(IpcError::MalformedAcknowledgement);
        }
        if let Some(error) = &self.error {
            error.validate()?;
        }
        Ok(())
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ERROR_CODE_BYTES
        && value.is_ascii()
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (index > 0 && matches!(byte, b'_' | b'-' | b'.'))
        })
}

fn deserialize_event_id<'de, D>(deserializer: D) -> Result<EventId, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    EventId::parse(&value).map_err(|_| de::Error::custom("invalid event identifier"))
}
