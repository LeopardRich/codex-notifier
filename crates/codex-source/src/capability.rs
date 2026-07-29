//! Evidence-gated Codex event capability reporting.

use crate::{CodexCliVersion, CodexInterface};

/// Availability of one event on an exact Codex version and interface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CapabilityAvailability {
    /// A real external-process fixture selects an implemented adapter.
    Supported,
    /// The interface is documented but lacks the required real fixture.
    Unverified,
    /// The exact Codex release has no fixture-gated support.
    UnsupportedVersion,
    /// The event is not supported through this verified interface.
    UnsupportedInterface,
}

impl CapabilityAvailability {
    /// Returns the stable machine-readable capability state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Unverified => "unverified",
            Self::UnsupportedVersion => "unsupported_version",
            Self::UnsupportedInterface => "unsupported_interface",
        }
    }
}

/// Safe installation action for approval notifications.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ApprovalInstallation {
    /// Configure a client for the fixture-verified app-server request.
    ConfigureAppServer,
    /// Report unavailability without installing hooks or fallback scrapers.
    ReportUnavailable,
}

/// Read-only capability result shared by future installer and doctor commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodexCapabilityReport {
    version: Option<CodexCliVersion>,
    interface: CodexInterface,
    task_completed: CapabilityAvailability,
    approval_requested: CapabilityAvailability,
}

impl CodexCapabilityReport {
    /// Inspects one exact version/interface pair without retaining unknown text.
    #[must_use]
    pub fn inspect(version: &str, interface: CodexInterface) -> Self {
        let Ok(version) = version.parse::<CodexCliVersion>() else {
            return Self {
                version: None,
                interface,
                task_completed: CapabilityAvailability::UnsupportedVersion,
                approval_requested: CapabilityAvailability::UnsupportedVersion,
            };
        };
        let (task_completed, approval_requested) = match interface {
            CodexInterface::CliHook => (
                CapabilityAvailability::Supported,
                CapabilityAvailability::Unverified,
            ),
            CodexInterface::AppServer => (
                CapabilityAvailability::UnsupportedInterface,
                CapabilityAvailability::Supported,
            ),
        };
        Self {
            version: Some(version),
            interface,
            task_completed,
            approval_requested,
        }
    }

    /// Returns the recognized exact version, or none for unsupported input.
    #[must_use]
    pub const fn version(self) -> Option<CodexCliVersion> {
        self.version
    }

    /// Returns the inspected external interface.
    #[must_use]
    pub const fn interface(self) -> CodexInterface {
        self.interface
    }

    /// Returns task-completion availability.
    #[must_use]
    pub const fn task_completed(self) -> CapabilityAvailability {
        self.task_completed
    }

    /// Returns approval-request availability.
    #[must_use]
    pub const fn approval_requested(self) -> CapabilityAvailability {
        self.approval_requested
    }

    /// Returns whether installation may configure approval ingestion.
    #[must_use]
    pub const fn approval_installation(self) -> ApprovalInstallation {
        match self.approval_requested {
            CapabilityAvailability::Supported => ApprovalInstallation::ConfigureAppServer,
            CapabilityAvailability::Unverified
            | CapabilityAvailability::UnsupportedVersion
            | CapabilityAvailability::UnsupportedInterface => {
                ApprovalInstallation::ReportUnavailable
            }
        }
    }

    /// Returns a fixed installation notice that contains no supplied version.
    #[must_use]
    pub const fn approval_installation_notice(self) -> &'static str {
        match self.approval_requested {
            CapabilityAvailability::Supported => {
                "Configure display-only approval notifications through Codex app-server."
            }
            CapabilityAvailability::Unverified => {
                "Approval notifications are unavailable for the CLI hook; no approval hook will be installed."
            }
            CapabilityAvailability::UnsupportedVersion => {
                "Approval notifications are unavailable for this Codex version."
            }
            CapabilityAvailability::UnsupportedInterface => {
                "Approval notifications are unavailable through this Codex interface."
            }
        }
    }
}
