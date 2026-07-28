//! Protocol version 1 validation limits.

/// The only schema version accepted by this crate.
pub const SCHEMA_VERSION: u16 = 1;
/// Maximum encoded canonical event size in UTF-8 bytes.
pub const MAX_EVENT_BYTES: usize = 16_384;
/// Maximum encoded acknowledgement size in UTF-8 bytes.
pub const MAX_ACK_BYTES: usize = 2_048;
/// Maximum title length in Unicode scalar values.
pub const MAX_TITLE_SCALARS: usize = 120;
/// Maximum body length in Unicode scalar values.
pub const MAX_BODY_SCALARS: usize = 512;
/// Maximum host label length in Unicode scalar values.
pub const MAX_HOST_LABEL_SCALARS: usize = 64;
/// Maximum project label length in Unicode scalar values.
pub const MAX_PROJECT_LABEL_SCALARS: usize = 80;
/// Maximum session identifier length in ASCII bytes.
pub const MAX_SESSION_ID_BYTES: usize = 128;
/// Maximum routing profile length in ASCII bytes.
pub const MAX_ROUTING_PROFILE_BYTES: usize = 64;
/// Maximum number of top-level extension entries.
pub const MAX_EXTENSION_ENTRIES: usize = 32;
/// Maximum encoded extension object size in UTF-8 bytes.
pub const MAX_EXTENSIONS_BYTES: usize = 4_096;
/// Maximum extension object or array entries.
pub const MAX_EXTENSION_CONTAINER_ENTRIES: usize = 16;
/// Maximum extension nesting depth, counting the value as depth one.
pub const MAX_EXTENSION_DEPTH: usize = 4;
/// Maximum extension string or nested-key length in Unicode scalar values.
pub const MAX_EXTENSION_STRING_SCALARS: usize = 256;
/// Maximum accepted event age at initial ingestion, in seconds.
pub const MAX_PAST_AGE_SECONDS: i64 = 7 * 24 * 60 * 60;
/// Maximum accepted future clock skew at initial ingestion, in seconds.
pub const MAX_FUTURE_SKEW_SECONDS: i64 = 5 * 60;
