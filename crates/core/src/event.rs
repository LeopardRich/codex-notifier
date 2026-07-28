//! Validated canonical event model.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, UtcOffset};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::error::{EventError, Field};
use crate::json::{parse_unique, reject_unknown_fields};
use crate::limits::{
    MAX_BODY_SCALARS, MAX_EVENT_BYTES, MAX_EXTENSION_CONTAINER_ENTRIES, MAX_EXTENSION_DEPTH,
    MAX_EXTENSION_ENTRIES, MAX_EXTENSION_STRING_SCALARS, MAX_EXTENSIONS_BYTES,
    MAX_FUTURE_SKEW_SECONDS, MAX_HOST_LABEL_SCALARS, MAX_PAST_AGE_SECONDS,
    MAX_PROJECT_LABEL_SCALARS, MAX_ROUTING_PROFILE_BYTES, MAX_SESSION_ID_BYTES, MAX_TITLE_SCALARS,
    SCHEMA_VERSION,
};

const EVENT_FIELDS: &[&str] = &[
    "schema_version",
    "event_id",
    "kind",
    "occurred_at",
    "source",
    "presentation",
    "routing",
    "extensions",
];
const SOURCE_FIELDS: &[&str] = &["host_label", "project_label", "session_id"];
const PRESENTATION_FIELDS: &[&str] = &["title", "body", "urgency", "privacy"];
const ROUTING_FIELDS: &[&str] = &["profile"];

/// A canonical `UUIDv7` event identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EventId(Uuid);

impl EventId {
    /// Validates an existing UUID as version 7.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::InvalidEventId`] when `value` is not version 7.
    pub fn from_uuid(value: Uuid) -> Result<Self, EventError> {
        if value.get_version_num() == 7 {
            Ok(Self(value))
        } else {
            Err(EventError::InvalidEventId)
        }
    }

    /// Parses lowercase canonical hyphenated `UUIDv7` text.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::InvalidEventId`] for malformed, noncanonical, or
    /// non-version-7 input.
    pub fn parse(value: &str) -> Result<Self, EventError> {
        let uuid = Uuid::parse_str(value).map_err(|_| EventError::InvalidEventId)?;
        if uuid.hyphenated().to_string() != value || uuid.get_version_num() != 7 {
            return Err(EventError::InvalidEventId);
        }
        Ok(Self(uuid))
    }

    /// Returns the underlying UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(formatter)
    }
}

impl Serialize for EventId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// User-facing event kinds supported by protocol version 1.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// Codex is waiting for an approval decision.
    ApprovalRequested,
    /// A Codex turn or task has completed.
    TaskCompleted,
}

impl EventKind {
    fn parse(value: &str) -> Result<Self, EventError> {
        match value {
            "approval_requested" => Ok(Self::ApprovalRequested),
            "task_completed" => Ok(Self::TaskCompleted),
            _ => Err(EventError::UnknownKind),
        }
    }
}

/// Notification urgency with platform-independent semantics.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Urgency {
    /// Normal delivery priority.
    Normal,
    /// High delivery priority for time-sensitive attention.
    High,
}

/// Privacy policy applied before calling a native notification adapter.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Privacy {
    /// Use generic private display text.
    Private,
    /// Allow bounded canonical display text.
    Public,
}

/// Sanitized source metadata that cannot affect transport routing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EventSource {
    host_label: String,
    project_label: Option<String>,
    session_id: Option<String>,
}

impl EventSource {
    /// Creates validated and NFC-normalized source metadata.
    ///
    /// # Errors
    ///
    /// Returns a classified field error when a label or identifier is empty,
    /// overlong, path-like, non-ASCII where required, or contains controls.
    pub fn new(
        host_label: impl Into<String>,
        project_label: Option<String>,
        session_id: Option<String>,
    ) -> Result<Self, EventError> {
        let host_label = host_label.into();
        let host_label = normalize_bounded(&host_label, MAX_HOST_LABEL_SCALARS, Field::HostLabel)?;
        if host_label.contains(['/', '\\']) {
            return Err(EventError::InvalidField(Field::HostLabel));
        }

        let project_label = project_label
            .map(|value| {
                let value =
                    normalize_bounded(&value, MAX_PROJECT_LABEL_SCALARS, Field::ProjectLabel)?;
                if looks_absolute_path(&value) {
                    return Err(EventError::InvalidField(Field::ProjectLabel));
                }
                Ok(value)
            })
            .transpose()?;

        let session_id = session_id
            .map(|value| validate_ascii_id(value, MAX_SESSION_ID_BYTES, Field::SessionId))
            .transpose()?;

        Ok(Self {
            host_label,
            project_label,
            session_id,
        })
    }

    /// Returns the sanitized host display label.
    #[must_use]
    pub fn host_label(&self) -> &str {
        &self.host_label
    }

    /// Returns the optional sanitized project display label.
    #[must_use]
    pub fn project_label(&self) -> Option<&str> {
        self.project_label.as_deref()
    }

    /// Returns the optional opaque or hashed session identifier.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

/// Bounded display text and presentation policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Presentation {
    title: String,
    body: String,
    urgency: Urgency,
    privacy: Privacy,
}

impl Presentation {
    /// Creates validated and NFC-normalized presentation data.
    ///
    /// # Errors
    ///
    /// Returns a classified field error when title or body is empty, overlong,
    /// or contains forbidden control characters.
    pub fn new(
        title: impl Into<String>,
        body: impl Into<String>,
        urgency: Urgency,
        privacy: Privacy,
    ) -> Result<Self, EventError> {
        let title = title.into();
        let body = body.into();
        Ok(Self {
            title: normalize_bounded(&title, MAX_TITLE_SCALARS, Field::Title)?,
            body: normalize_bounded(&body, MAX_BODY_SCALARS, Field::Body)?,
            urgency,
            privacy,
        })
    }

    /// Returns the canonical title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the canonical body.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Returns the requested urgency.
    #[must_use]
    pub const fn urgency(&self) -> Urgency {
        self.urgency
    }

    /// Returns the presentation privacy policy.
    #[must_use]
    pub const fn privacy(&self) -> Privacy {
        self.privacy
    }
}

/// A named desktop profile, never a network address or command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Routing {
    profile: String,
}

impl Routing {
    /// Creates a validated routing profile.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::InvalidRouting`] unless the profile matches the
    /// protocol version 1 ASCII grammar and length limit.
    pub fn new(profile: impl Into<String>) -> Result<Self, EventError> {
        let profile = profile.into();
        if profile.is_empty()
            || profile.len() > MAX_ROUTING_PROFILE_BYTES
            || !profile.is_ascii()
            || !profile
                .bytes()
                .enumerate()
                .all(|(index, byte)| valid_profile_byte(index, byte))
        {
            return Err(EventError::InvalidRouting);
        }
        Ok(Self { profile })
    }

    /// Returns the profile name.
    #[must_use]
    pub fn profile(&self) -> &str {
        &self.profile
    }
}

/// Bounded namespaced forward-compatible metadata.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Extensions(BTreeMap<String, Value>);

impl Extensions {
    /// Validates and normalizes an extension map.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::InvalidExtension`] when namespace, count, type,
    /// nesting, string, container, or encoded-size limits are violated.
    pub fn new(values: BTreeMap<String, Value>) -> Result<Self, EventError> {
        if values.len() > MAX_EXTENSION_ENTRIES {
            return Err(EventError::InvalidExtension);
        }

        let mut normalized = BTreeMap::new();
        for (key, value) in values {
            if !valid_extension_namespace(&key) {
                return Err(EventError::InvalidExtension);
            }
            normalized.insert(key, normalize_extension_value(value, 1)?);
        }

        let encoded = serde_json::to_vec(&normalized).map_err(|_| EventError::Serialization)?;
        if encoded.len() > MAX_EXTENSIONS_BYTES {
            return Err(EventError::InvalidExtension);
        }
        Ok(Self(normalized))
    }

    /// Returns the validated extension map.
    #[must_use]
    pub const fn as_map(&self) -> &BTreeMap<String, Value> {
        &self.0
    }
}

/// A fully validated protocol version 1 event.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CanonicalEvent {
    schema_version: u16,
    event_id: EventId,
    kind: EventKind,
    occurred_at: CanonicalTimestamp,
    source: EventSource,
    presentation: Presentation,
    routing: Option<Routing>,
    extensions: Extensions,
}

impl CanonicalEvent {
    /// Creates a canonical event at an initial-ingestion time.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::TimestampOutOfRange`] for an occurrence time
    /// outside the ingestion window, or a size/serialization error when the
    /// resulting envelope cannot satisfy protocol limits.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_id: EventId,
        kind: EventKind,
        occurred_at: OffsetDateTime,
        source: EventSource,
        presentation: Presentation,
        routing: Option<Routing>,
        extensions: Extensions,
        received_at: OffsetDateTime,
    ) -> Result<Self, EventError> {
        validate_timestamp_range(occurred_at, received_at)?;
        let event = Self {
            schema_version: SCHEMA_VERSION,
            event_id,
            kind,
            occurred_at: CanonicalTimestamp(occurred_at.to_offset(UtcOffset::UTC)),
            source,
            presentation,
            routing,
            extensions,
        };
        event.validate_encoded_size()?;
        Ok(event)
    }

    /// Parses and validates one duplicate-free protocol JSON object.
    ///
    /// # Errors
    ///
    /// Returns a stable [`EventError`] classification for size, JSON, shape,
    /// version, identifier, kind, timestamp, field, routing, or extension
    /// violations.
    pub fn from_json(input: &[u8], received_at: OffsetDateTime) -> Result<Self, EventError> {
        if input.len() > MAX_EVENT_BYTES {
            return Err(EventError::PayloadTooLarge);
        }
        let value = parse_unique(input)?;
        validate_wire_shape(&value)?;

        let object = value.as_object().ok_or(EventError::InvalidShape)?;
        let schema_version = object
            .get("schema_version")
            .and_then(Value::as_u64)
            .ok_or(EventError::InvalidShape)?;
        if schema_version != u64::from(SCHEMA_VERSION) {
            return Err(EventError::UnsupportedSchema);
        }
        let kind_text = object
            .get("kind")
            .and_then(Value::as_str)
            .ok_or(EventError::InvalidShape)?;
        let kind = EventKind::parse(kind_text)?;

        let wire: WireEvent =
            serde_json::from_value(value).map_err(|_| EventError::InvalidShape)?;
        let event_id = EventId::parse(&wire.event_id)?;
        let occurred_at = parse_timestamp(&wire.occurred_at)?;
        let source = EventSource::new(
            wire.source.host_label,
            wire.source.project_label,
            wire.source.session_id,
        )?;
        let presentation = Presentation::new(
            wire.presentation.title,
            wire.presentation.body,
            wire.presentation.urgency,
            wire.presentation.privacy,
        )?;
        let routing = wire
            .routing
            .map(|routing| Routing::new(routing.profile))
            .transpose()?;
        let extensions = Extensions::new(wire.extensions)?;
        Self::new(
            event_id,
            kind,
            occurred_at,
            source,
            presentation,
            routing,
            extensions,
            received_at,
        )
    }

    /// Serializes the validated event using the canonical compact JSON form.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::Serialization`] for an unexpected serializer
    /// failure or [`EventError::PayloadTooLarge`] if the encoded event exceeds
    /// the protocol limit.
    pub fn to_json(&self) -> Result<Vec<u8>, EventError> {
        let encoded = serde_json::to_vec(self).map_err(|_| EventError::Serialization)?;
        if encoded.len() > MAX_EVENT_BYTES {
            return Err(EventError::PayloadTooLarge);
        }
        Ok(encoded)
    }

    /// Returns the schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// Returns the stable event identifier.
    #[must_use]
    pub const fn event_id(&self) -> EventId {
        self.event_id
    }

    /// Returns the event kind.
    #[must_use]
    pub const fn kind(&self) -> EventKind {
        self.kind
    }

    /// Returns the UTC occurrence time.
    #[must_use]
    pub const fn occurred_at(&self) -> OffsetDateTime {
        self.occurred_at.0
    }

    /// Returns source metadata.
    #[must_use]
    pub const fn source(&self) -> &EventSource {
        &self.source
    }

    /// Returns presentation data.
    #[must_use]
    pub const fn presentation(&self) -> &Presentation {
        &self.presentation
    }

    /// Returns the optional route.
    #[must_use]
    pub const fn routing(&self) -> Option<&Routing> {
        self.routing.as_ref()
    }

    /// Returns extension metadata.
    #[must_use]
    pub const fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    fn validate_encoded_size(&self) -> Result<(), EventError> {
        if self.to_json()?.len() > MAX_EVENT_BYTES {
            Err(EventError::PayloadTooLarge)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CanonicalTimestamp(OffsetDateTime);

impl Serialize for CanonicalTimestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format_timestamp(self.0))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEvent {
    #[allow(dead_code)]
    schema_version: u16,
    event_id: String,
    #[allow(dead_code)]
    kind: String,
    occurred_at: String,
    source: WireSource,
    presentation: WirePresentation,
    routing: Option<WireRouting>,
    extensions: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSource {
    host_label: String,
    project_label: Option<String>,
    session_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePresentation {
    title: String,
    body: String,
    urgency: Urgency,
    privacy: Privacy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRouting {
    profile: String,
}

fn validate_wire_shape(value: &Value) -> Result<(), EventError> {
    reject_unknown_fields(value, EVENT_FIELDS)?;
    require_fields(value, EVENT_FIELDS)?;
    let object = value.as_object().ok_or(EventError::InvalidShape)?;

    let source = object.get("source").ok_or(EventError::InvalidShape)?;
    reject_unknown_fields(source, SOURCE_FIELDS)?;
    require_fields(source, SOURCE_FIELDS)?;

    let presentation = object.get("presentation").ok_or(EventError::InvalidShape)?;
    reject_unknown_fields(presentation, PRESENTATION_FIELDS)?;
    require_fields(presentation, PRESENTATION_FIELDS)?;

    if let Some(routing) = object.get("routing").filter(|value| !value.is_null()) {
        reject_unknown_fields(routing, ROUTING_FIELDS)?;
        require_fields(routing, ROUTING_FIELDS)?;
    }
    Ok(())
}

fn require_fields(value: &Value, required: &[&str]) -> Result<(), EventError> {
    let object = value.as_object().ok_or(EventError::InvalidShape)?;
    if required.iter().any(|field| !object.contains_key(*field)) {
        return Err(EventError::InvalidShape);
    }
    Ok(())
}

fn normalize_bounded(value: &str, max: usize, field: Field) -> Result<String, EventError> {
    let normalized: String = value.nfc().collect();
    let length = normalized.chars().count();
    if length > max {
        return Err(EventError::FieldTooLong(field));
    }
    if length == 0 || contains_control(&normalized) {
        return Err(EventError::InvalidField(field));
    }
    Ok(normalized)
}

fn validate_ascii_id(value: String, max: usize, field: Field) -> Result<String, EventError> {
    if value.len() > max {
        return Err(EventError::FieldTooLong(field));
    }
    if value.is_empty()
        || !value.is_ascii()
        || contains_control(&value)
        || value.contains(['/', '\\'])
    {
        return Err(EventError::InvalidField(field));
    }
    Ok(value)
}

fn contains_control(value: &str) -> bool {
    value.chars().any(|character| {
        let code = u32::from(character);
        code <= 0x1f || (0x7f..=0x9f).contains(&code)
    })
}

fn looks_absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.starts_with(['/', '\\'])
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
}

fn valid_profile_byte(index: usize, byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'_' | b'-'))
}

fn valid_extension_namespace(key: &str) -> bool {
    if !(3..=64).contains(&key.len()) || !key.is_ascii() {
        return false;
    }
    let mut segments = key.split('.');
    let Some(first) = segments.next() else {
        return false;
    };
    let Some(second) = segments.next() else {
        return false;
    };
    valid_extension_segment(first)
        && valid_extension_segment(second)
        && segments.all(valid_extension_segment)
}

fn valid_extension_segment(segment: &str) -> bool {
    !segment.is_empty()
        && (segment.as_bytes()[0].is_ascii_lowercase() || segment.as_bytes()[0].is_ascii_digit())
        && segment.bytes().skip(1).all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn normalize_extension_value(value: Value, depth: usize) -> Result<Value, EventError> {
    if depth > MAX_EXTENSION_DEPTH {
        return Err(EventError::InvalidExtension);
    }
    match value {
        Value::Null | Value::Bool(_) => Ok(value),
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                Ok(Value::Number(number))
            } else {
                Err(EventError::InvalidExtension)
            }
        }
        Value::String(value) => {
            let normalized: String = value.nfc().collect();
            if normalized.chars().count() > MAX_EXTENSION_STRING_SCALARS
                || contains_control(&normalized)
            {
                return Err(EventError::InvalidExtension);
            }
            Ok(Value::String(normalized))
        }
        Value::Array(values) => {
            if values.len() > MAX_EXTENSION_CONTAINER_ENTRIES {
                return Err(EventError::InvalidExtension);
            }
            values
                .into_iter()
                .map(|value| normalize_extension_value(value, depth + 1))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        Value::Object(values) => {
            if values.len() > MAX_EXTENSION_CONTAINER_ENTRIES {
                return Err(EventError::InvalidExtension);
            }
            let mut normalized = serde_json::Map::new();
            for (key, value) in values {
                let key: String = key.nfc().collect();
                if key.is_empty()
                    || key.chars().count() > MAX_EXTENSION_STRING_SCALARS
                    || contains_control(&key)
                    || normalized.contains_key(&key)
                {
                    return Err(EventError::InvalidExtension);
                }
                normalized.insert(key, normalize_extension_value(value, depth + 1)?);
            }
            Ok(Value::Object(normalized))
        }
    }
}

fn validate_timestamp_range(
    occurred_at: OffsetDateTime,
    received_at: OffsetDateTime,
) -> Result<(), EventError> {
    let oldest = received_at
        .checked_sub(Duration::seconds(MAX_PAST_AGE_SECONDS))
        .ok_or(EventError::TimestampOutOfRange)?;
    let newest = received_at
        .checked_add(Duration::seconds(MAX_FUTURE_SKEW_SECONDS))
        .ok_or(EventError::TimestampOutOfRange)?;
    if occurred_at < oldest || occurred_at > newest {
        return Err(EventError::TimestampOutOfRange);
    }
    Ok(())
}

fn parse_timestamp(value: &str) -> Result<OffsetDateTime, EventError> {
    let parsed =
        OffsetDateTime::parse(value, &Rfc3339).map_err(|_| EventError::InvalidTimestamp)?;
    if parsed.offset() != UtcOffset::UTC
        || parsed.nanosecond() % 1_000_000 != 0
        || format_timestamp(parsed) != value
    {
        return Err(EventError::InvalidTimestamp);
    }
    Ok(parsed)
}

fn format_timestamp(value: OffsetDateTime) -> String {
    let value = value.to_offset(UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        value.year(),
        u8::from(value.month()),
        value.day(),
        value.hour(),
        value.minute(),
        value.second(),
        value.millisecond(),
    )
}
