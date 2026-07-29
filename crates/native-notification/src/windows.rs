//! Windows Toast implementation through safe `WinRT` projections.

use std::fmt;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, get_current_pid};
use windows::Data::Xml::Dom::{XmlDocument, XmlElement};
use windows::UI::Notifications::{
    NotificationSetting, ToastNotification, ToastNotificationManager, ToastNotificationPriority,
    ToastNotifier,
};
use windows::core::{HRESULT, HSTRING};
use winreg::{
    RegKey,
    enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
};

use crate::{
    FocusStatus, NotificationBackend, NotificationDiagnostic, NotificationError,
    NotificationStatus, PreparedNotification,
};
use codex_notifier_core::Urgency;

/// Stable unpackaged application identity installed by Windows packaging.
pub const CODEX_NOTIFIER_APP_ID: &str = "LeopardRich.CodexNotifier";
const MAX_APP_ID_BYTES: usize = 128;
const ERROR_NOT_FOUND_HRESULT: i32 = -2_147_023_728;
const APP_ID_REGISTRY_PREFIX: &str = r"Software\Classes\AppUserModelId";
const WINDOWS_VERSION_REGISTRY_PATH: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

/// Strict Windows application user model identifier.
#[derive(Clone, Eq, PartialEq)]
pub struct WindowsApplicationId(String);

impl WindowsApplicationId {
    /// Validates the product-owned application identity syntax.
    ///
    /// # Errors
    ///
    /// Returns [`NotificationError::ApplicationIdentityMissing`] for empty,
    /// overlong, non-ASCII, path-like, control-containing, or punctuation-only
    /// input.
    pub fn new(value: impl Into<String>) -> Result<Self, NotificationError> {
        let value = value.into();
        let bytes = value.as_bytes();
        let valid = !bytes.is_empty()
            && bytes.len() <= MAX_APP_ID_BYTES
            && bytes[0].is_ascii_alphanumeric()
            && bytes[bytes.len() - 1].is_ascii_alphanumeric()
            && bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
        if !valid {
            return Err(NotificationError::ApplicationIdentityMissing);
        }
        Ok(Self(value))
    }

    /// Returns the validated application user model identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for WindowsApplicationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WindowsApplicationId(<product-id>)")
    }
}

/// `WinRT` Toast backend for an interactive Windows user session.
pub struct WindowsNotificationBackend {
    application_id: WindowsApplicationId,
    host: HostStatus,
    session: SessionStatus,
    registration: ApplicationRegistrationStatus,
}

impl WindowsNotificationBackend {
    /// Creates a backend for a packaging-owned application identity.
    ///
    /// The operating-system registration and notification setting are checked
    /// by [`NotificationBackend::diagnose`] before a delivery is accepted.
    #[must_use]
    pub fn new(application_id: WindowsApplicationId) -> Self {
        let registration = current_application_registration_status(&application_id);
        Self {
            application_id,
            host: current_host_status(),
            session: current_session_status(),
            registration,
        }
    }

    /// Creates the standard product backend.
    #[must_use]
    pub fn codex_notifier() -> Self {
        Self::new(WindowsApplicationId(CODEX_NOTIFIER_APP_ID.to_owned()))
    }

    fn notifier(&self) -> Result<ToastNotifier, NotificationError> {
        match self.session {
            SessionStatus::Interactive => {}
            SessionStatus::NonInteractive => {
                return Err(NotificationError::NoInteractiveSession);
            }
            SessionStatus::Unknown => return Err(NotificationError::Unavailable),
        }
        match self.host {
            HostStatus::Desktop => {}
            HostStatus::Server => return Err(NotificationError::UnsupportedPlatform),
            HostStatus::Unknown => return Err(NotificationError::Unavailable),
        }
        match self.registration {
            ApplicationRegistrationStatus::Registered => {}
            ApplicationRegistrationStatus::Missing => {
                return Err(NotificationError::ApplicationIdentityMissing);
            }
            ApplicationRegistrationStatus::Unknown => {
                return Err(NotificationError::Unavailable);
            }
        }
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(
            self.application_id.as_str(),
        ))
        .map_err(|error| map_query_error(error.code()))
    }

    fn ready_notifier(&self) -> Result<ToastNotifier, NotificationError> {
        let notifier = self.notifier()?;
        match notifier.Setting() {
            Ok(setting) => setting_error(setting)?,
            Err(error) => setting_query_error(error.code(), self.registration)?,
        }
        Ok(notifier)
    }
}

impl NotificationBackend for WindowsNotificationBackend {
    fn diagnose(&self) -> NotificationDiagnostic {
        let status = match self.ready_notifier() {
            Ok(_) => NotificationStatus::Ready,
            Err(error) => status_for_error(error),
        };
        NotificationDiagnostic::new(status, FocusStatus::SystemManaged)
    }

    fn show(&self, notification: &PreparedNotification) -> Result<(), NotificationError> {
        let notifier = self.ready_notifier()?;
        let document =
            build_document(notification).map_err(|_| NotificationError::DeliveryFailed)?;
        let toast = ToastNotification::CreateToastNotification(&document)
            .map_err(|_| NotificationError::DeliveryFailed)?;
        toast
            .SetSuppressPopup(notification.silent())
            .map_err(|_| NotificationError::DeliveryFailed)?;
        if notification.urgency() == Urgency::High {
            toast
                .SetPriority(ToastNotificationPriority::High)
                .map_err(|_| NotificationError::DeliveryFailed)?;
        }
        notifier
            .Show(&toast)
            .map_err(|_| NotificationError::DeliveryFailed)
    }
}

fn build_document(notification: &PreparedNotification) -> windows::core::Result<XmlDocument> {
    let document = XmlDocument::new()?;
    let toast = document.CreateElement(&HSTRING::from("toast"))?;
    toast.SetAttribute(
        &HSTRING::from("duration"),
        &HSTRING::from(if notification.urgency() == Urgency::High {
            "long"
        } else {
            "short"
        }),
    )?;
    let visual = document.CreateElement(&HSTRING::from("visual"))?;
    let binding = document.CreateElement(&HSTRING::from("binding"))?;
    binding.SetAttribute(&HSTRING::from("template"), &HSTRING::from("ToastGeneric"))?;
    append_text(&document, &binding, notification.title())?;
    append_text(&document, &binding, notification.body())?;
    visual.AppendChild(&binding)?;
    toast.AppendChild(&visual)?;
    if notification.silent() {
        let audio = document.CreateElement(&HSTRING::from("audio"))?;
        audio.SetAttribute(&HSTRING::from("silent"), &HSTRING::from("true"))?;
        toast.AppendChild(&audio)?;
    }
    document.AppendChild(&toast)?;
    Ok(document)
}

fn append_text(
    document: &XmlDocument,
    binding: &XmlElement,
    value: &str,
) -> windows::core::Result<()> {
    let text = document.CreateElement(&HSTRING::from("text"))?;
    let content = document.CreateTextNode(&HSTRING::from(value))?;
    text.AppendChild(&content)?;
    binding.AppendChild(&text)?;
    Ok(())
}

fn setting_error(setting: NotificationSetting) -> Result<(), NotificationError> {
    match setting {
        NotificationSetting::Enabled => Ok(()),
        NotificationSetting::DisabledForApplication => {
            Err(NotificationError::DisabledForApplication)
        }
        NotificationSetting::DisabledForUser => Err(NotificationError::DisabledForUser),
        NotificationSetting::DisabledByGroupPolicy => Err(NotificationError::DisabledByPolicy),
        NotificationSetting::DisabledByManifest => {
            Err(NotificationError::ApplicationIdentityMissing)
        }
        _ => Err(NotificationError::Unavailable),
    }
}

const fn setting_query_error(
    code: HRESULT,
    registration: ApplicationRegistrationStatus,
) -> Result<(), NotificationError> {
    if code.0 == ERROR_NOT_FOUND_HRESULT
        && matches!(registration, ApplicationRegistrationStatus::Registered)
    {
        Ok(())
    } else {
        Err(map_query_error(code))
    }
}

const fn map_query_error(code: HRESULT) -> NotificationError {
    if code.0 == ERROR_NOT_FOUND_HRESULT {
        NotificationError::ApplicationIdentityMissing
    } else {
        NotificationError::Unavailable
    }
}

const fn status_for_error(error: NotificationError) -> NotificationStatus {
    match error {
        NotificationError::UnsupportedPlatform => NotificationStatus::UnsupportedPlatform,
        NotificationError::ApplicationIdentityMissing => {
            NotificationStatus::ApplicationIdentityMissing
        }
        NotificationError::AuthorizationNotDetermined => {
            NotificationStatus::AuthorizationNotDetermined
        }
        NotificationError::DisabledForApplication => NotificationStatus::DisabledForApplication,
        NotificationError::DisabledForUser => NotificationStatus::DisabledForUser,
        NotificationError::DisabledByPolicy => NotificationStatus::DisabledByPolicy,
        NotificationError::NoInteractiveSession => NotificationStatus::NoInteractiveSession,
        NotificationError::Unavailable | NotificationError::DeliveryFailed => {
            NotificationStatus::Unavailable
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionStatus {
    Interactive,
    NonInteractive,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostStatus {
    Desktop,
    Server,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ApplicationRegistrationStatus {
    Registered,
    Missing,
    Unknown,
}

fn current_application_registration_status(
    application_id: &WindowsApplicationId,
) -> ApplicationRegistrationStatus {
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let path = format!(r"{APP_ID_REGISTRY_PREFIX}\{}", application_id.as_str());
    match current_user.open_subkey(path) {
        Ok(_) => ApplicationRegistrationStatus::Registered,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            ApplicationRegistrationStatus::Missing
        }
        Err(_) => ApplicationRegistrationStatus::Unknown,
    }
}

fn current_host_status() -> HostStatus {
    let local_machine = RegKey::predef(HKEY_LOCAL_MACHINE);
    let installation_type = local_machine
        .open_subkey(WINDOWS_VERSION_REGISTRY_PATH)
        .and_then(|key| key.get_value::<String, _>("InstallationType"))
        .ok();
    classify_installation_type(installation_type.as_deref())
}

fn classify_installation_type(value: Option<&str>) -> HostStatus {
    match value {
        Some("Client") => HostStatus::Desktop,
        Some("Server" | "Server Core") => HostStatus::Server,
        _ => HostStatus::Unknown,
    }
}

fn current_session_status() -> SessionStatus {
    let Ok(pid) = get_current_pid() else {
        return SessionStatus::Unknown;
    };
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    match system
        .process(pid)
        .and_then(sysinfo::Process::session_id)
        .map(sysinfo::Pid::as_u32)
    {
        Some(0) => SessionStatus::NonInteractive,
        Some(_) => SessionStatus::Interactive,
        None => SessionStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NotificationContentPolicy, NotificationPolicy};
    use codex_notifier_core::{
        CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy,
    };
    use std::collections::BTreeMap;
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};

    fn prepared(title: &str, body: &str, quiet: bool) -> PreparedNotification {
        let time = OffsetDateTime::parse("2026-07-29T04:00:00.000Z", &Rfc3339).expect("valid time");
        let event = CanonicalEvent::new(
            EventId::parse("01983c8d-b800-7000-8000-000000000012").expect("UUIDv7"),
            EventKind::TaskCompleted,
            time,
            EventSource::new("workstation", None, None).expect("source"),
            Presentation::new(title, body, Urgency::High, Privacy::Public).expect("presentation"),
            None,
            Extensions::new(BTreeMap::new()).expect("extensions"),
            time,
        )
        .expect("event");
        crate::prepare(
            &event,
            NotificationPolicy::new(NotificationContentPolicy::Public, quiet),
        )
    }

    #[test]
    fn application_identity_is_strict_and_redacted_in_debug() {
        let identity = WindowsApplicationId::new(CODEX_NOTIFIER_APP_ID).expect("product ID");
        assert_eq!(identity.as_str(), CODEX_NOTIFIER_APP_ID);
        assert_eq!(
            format!("{identity:?}"),
            "WindowsApplicationId(<product-id>)"
        );
        for invalid in [
            "",
            ".invalid",
            "invalid.",
            "has space",
            "path\\value",
            "bad!",
        ] {
            assert_eq!(
                WindowsApplicationId::new(invalid),
                Err(NotificationError::ApplicationIdentityMissing)
            );
        }
        assert_eq!(
            WindowsApplicationId::new("a".repeat(MAX_APP_ID_BYTES + 1)),
            Err(NotificationError::ApplicationIdentityMissing)
        );
    }

    #[test]
    fn notification_settings_have_distinct_diagnostics() {
        for (setting, error, status) in [
            (
                NotificationSetting::DisabledForApplication,
                NotificationError::DisabledForApplication,
                NotificationStatus::DisabledForApplication,
            ),
            (
                NotificationSetting::DisabledForUser,
                NotificationError::DisabledForUser,
                NotificationStatus::DisabledForUser,
            ),
            (
                NotificationSetting::DisabledByGroupPolicy,
                NotificationError::DisabledByPolicy,
                NotificationStatus::DisabledByPolicy,
            ),
            (
                NotificationSetting::DisabledByManifest,
                NotificationError::ApplicationIdentityMissing,
                NotificationStatus::ApplicationIdentityMissing,
            ),
        ] {
            assert_eq!(setting_error(setting), Err(error));
            assert_eq!(status_for_error(error), status);
        }
        assert_eq!(setting_error(NotificationSetting::Enabled), Ok(()));
        assert_eq!(
            setting_error(NotificationSetting(99)),
            Err(NotificationError::Unavailable)
        );
        assert_eq!(
            map_query_error(HRESULT(ERROR_NOT_FOUND_HRESULT)),
            NotificationError::ApplicationIdentityMissing
        );
        assert_eq!(
            setting_query_error(
                HRESULT(ERROR_NOT_FOUND_HRESULT),
                ApplicationRegistrationStatus::Registered,
            ),
            Ok(())
        );
        assert_eq!(
            setting_query_error(
                HRESULT(ERROR_NOT_FOUND_HRESULT),
                ApplicationRegistrationStatus::Missing,
            ),
            Err(NotificationError::ApplicationIdentityMissing)
        );
        assert_eq!(
            map_query_error(HRESULT(-2_147_467_259)),
            NotificationError::Unavailable
        );
    }

    #[test]
    fn windows_server_is_outside_the_desktop_notification_scope() {
        assert_eq!(
            classify_installation_type(Some("Client")),
            HostStatus::Desktop
        );
        assert_eq!(
            classify_installation_type(Some("Server")),
            HostStatus::Server
        );
        assert_eq!(
            classify_installation_type(Some("Server Core")),
            HostStatus::Server
        );
        assert_eq!(classify_installation_type(None), HostStatus::Unknown);
    }

    #[test]
    fn noninteractive_session_diagnostic_precedes_host_classification() {
        let backend = WindowsNotificationBackend {
            application_id: WindowsApplicationId(CODEX_NOTIFIER_APP_ID.to_owned()),
            host: HostStatus::Server,
            session: SessionStatus::NonInteractive,
            registration: ApplicationRegistrationStatus::Missing,
        };

        assert!(matches!(
            backend.notifier(),
            Err(NotificationError::NoInteractiveSession)
        ));
    }

    #[test]
    fn dom_builder_escapes_xml_and_never_adds_actions() {
        let notification = prepared("A <tag> & value", "quotes: \" ' >", false);
        let xml = build_document(&notification)
            .expect("valid document")
            .GetXml()
            .expect("serialized XML")
            .to_string();
        assert!(xml.contains("A &lt;tag&gt; &amp; value"));
        assert!(xml.contains("quotes: \" ' &gt;"));
        assert!(!xml.contains("<action"));
        assert!(!xml.contains("activationType"));
        assert!(!xml.contains("launch="));
        assert!(!xml.contains("<audio"));
    }

    #[test]
    fn silent_dom_suppresses_audio_and_popup_policy_is_bounded() {
        let notification = prepared("title", "body", true);
        assert!(notification.silent());
        let xml = build_document(&notification)
            .expect("valid document")
            .GetXml()
            .expect("serialized XML")
            .to_string();
        assert!(xml.contains("<audio silent=\"true\""));
        assert!(xml.contains("duration=\"long\""));
    }

    #[test]
    fn current_test_process_has_a_classified_session() {
        assert_ne!(current_session_status(), SessionStatus::Unknown);
    }
}
