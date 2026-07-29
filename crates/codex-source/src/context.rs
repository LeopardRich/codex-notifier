//! Trusted source labels shared by versioned Codex adapters.

use codex_notifier_core::{EventSource, Routing};

use crate::SourceError;

/// Adapter-owned labels and optional route, never values copied from payloads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceContext {
    pub(crate) host_label: String,
    pub(crate) project_label: Option<String>,
    pub(crate) routing: Option<Routing>,
}

impl SourceContext {
    /// Creates validated non-payload source context.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::InvalidContext`] for invalid display labels or
    /// routing profiles.
    pub fn new(
        host_label: impl Into<String>,
        project_label: Option<String>,
        routing_profile: Option<String>,
    ) -> Result<Self, SourceError> {
        let host_label = host_label.into();
        EventSource::new(host_label.clone(), project_label.clone(), None)
            .map_err(|_| SourceError::InvalidContext)?;
        let routing = routing_profile
            .map(Routing::new)
            .transpose()
            .map_err(|_| SourceError::InvalidContext)?;
        Ok(Self {
            host_label,
            project_label,
            routing,
        })
    }
}
