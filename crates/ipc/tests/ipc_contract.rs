//! Real local endpoint, acknowledgement, concurrency, stale recovery, and proxy tests.

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration as StdDuration;

use codex_notifier_core::{
    CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Urgency,
};
use codex_notifier_ipc::{
    AckError, AckStatus, Acknowledgement, IpcClient, IpcEndpoint, IpcError, IpcPolicy, IpcServer,
};
use tempfile::TempDir;
use time::{Duration, OffsetDateTime};
use tokio::sync::oneshot;

const UUID_V7: &str = "01890f4d-e000-7000-8000-000000000000";
static NEXT_PROFILE: AtomicUsize = AtomicUsize::new(0);

fn event() -> CanonicalEvent {
    let now = OffsetDateTime::now_utc();
    CanonicalEvent::new(
        EventId::parse(UUID_V7).expect("fixture UUIDv7"),
        EventKind::TaskCompleted,
        now - Duration::seconds(1),
        EventSource::new("workstation", Some("project".to_owned()), None).expect("fixture source"),
        Presentation::new("Title", "Body", Urgency::Normal, Privacy::Private)
            .expect("fixture presentation"),
        None,
        Extensions::new(BTreeMap::new()).expect("fixture extensions"),
        now,
    )
    .expect("fixture event")
}

fn endpoint() -> (TempDir, IpcEndpoint) {
    #[cfg(unix)]
    let directory = tempfile::Builder::new()
        .prefix("cn")
        .tempdir_in("/tmp")
        .expect("short temporary directory");
    #[cfg(windows)]
    let directory = tempfile::tempdir().expect("temporary directory");
    let profile = format!(
        "t{}_{}",
        std::process::id(),
        NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
    );
    let endpoint = IpcEndpoint::new(directory.path(), profile).expect("endpoint");
    (directory, endpoint)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn valid_event_completes_structured_acknowledgement() {
    let (_directory, endpoint) = endpoint();
    let server = Arc::new(IpcServer::bind(endpoint.clone(), IpcPolicy::default()).expect("server"));
    assert_eq!(server.endpoint(), &endpoint);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .serve_until(
                    Arc::new(|event: CanonicalEvent| {
                        Acknowledgement::success(event.event_id(), AckStatus::Accepted)
                            .expect("valid acknowledgement")
                    }),
                    async {
                        let _ = shutdown_rx.await;
                    },
                )
                .await
        })
    };

    let acknowledgement = IpcClient::new(endpoint, IpcPolicy::default())
        .submit(&event())
        .await
        .expect("submit event");
    assert_eq!(acknowledgement.status(), AckStatus::Accepted);
    assert_eq!(acknowledgement.event_id().to_string(), UUID_V7);
    shutdown_tx.send(()).expect("shutdown");
    let report = server_task.await.expect("server task").expect("server run");
    assert_eq!(report.completed, 1);
    assert_eq!(report.rejected, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_connections_never_exceed_the_configured_bound() {
    let (_directory, endpoint) = endpoint();
    let policy =
        IpcPolicy::new(StdDuration::from_secs(2), StdDuration::from_secs(2), 4).expect("policy");
    let server = Arc::new(IpcServer::bind(endpoint.clone(), policy).expect("server"));
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let handler = {
        let active = Arc::clone(&active);
        let maximum = Arc::clone(&maximum);
        Arc::new(move |event: CanonicalEvent| {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            maximum.fetch_max(current, Ordering::SeqCst);
            std::thread::sleep(StdDuration::from_millis(20));
            active.fetch_sub(1, Ordering::SeqCst);
            Acknowledgement::success(event.event_id(), AckStatus::Accepted)
                .expect("valid acknowledgement")
        })
    };
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .serve_until(handler, async {
                    let _ = shutdown_rx.await;
                })
                .await
        })
    };

    let mut clients = Vec::new();
    for _ in 0..16 {
        let client = IpcClient::new(endpoint.clone(), policy);
        clients.push(tokio::spawn(async move { client.submit(&event()).await }));
    }
    for client in clients {
        assert_eq!(
            client
                .await
                .expect("client task")
                .expect("submission")
                .status(),
            AckStatus::Accepted
        );
    }
    shutdown_tx.send(()).expect("shutdown");
    let report = server_task.await.expect("server task").expect("server run");
    assert_eq!(report.completed, 16);
    assert!(maximum.load(Ordering::SeqCst) <= 4);
}

#[test]
fn acknowledgement_validation_rejects_injection_and_inconsistent_status() {
    assert_eq!(
        AckError::new("bad\nfield", false, "Fixed message"),
        Err(IpcError::MalformedAcknowledgement)
    );
    assert_eq!(
        AckError::new("safe_code", false, "forged\u{1b}[2J"),
        Err(IpcError::MalformedAcknowledgement)
    );
    assert_eq!(
        Acknowledgement::success(
            EventId::parse(UUID_V7).expect("fixture ID"),
            AckStatus::Rejected,
        ),
        Err(IpcError::MalformedAcknowledgement)
    );
}

#[tokio::test]
async fn active_endpoint_cannot_be_displaced() {
    let (_directory, endpoint) = endpoint();
    let _server = IpcServer::bind(endpoint.clone(), IpcPolicy::default()).expect("first server");
    assert!(matches!(
        IpcServer::bind(endpoint, IpcPolicy::default()),
        Err(IpcError::AlreadyRunning)
    ));
}

#[tokio::test]
async fn absent_endpoint_connection_is_bounded() {
    let (_directory, endpoint) = endpoint();
    let policy =
        IpcPolicy::new(StdDuration::from_millis(50), StdDuration::from_secs(1), 1).expect("policy");
    let started = std::time::Instant::now();
    assert!(matches!(
        IpcClient::new(endpoint, policy).submit(&event()).await,
        Err(IpcError::ConnectionFailed | IpcError::Timeout)
    ));
    assert!(started.elapsed() < StdDuration::from_secs(5));
}

#[cfg(unix)]
#[tokio::test]
async fn stale_owned_socket_recovers_but_unrelated_file_is_preserved() {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    let (_directory, endpoint) = endpoint();
    std::fs::create_dir_all(endpoint.runtime_dir()).expect("runtime directory");
    let socket_path = endpoint
        .runtime_dir()
        .join(format!("codex-notifier-{}.sock", endpoint.profile()));
    let stale = UnixListener::bind(&socket_path).expect("stale listener");
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
        .expect("secure stale socket permissions");
    drop(stale);
    let server = IpcServer::bind(endpoint.clone(), IpcPolicy::default()).expect("recover stale");
    drop(server);

    std::fs::write(&socket_path, b"unrelated").expect("unrelated file");
    assert!(matches!(
        IpcServer::bind(endpoint, IpcPolicy::default()),
        Err(IpcError::InsecureEndpoint)
    ));
    assert_eq!(
        std::fs::read(socket_path).expect("file preserved"),
        b"unrelated"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proxy_environment_is_ignored_by_a_real_child_client() {
    let (_directory, endpoint) = endpoint();
    let server = Arc::new(IpcServer::bind(endpoint.clone(), IpcPolicy::default()).expect("server"));
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .serve_until(
                    Arc::new(|event: CanonicalEvent| {
                        Acknowledgement::success(event.event_id(), AckStatus::Accepted)
                            .expect("acknowledgement")
                    }),
                    async {
                        let _ = shutdown_rx.await;
                    },
                )
                .await
        })
    };

    let executable = std::env::current_exe().expect("test executable");
    let runtime_dir = endpoint.runtime_dir().to_owned();
    let profile = endpoint.profile().to_owned();
    let status = tokio::task::spawn_blocking(move || {
        Command::new(executable)
            .arg("--exact")
            .arg("proxy_child_submission")
            .arg("--nocapture")
            .env("CODEX_NOTIFIER_PROXY_CHILD", "1")
            .env("CODEX_NOTIFIER_TEST_RUNTIME", runtime_dir)
            .env("CODEX_NOTIFIER_TEST_PROFILE", profile)
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "socks5://127.0.0.1:1")
            .status()
    })
    .await
    .expect("child task")
    .expect("child process");
    assert!(status.success());
    shutdown_tx.send(()).expect("shutdown");
    let report = server_task.await.expect("server task").expect("server run");
    assert_eq!(report.completed, 1);
}

#[test]
fn proxy_child_submission() {
    if std::env::var_os("CODEX_NOTIFIER_PROXY_CHILD").is_none() {
        return;
    }
    assert!(std::env::var_os("HTTP_PROXY").is_some());
    assert!(std::env::var_os("HTTPS_PROXY").is_some());
    assert!(std::env::var_os("ALL_PROXY").is_some());
    let runtime = std::env::var_os("CODEX_NOTIFIER_TEST_RUNTIME").expect("runtime path");
    let profile = std::env::var("CODEX_NOTIFIER_TEST_PROFILE").expect("profile");
    let endpoint = IpcEndpoint::new(runtime, profile).expect("child endpoint");
    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime");
    let acknowledgement = runtime
        .block_on(IpcClient::new(endpoint, IpcPolicy::default()).submit(&event()))
        .expect("proxy-independent submission");
    assert_eq!(acknowledgement.status(), AckStatus::Accepted);
}

#[test]
fn invalid_endpoint_and_policy_boundaries_are_rejected() {
    let directory = tempfile::tempdir().expect("temporary directory");
    assert_eq!(
        IpcEndpoint::new(directory.path(), "-invalid"),
        Err(IpcError::InvalidEndpoint)
    );
    assert_eq!(
        IpcPolicy::new(StdDuration::from_millis(9), StdDuration::from_secs(1), 1,),
        Err(IpcError::InvalidEndpoint)
    );
    assert_eq!(
        IpcPolicy::new(StdDuration::from_secs(1), StdDuration::from_secs(1), 257,),
        Err(IpcError::InvalidEndpoint)
    );
}
