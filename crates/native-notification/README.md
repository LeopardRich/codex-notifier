# native-notification

Provides the policy boundary between canonical events and native desktop
notifications. Private mode always maps the two event kinds to fixed generic
text. Public mode requires both explicit application policy and public event
presentation. The canonical event ID, native title/body lengths, control
removal, urgency, and silent delivery are applied before a platform API is
called.

The Windows implementation uses safe `windows-rs` WinRT projections and builds
Toast XML through DOM nodes rather than payload interpolation. It validates the
product AUMID, rejects Session 0, checks `ToastNotifier.Setting`, and returns
separate identity, application-disabled, user-disabled, policy-disabled,
session, availability, and delivery classifications. Focus/do-not-disturb is
owned by Windows; the diagnostic reports `system_managed` instead of claiming
to know a state that the public Toast API does not expose.

The macOS implementation uses safe `usernotifications-rs` bindings to Apple's
modern UserNotifications framework. It verifies the fixed product bundle ID
and `.app` identity, checks the user's Aqua launch domain, and distinguishes
authorization-not-determined, denial/application disablement, missing identity,
missing GUI session, API unavailability, and delivery rejection. Event display
never requests authorization. Requests are display-only and keyed by the
canonical event ID. Silent delivery omits sound and uses a passive interruption
level; active delivery never requests time-sensitive or critical entitlement,
so Focus remains system-managed.

Windows and macOS code and dependencies are selected only for their target
platforms. The shared adapter has platform-independent fake-backend contracts.
Linux desktop notifications remain out of scope.

The ignored `windows_smoke` integration test deliberately displays one real
Toast for each event kind and must be invoked only in an interactive test
session. The ignored `macos_smoke` test creates an ad-hoc signed product-ID app
bundle, explicitly requests authorization, and submits one real notification
for each event kind in an interactive Aqua session.
