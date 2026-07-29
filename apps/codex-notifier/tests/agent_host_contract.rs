//! Real IPC/SQLite composition, single-instance, and shutdown contract tests.

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use codex_notifier::{
    AgentHost, ApprovalRequestedEmitter, EmitError, HostError, TaskCompletedEmitter, database_path,
};
use codex_notifier_application::{
    AgentError, AgentLease, AgentPolicy, AgentQueue, AgentQueueError, AgentRuntime, AgentState,
    CancellationToken, DeliveryFailure, DeliveryFuture, DeliveryOutcome, EnqueueResult,
    EventDelivery, RetryResult, RoleDeliveryFactory, RuntimeRole, SafeErrorCode,
};
use codex_notifier_codex_source::{ApprovalRequestedContext, SourceError, TaskCompletedContext};
use codex_notifier_config::{
    CliOverrides, Config, ConfigLoader, PathEnvironment, Platform, StateDirectoryProbe,
};
use codex_notifier_core::{
    CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Urgency,
};
use codex_notifier_ipc::{AckStatus, IpcClient, IpcEndpoint, IpcError, IpcPolicy};
use tempfile::TempDir;
use time::{Duration, OffsetDateTime};
use tokio::sync::oneshot;

const HOOK_RETURN_LIMIT: StdDuration = StdDuration::from_secs(5);
const BATCH_DELIVERY_LIMIT: StdDuration = StdDuration::from_secs(10);
const PROCESS_RSS_LIMIT_BYTES: u64 = 512 * 1024 * 1024;
const DATABASE_SIZE_LIMIT_BYTES: u64 = 8 * 1024 * 1024;

static NEXT_PROFILE: AtomicUsize = AtomicUsize::new(0);

fn event(index: usize) -> CanonicalEvent {
    let now = OffsetDateTime::now_utc();
    let id = format!("01890f4d-e000-7000-8000-{index:012x}");
    CanonicalEvent::new(
        EventId::parse(&id).expect("fixture UUIDv7"),
        EventKind::TaskCompleted,
        now - Duration::seconds(1),
        EventSource::new("workstation", Some("project".to_owned()), None).expect("source"),
        Presentation::new("Title", "Body", Urgency::Normal, Privacy::Private)
            .expect("presentation"),
        None,
        Extensions::new(BTreeMap::new()).expect("extensions"),
        now,
    )
    .expect("event")
}

#[cfg(unix)]
fn test_directory() -> TempDir {
    tempfile::Builder::new()
        .prefix("cnh")
        .tempdir_in("/tmp")
        .expect("short temporary directory")
}

#[cfg(windows)]
fn test_directory() -> TempDir {
    tempfile::tempdir().expect("temporary directory")
}

fn endpoint(directory: &TempDir) -> IpcEndpoint {
    let profile = format!(
        "h{}_{}",
        std::process::id(),
        NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
    );
    IpcEndpoint::new(directory.path().join("run"), profile).expect("endpoint")
}

struct TestDelivery {
    cancel_aware: bool,
    retryable_failures: AtomicUsize,
    attempts: AtomicUsize,
    active: AtomicUsize,
    delivered: AtomicUsize,
}

impl TestDelivery {
    fn immediate() -> Self {
        Self {
            cancel_aware: false,
            retryable_failures: AtomicUsize::new(0),
            attempts: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            delivered: AtomicUsize::new(0),
        }
    }

    fn cancel_aware() -> Self {
        Self {
            cancel_aware: true,
            retryable_failures: AtomicUsize::new(0),
            attempts: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            delivered: AtomicUsize::new(0),
        }
    }

    fn retry_once() -> Self {
        Self {
            cancel_aware: false,
            retryable_failures: AtomicUsize::new(1),
            attempts: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            delivered: AtomicUsize::new(0),
        }
    }

    fn always_retry() -> Self {
        Self {
            cancel_aware: false,
            retryable_failures: AtomicUsize::new(usize::MAX),
            attempts: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            delivered: AtomicUsize::new(0),
        }
    }
}

impl EventDelivery for TestDelivery {
    fn deliver<'a>(
        &'a self,
        _event: &'a CanonicalEvent,
        cancellation: CancellationToken,
    ) -> DeliveryFuture<'a> {
        Box::pin(async move {
            self.attempts.fetch_add(1, Ordering::AcqRel);
            self.active.fetch_add(1, Ordering::AcqRel);
            if self.cancel_aware {
                cancellation.cancelled().await;
                self.active.fetch_sub(1, Ordering::AcqRel);
                DeliveryOutcome::Cancelled
            } else if self
                .retryable_failures
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                    value.checked_sub(1)
                })
                .is_ok()
            {
                self.active.fetch_sub(1, Ordering::AcqRel);
                DeliveryOutcome::Failed(DeliveryFailure::new(
                    SafeErrorCode::parse("ssh_network_unavailable").expect("safe error code"),
                    true,
                ))
            } else {
                self.delivered.fetch_add(1, Ordering::AcqRel);
                self.active.fetch_sub(1, Ordering::AcqRel);
                DeliveryOutcome::Delivered
            }
        })
    }
}

struct TestFactory {
    delivery: Arc<TestDelivery>,
    desktop_calls: AtomicUsize,
    relay_calls: AtomicUsize,
}

impl TestFactory {
    fn new(delivery: TestDelivery) -> Self {
        Self {
            delivery: Arc::new(delivery),
            desktop_calls: AtomicUsize::new(0),
            relay_calls: AtomicUsize::new(0),
        }
    }
}

impl RoleDeliveryFactory for TestFactory {
    fn desktop(&self) -> Result<Arc<dyn EventDelivery>, AgentError> {
        self.desktop_calls.fetch_add(1, Ordering::AcqRel);
        Ok(self.delivery.clone())
    }

    fn relay(&self) -> Result<Arc<dyn EventDelivery>, AgentError> {
        self.relay_calls.fetch_add(1, Ordering::AcqRel);
        Ok(self.delivery.clone())
    }
}

struct MemoryQueue {
    queued: Mutex<VecDeque<CanonicalEvent>>,
    leased: Mutex<BTreeMap<String, CanonicalEvent>>,
    submitted: Mutex<Vec<CanonicalEvent>>,
    next: AtomicUsize,
}

impl MemoryQueue {
    fn new() -> Self {
        Self {
            queued: Mutex::new(VecDeque::new()),
            leased: Mutex::new(BTreeMap::new()),
            submitted: Mutex::new(Vec::new()),
            next: AtomicUsize::new(0),
        }
    }

    fn counts(&self) -> (usize, usize) {
        (
            self.queued.lock().expect("queued").len(),
            self.leased.lock().expect("leased").len(),
        )
    }

    fn submissions(&self) -> Vec<CanonicalEvent> {
        self.submitted.lock().expect("submitted").clone()
    }
}

impl AgentQueue for MemoryQueue {
    fn enqueue(&self, event: &CanonicalEvent) -> Result<EnqueueResult, AgentQueueError> {
        self.submitted
            .lock()
            .map_err(|_| AgentQueueError::Unavailable)?
            .push(event.clone());
        self.queued
            .lock()
            .map_err(|_| AgentQueueError::Unavailable)?
            .push_back(event.clone());
        Ok(EnqueueResult::Enqueued)
    }

    fn lease(&self, worker: usize) -> Result<Option<AgentLease>, AgentQueueError> {
        let Some(event) = self
            .queued
            .lock()
            .map_err(|_| AgentQueueError::Unavailable)?
            .pop_front()
        else {
            return Ok(None);
        };
        let token = format!("w{worker}_{}", self.next.fetch_add(1, Ordering::Relaxed));
        self.leased
            .lock()
            .map_err(|_| AgentQueueError::Unavailable)?
            .insert(token.clone(), event.clone());
        AgentLease::new(event, token, 1).map(Some)
    }

    fn acknowledge(&self, lease: &AgentLease) -> Result<(), AgentQueueError> {
        self.leased
            .lock()
            .map_err(|_| AgentQueueError::Unavailable)?
            .remove(lease.token())
            .ok_or(AgentQueueError::Corrupt)?;
        Ok(())
    }

    fn retry(
        &self,
        lease: &AgentLease,
        code: &SafeErrorCode,
    ) -> Result<RetryResult, AgentQueueError> {
        self.release(lease, code)?;
        Ok(RetryResult::Scheduled)
    }

    fn release(&self, lease: &AgentLease, _code: &SafeErrorCode) -> Result<(), AgentQueueError> {
        let event = self
            .leased
            .lock()
            .map_err(|_| AgentQueueError::Unavailable)?
            .remove(lease.token())
            .ok_or(AgentQueueError::Corrupt)?;
        self.queued
            .lock()
            .map_err(|_| AgentQueueError::Unavailable)?
            .push_back(event);
        Ok(())
    }

    fn dead_letter(
        &self,
        lease: &AgentLease,
        _code: &SafeErrorCode,
    ) -> Result<(), AgentQueueError> {
        self.acknowledge(lease)
    }
}

fn task_completed_payload() -> Vec<u8> {
    let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/codex-0.144.5-windows-cli-task-completed.json"
    ))
    .expect("task-completion fixture");
    let mut payload = fixture.get("payload").cloned().expect("fixture payload");
    payload
        .as_object_mut()
        .expect("payload object")
        .remove("observed_keys");
    serde_json::to_vec(&payload).expect("hook payload")
}

fn approval_requested_payload() -> Vec<u8> {
    let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/codex-0.144.5-windows-cli-approval-requested.json"
    ))
    .expect("approval-request fixture");
    let mut payload = fixture.get("payload").cloned().expect("fixture payload");
    payload
        .as_object_mut()
        .expect("payload object")
        .remove("observed_keys");
    payload["params"]
        .as_object_mut()
        .expect("params object")
        .remove("observed_keys");
    payload["params"]["startedAtMs"] =
        serde_json::json!(OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000);
    serde_json::to_vec(&payload).expect("app-server payload")
}

async fn run_emit_child(
    event_name: &str,
    payload: &[u8],
    directory: &TempDir,
    ipc_profile: &str,
) -> StdDuration {
    let started = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_codex-notifier"))
        .args([
            "emit",
            event_name,
            "--codex-version",
            "0.144.5",
            "--state-dir",
        ])
        .arg(directory.path())
        .args([
            "--ipc-profile",
            ipc_profile,
            "--host-label",
            "workstation",
            "--project-label",
            "codex-noti",
            "--routing-profile",
            "desktop",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("emit child");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(payload)
        .expect("write Codex payload");
    let output = tokio::time::timeout(
        StdDuration::from_secs(5),
        tokio::task::spawn_blocking(move || child.wait_with_output()),
    )
    .await
    .expect("emit timeout")
    .expect("emit wait task")
    .expect("emit output");
    assert!(
        output.status.success(),
        "emit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    started.elapsed()
}

fn process_rss_bytes() -> u64 {
    let system = sysinfo::System::new_all();
    system
        .process(sysinfo::Pid::from_u32(std::process::id()))
        .map_or(0, sysinfo::Process::memory)
}

fn database_footprint(path: &std::path::Path) -> u64 {
    [
        path.to_path_buf(),
        path.with_extension("sqlite3-wal"),
        path.with_extension("sqlite3-shm"),
    ]
    .iter()
    .filter_map(|candidate| fs::metadata(candidate).ok())
    .map(|metadata| metadata.len())
    .sum()
}

fn doctor_output(version: &str, interface: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-notifier"))
        .args([
            "doctor",
            "codex",
            "--codex-version",
            version,
            "--interface",
            interface,
        ])
        .output()
        .expect("doctor child");
    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("doctor UTF-8")
}

#[derive(Clone, Copy)]
struct WritableProbe;

impl StateDirectoryProbe for WritableProbe {
    fn is_writable(&self, _path: &std::path::Path) -> bool {
        true
    }
}

fn config(directory: &TempDir, role: &str, profile: &str) -> Config {
    config_with_user(directory, role, profile, None)
}

fn config_with_user(
    directory: &TempDir,
    role: &str,
    profile: &str,
    user_toml: Option<&str>,
) -> Config {
    config_with_user_and_queue_limit(directory, role, profile, user_toml, 16)
}

fn config_with_user_and_queue_limit(
    directory: &TempDir,
    role: &str,
    profile: &str,
    user_toml: Option<&str>,
    queue_limit: usize,
) -> Config {
    #[cfg(windows)]
    let paths = PathEnvironment::new()
        .with_windows_app_data(directory.path())
        .with_windows_local_app_data(directory.path())
        .resolve(Platform::Windows)
        .expect("paths");
    #[cfg(target_os = "macos")]
    let paths = PathEnvironment::new()
        .with_home(directory.path())
        .resolve(Platform::MacOs)
        .expect("paths");
    #[cfg(all(unix, not(target_os = "macos")))]
    let paths = PathEnvironment::new()
        .with_home(directory.path())
        .with_xdg_config_home(directory.path().join("config"))
        .with_xdg_state_home(directory.path())
        .resolve(Platform::Xdg)
        .expect("paths");
    let mut cli = CliOverrides::new()
        .with_role(role)
        .with_profile(profile)
        .with_state_dir(directory.path())
        .with_max_queue_entries(queue_limit);
    if role == "relay" {
        cli = cli.with_relay_host("desktop-test");
    }
    ConfigLoader::load(&paths, user_toml, None, cli, &WritableProbe).expect("configuration")
}

async fn wait_ready(runtime: &AgentRuntime) {
    tokio::time::timeout(StdDuration::from_secs(2), async {
        while runtime.state() != AgentState::Ready {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("agent readiness");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_ipc_and_sqlite_route_only_to_selected_desktop_port() {
    let directory = test_directory();
    let profile = format!(
        "d{}_{}",
        std::process::id(),
        NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
    );
    let config = config(&directory, "desktop", &profile);
    let endpoint = IpcEndpoint::new(directory.path().join("run"), &profile).expect("endpoint");
    let factory = TestFactory::new(TestDelivery::immediate());
    let host = AgentHost::from_config(&config, &factory).expect("host");
    let runtime = host.runtime();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let runner = tokio::spawn(async move {
        host.run_until(async {
            let _ = shutdown_rx.await;
        })
        .await
    });
    wait_ready(&runtime).await;
    let acknowledgement = IpcClient::new(endpoint, IpcPolicy::default())
        .submit(&event(1))
        .await
        .expect("submission");
    assert_eq!(acknowledgement.status(), AckStatus::Accepted);
    tokio::time::timeout(StdDuration::from_secs(2), async {
        while factory.delivery.delivered.load(Ordering::Acquire) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("delivery");
    shutdown_tx.send(()).expect("shutdown");
    let report = runner.await.expect("runner").expect("host run");
    assert_eq!(report.agent.delivered, 1);
    assert_eq!(factory.desktop_calls.load(Ordering::Acquire), 1);
    assert_eq!(factory.relay_calls.load(Ordering::Acquire), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_event_stdin_emit_reaches_agent_without_sensitive_values() {
    let directory = test_directory();
    let endpoint = endpoint(&directory);
    let ipc_profile = endpoint.profile().to_owned();
    let queue = Arc::new(MemoryQueue::new());
    let factory = TestFactory::new(TestDelivery::immediate());
    let host = AgentHost::bind(
        endpoint,
        IpcPolicy::default(),
        RuntimeRole::Desktop,
        AgentPolicy::new(1, StdDuration::from_secs(1)).expect("policy"),
        queue.clone(),
        &factory,
    )
    .expect("host");
    let runtime = host.runtime();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let runner = tokio::spawn(async move {
        host.run_until(async {
            let _ = shutdown_rx.await;
        })
        .await
    });
    wait_ready(&runtime).await;

    let task_elapsed = run_emit_child(
        "task-completed",
        &task_completed_payload(),
        &directory,
        &ipc_profile,
    )
    .await;
    let approval_elapsed = run_emit_child(
        "approval-requested",
        &approval_requested_payload(),
        &directory,
        &ipc_profile,
    )
    .await;
    assert!(
        task_elapsed <= HOOK_RETURN_LIMIT,
        "task hook took {task_elapsed:?}"
    );
    assert!(
        approval_elapsed <= HOOK_RETURN_LIMIT,
        "approval hook took {approval_elapsed:?}"
    );

    let submissions = queue.submissions();
    assert_eq!(submissions.len(), 2);
    for event in &submissions {
        assert_eq!(event.event_id().as_uuid().get_version_num(), 7);
        assert_eq!(event.source().host_label(), "workstation");
        assert_eq!(event.source().project_label(), Some("codex-noti"));
        assert_eq!(
            event.routing().map(codex_notifier_core::Routing::profile),
            Some("desktop")
        );
        let canonical = String::from_utf8(event.to_json().expect("canonical JSON")).expect("UTF-8");
        for sensitive in [
            "<redacted-session-id>",
            "<redacted-request-id>",
            "<redacted-path>",
            "<redacted-model>",
            "<redacted-turn-id>",
            "<redacted-message>",
            "<redacted-command>",
            "<redacted-environment-id>",
            "<redacted-amendment>",
            "acceptWithExecpolicyAmendment",
        ] {
            assert!(!canonical.contains(sensitive));
        }
    }
    assert!(
        submissions
            .iter()
            .any(|event| event.kind() == EventKind::TaskCompleted)
    );
    assert!(
        submissions
            .iter()
            .any(|event| event.kind() == EventKind::ApprovalRequested)
    );

    shutdown_tx.send(()).expect("shutdown");
    runner.await.expect("runner").expect("host run");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_emit_ipc_sqlite_and_desktop_delivery_form_one_durable_path() {
    let directory = test_directory();
    let profile = format!(
        "se{}_{}",
        std::process::id(),
        NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
    );
    let config = config_with_user_and_queue_limit(&directory, "desktop", &profile, None, 8);
    let factory = TestFactory::new(TestDelivery::immediate());
    let host = AgentHost::from_config(&config, &factory).expect("desktop host");
    let runtime = host.runtime();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let runner = tokio::spawn(async move {
        host.run_until(async {
            let _ = shutdown_rx.await;
        })
        .await
    });
    wait_ready(&runtime).await;

    let task_elapsed = run_emit_child(
        "task-completed",
        &task_completed_payload(),
        &directory,
        &profile,
    )
    .await;
    let approval_elapsed = run_emit_child(
        "approval-requested",
        &approval_requested_payload(),
        &directory,
        &profile,
    )
    .await;
    assert!(task_elapsed <= HOOK_RETURN_LIMIT);
    assert!(approval_elapsed <= HOOK_RETURN_LIMIT);
    tokio::time::timeout(StdDuration::from_secs(2), async {
        while factory.delivery.delivered.load(Ordering::Acquire) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("both native-bound deliveries");

    shutdown_tx.send(()).expect("shutdown");
    let report = runner.await.expect("runner").expect("host run");
    assert_eq!(report.agent.delivered, 2);
    let snapshot = codex_notifier_persistence::SqliteStore::inspect_read_only(&database_path(
        directory.path(),
    ))
    .expect("durable snapshot");
    assert_eq!(snapshot.queue_entries(), 0);
    assert_eq!(snapshot.receipt_entries(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hundred_event_load_stays_within_latency_memory_and_database_bounds() {
    let directory = test_directory();
    let profile = format!(
        "lb{}_{}",
        std::process::id(),
        NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
    );
    let config = config_with_user_and_queue_limit(&directory, "desktop", &profile, None, 256);
    let endpoint = IpcEndpoint::new(directory.path().join("run"), &profile).expect("endpoint");
    let factory = TestFactory::new(TestDelivery::immediate());
    let host = AgentHost::from_config(&config, &factory).expect("desktop host");
    let runtime = host.runtime();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let runner = tokio::spawn(async move {
        host.run_until(async {
            let _ = shutdown_rx.await;
        })
        .await
    });
    wait_ready(&runtime).await;

    let started = Instant::now();
    let client = IpcClient::new(endpoint, IpcPolicy::default());
    for index in 1_000..1_100 {
        let acknowledgement = client.submit(&event(index)).await.expect("load submission");
        assert_eq!(acknowledgement.status(), AckStatus::Accepted);
    }
    tokio::time::timeout(BATCH_DELIVERY_LIMIT, async {
        while factory.delivery.delivered.load(Ordering::Acquire) != 100 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("100-event delivery deadline");
    let delivery_elapsed = started.elapsed();
    let rss_bytes = process_rss_bytes();

    shutdown_tx.send(()).expect("shutdown");
    let report = runner.await.expect("runner").expect("host run");
    let database = database_path(directory.path());
    let database_bytes = database_footprint(&database);
    let snapshot = codex_notifier_persistence::SqliteStore::inspect_read_only(&database)
        .expect("durable snapshot");
    eprintln!(
        "stage18_baseline events=100 elapsed_ms={} rss_bytes={rss_bytes} database_bytes={database_bytes}",
        delivery_elapsed.as_millis()
    );
    assert!(delivery_elapsed <= BATCH_DELIVERY_LIMIT);
    assert!(rss_bytes > 0);
    assert!(rss_bytes <= PROCESS_RSS_LIMIT_BYTES);
    assert!(database_bytes > 0);
    assert!(database_bytes <= DATABASE_SIZE_LIMIT_BYTES);
    assert_eq!(report.agent.delivered, 100);
    assert!(report.agent.peak_active_deliveries <= 4);
    assert_eq!(snapshot.queue_entries(), 0);
    assert_eq!(snapshot.receipt_entries(), 100);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hundred_duplicate_retries_produce_one_desktop_delivery() {
    let directory = test_directory();
    let profile = format!(
        "dd{}_{}",
        std::process::id(),
        NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
    );
    let config = config_with_user_and_queue_limit(&directory, "desktop", &profile, None, 8);
    let endpoint = IpcEndpoint::new(directory.path().join("run"), &profile).expect("endpoint");
    let factory = TestFactory::new(TestDelivery::immediate());
    let host = AgentHost::from_config(&config, &factory).expect("desktop host");
    let runtime = host.runtime();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let runner = tokio::spawn(async move {
        host.run_until(async {
            let _ = shutdown_rx.await;
        })
        .await
    });
    wait_ready(&runtime).await;

    let fixture = event(2_000);
    let client = IpcClient::new(endpoint, IpcPolicy::default());
    assert_eq!(
        client
            .submit(&fixture)
            .await
            .expect("initial submission")
            .status(),
        AckStatus::Accepted
    );
    tokio::time::timeout(StdDuration::from_secs(2), async {
        while factory.delivery.delivered.load(Ordering::Acquire) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("initial delivery");
    for _ in 0..100 {
        assert_eq!(
            client
                .submit(&fixture)
                .await
                .expect("duplicate retry")
                .status(),
            AckStatus::Duplicate
        );
    }
    tokio::time::sleep(StdDuration::from_millis(50)).await;
    assert_eq!(factory.delivery.delivered.load(Ordering::Acquire), 1);

    shutdown_tx.send(()).expect("shutdown");
    let report = runner.await.expect("runner").expect("host run");
    assert_eq!(report.agent.delivered, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_queue_rejects_new_ipc_work_without_losing_the_leased_event() {
    let directory = test_directory();
    let profile = format!(
        "qf{}_{}",
        std::process::id(),
        NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
    );
    let config = config_with_user_and_queue_limit(&directory, "desktop", &profile, None, 1);
    let endpoint = IpcEndpoint::new(directory.path().join("run"), &profile).expect("endpoint");
    let factory = TestFactory::new(TestDelivery::cancel_aware());
    let host = AgentHost::from_config(&config, &factory).expect("desktop host");
    let runtime = host.runtime();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let runner = tokio::spawn(async move {
        host.run_until(async {
            let _ = shutdown_rx.await;
        })
        .await
    });
    wait_ready(&runtime).await;
    let client = IpcClient::new(endpoint, IpcPolicy::default());
    assert_eq!(
        client
            .submit(&event(3_000))
            .await
            .expect("first event")
            .status(),
        AckStatus::Accepted
    );
    tokio::time::timeout(StdDuration::from_secs(2), async {
        while factory.delivery.active.load(Ordering::Acquire) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("leased event");
    let rejected = client.submit(&event(3_001)).await.expect("queue rejection");
    assert_eq!(rejected.status(), AckStatus::Rejected);
    let error = rejected.error().expect("safe queue-full error");
    assert_eq!(error.code(), "agent_queue_full");
    assert!(error.retryable());

    shutdown_tx.send(()).expect("shutdown");
    let report = runner.await.expect("runner").expect("host run");
    assert_eq!(report.agent.released, 1);
    let snapshot = codex_notifier_persistence::SqliteStore::inspect_read_only(&database_path(
        directory.path(),
    ))
    .expect("durable snapshot");
    assert_eq!(snapshot.queue_entries(), 1);
    assert_eq!(snapshot.receipt_entries(), 0);
}

#[tokio::test]
async fn emit_source_and_ipc_failures_remain_distinct() {
    let directory = test_directory();
    let endpoint = endpoint(&directory);
    let context = TaskCompletedContext::new("workstation", None, None).expect("context");
    assert_eq!(
        TaskCompletedEmitter::new(
            "0.144.6",
            endpoint.clone(),
            context.clone(),
            IpcPolicy::default(),
        )
        .expect_err("unsupported version"),
        EmitError::Source(SourceError::UnsupportedVersion)
    );

    let short_policy = IpcPolicy::new(
        StdDuration::from_millis(20),
        StdDuration::from_millis(20),
        1,
    )
    .expect("short IPC policy");
    let emitter = TaskCompletedEmitter::new("0.144.5", endpoint.clone(), context, short_policy)
        .expect("emitter");
    assert_eq!(
        emitter.emit(b"{}").await,
        Err(EmitError::Source(SourceError::IncompatiblePayload))
    );
    assert!(matches!(
        emitter.emit(&task_completed_payload()).await,
        Err(EmitError::Ipc(
            IpcError::ConnectionFailed | IpcError::Timeout
        ))
    ));

    let approval_context =
        ApprovalRequestedContext::new("workstation", None, None).expect("approval context");
    assert_eq!(
        ApprovalRequestedEmitter::new(
            "0.144.6",
            endpoint.clone(),
            approval_context.clone(),
            short_policy,
        )
        .expect_err("unsupported approval version"),
        EmitError::Source(SourceError::UnsupportedVersion)
    );
    let approval =
        ApprovalRequestedEmitter::new("0.144.5", endpoint, approval_context, short_policy)
            .expect("approval emitter");
    assert_eq!(
        approval.emit(b"{}").await,
        Err(EmitError::Source(SourceError::IncompatiblePayload))
    );
    assert!(matches!(
        approval.emit(&approval_requested_payload()).await,
        Err(EmitError::Ipc(
            IpcError::ConnectionFailed | IpcError::Timeout
        ))
    ));
}

#[test]
fn doctor_codex_matches_capability_and_installation_selection() {
    let app_server = doctor_output("0.144.5", "app-server");
    assert!(app_server.contains("codex_version=0.144.5"));
    assert!(app_server.contains("interface=app_server"));
    assert!(app_server.contains("task_completed=unsupported_interface"));
    assert!(app_server.contains("approval_requested=supported"));
    assert!(app_server.contains("approval_installation=configure_app_server"));
    assert!(app_server.contains("display-only"));

    let cli_hook = doctor_output("0.144.5", "cli-hook");
    assert!(cli_hook.contains("task_completed=supported"));
    assert!(cli_hook.contains("approval_requested=unverified"));
    assert!(cli_hook.contains("approval_installation=report_unavailable"));
    assert!(cli_hook.contains("no approval hook will be installed"));

    let unknown = doctor_output("sensitive-unknown-version", "app-server");
    assert!(unknown.contains("codex_version=unsupported"));
    assert!(unknown.contains("approval_requested=unsupported_version"));
    assert!(unknown.contains("approval_installation=report_unavailable"));
    assert!(!unknown.contains("sensitive-unknown-version"));
}

#[tokio::test]
async fn second_profile_instance_fails_before_another_adapter_initializes() {
    let directory = test_directory();
    let endpoint = endpoint(&directory);
    let queue: Arc<dyn AgentQueue> = Arc::new(MemoryQueue::new());
    let factory = TestFactory::new(TestDelivery::immediate());
    let _first = AgentHost::bind(
        endpoint.clone(),
        IpcPolicy::default(),
        RuntimeRole::Desktop,
        AgentPolicy::default(),
        Arc::clone(&queue),
        &factory,
    )
    .expect("first host");
    assert!(matches!(
        AgentHost::bind(
            endpoint,
            IpcPolicy::default(),
            RuntimeRole::Desktop,
            AgentPolicy::default(),
            queue,
            &factory,
        ),
        Err(HostError::Ipc(IpcError::AlreadyRunning))
    ));
    assert_eq!(factory.desktop_calls.load(Ordering::Acquire), 1);
    assert_eq!(factory.relay_calls.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn validated_relay_config_initializes_only_relay_port() {
    let directory = test_directory();
    let profile = format!(
        "r{}_{}",
        std::process::id(),
        NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
    );
    let config = config(&directory, "relay", &profile);
    let factory = TestFactory::new(TestDelivery::immediate());
    let _host = AgentHost::from_config(&config, &factory).expect("relay host");
    assert_eq!(factory.desktop_calls.load(Ordering::Acquire), 0);
    assert_eq!(factory.relay_calls.load(Ordering::Acquire), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn durable_relay_retry_wakes_at_backoff_and_then_acknowledges() {
    let directory = test_directory();
    let profile = format!(
        "rr{}_{}",
        std::process::id(),
        NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
    );
    let config = config_with_user(
        &directory,
        "relay",
        &profile,
        Some(
            "config_version = 1\n[relay]\nretry_initial_delay_ms = 100\nretry_max_delay_ms = 100\nretry_max_attempts = 3\n",
        ),
    );
    let factory = TestFactory::new(TestDelivery::retry_once());
    let host = AgentHost::from_config(&config, &factory).expect("relay host");
    let runtime = host.runtime();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let runner = tokio::spawn(async move {
        host.run_until(async {
            let _ = shutdown_rx.await;
        })
        .await
    });
    wait_ready(&runtime).await;
    let started = std::time::Instant::now();
    assert_eq!(
        runtime.submit(&event(90)),
        Ok(codex_notifier_application::SubmissionOutcome::Accepted)
    );
    tokio::time::timeout(StdDuration::from_secs(2), async {
        while factory.delivery.delivered.load(Ordering::Acquire) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("retry recovery");
    assert!(started.elapsed() >= StdDuration::from_millis(70));
    tokio::time::sleep(StdDuration::from_millis(20)).await;
    shutdown_tx.send(()).expect("shutdown");
    let report = runner.await.expect("runner").expect("host run");
    assert_eq!(report.agent.retried, 1);
    assert_eq!(report.agent.delivered, 1);
    assert_eq!(report.agent.dead_lettered, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exhausted_relay_attempts_become_one_bounded_dead_letter() {
    let directory = test_directory();
    let profile = format!(
        "rd{}_{}",
        std::process::id(),
        NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
    );
    let config = config_with_user(
        &directory,
        "relay",
        &profile,
        Some(
            "config_version = 1\n[relay]\nretry_initial_delay_ms = 100\nretry_max_delay_ms = 100\nretry_max_attempts = 2\n",
        ),
    );
    let factory = TestFactory::new(TestDelivery::always_retry());
    let host = AgentHost::from_config(&config, &factory).expect("relay host");
    let runtime = host.runtime();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let runner = tokio::spawn(async move {
        host.run_until(async {
            let _ = shutdown_rx.await;
        })
        .await
    });
    wait_ready(&runtime).await;
    runtime.submit(&event(91)).expect("relay submission");
    tokio::time::timeout(StdDuration::from_secs(2), async {
        while factory.delivery.attempts.load(Ordering::Acquire) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("attempt exhaustion");
    tokio::time::sleep(StdDuration::from_millis(20)).await;
    shutdown_tx.send(()).expect("shutdown");
    let report = runner.await.expect("runner").expect("host run");
    assert_eq!(factory.delivery.attempts.load(Ordering::Acquire), 2);
    assert_eq!(report.agent.retried, 1);
    assert_eq!(report.agent.delivered, 0);
    assert_eq!(report.agent.dead_lettered, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn future_relay_retry_survives_agent_restart_without_new_submission() {
    let directory = test_directory();
    let profile = format!(
        "rs{}_{}",
        std::process::id(),
        NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
    );
    let config = config_with_user(
        &directory,
        "relay",
        &profile,
        Some(
            "config_version = 1\n[relay]\nretry_initial_delay_ms = 1000\nretry_max_delay_ms = 1000\nretry_max_attempts = 3\n",
        ),
    );

    let failing_factory = TestFactory::new(TestDelivery::always_retry());
    let first_host = AgentHost::from_config(&config, &failing_factory).expect("first relay host");
    let first_runtime = first_host.runtime();
    let (first_shutdown_tx, first_shutdown_rx) = oneshot::channel();
    let first_runner = tokio::spawn(async move {
        first_host
            .run_until(async {
                let _ = first_shutdown_rx.await;
            })
            .await
    });
    wait_ready(&first_runtime).await;
    first_runtime.submit(&event(92)).expect("relay submission");
    tokio::time::timeout(StdDuration::from_secs(2), async {
        while failing_factory.delivery.attempts.load(Ordering::Acquire) < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("first failed attempt");
    tokio::time::sleep(StdDuration::from_millis(20)).await;
    first_shutdown_tx.send(()).expect("first shutdown");
    let first_report = first_runner
        .await
        .expect("first runner")
        .expect("first host run");
    assert_eq!(first_report.agent.retried, 1);

    let recovered_factory = TestFactory::new(TestDelivery::immediate());
    let second_host =
        AgentHost::from_config(&config, &recovered_factory).expect("recovered relay host");
    let second_runtime = second_host.runtime();
    let (second_shutdown_tx, second_shutdown_rx) = oneshot::channel();
    let second_runner = tokio::spawn(async move {
        second_host
            .run_until(async {
                let _ = second_shutdown_rx.await;
            })
            .await
    });
    wait_ready(&second_runtime).await;
    tokio::time::timeout(StdDuration::from_secs(2), async {
        while recovered_factory.delivery.delivered.load(Ordering::Acquire) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("restart recovery");
    tokio::time::sleep(StdDuration::from_millis(20)).await;
    second_shutdown_tx.send(()).expect("second shutdown");
    let second_report = second_runner
        .await
        .expect("second runner")
        .expect("second host run");
    assert_eq!(second_report.agent.delivered, 1);
    assert_eq!(second_report.agent.retried, 0);
    assert_eq!(second_report.agent.dead_lettered, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_shutdown_releases_inflight_work_and_removes_endpoint() {
    let directory = test_directory();
    let endpoint = endpoint(&directory);
    let queue = Arc::new(MemoryQueue::new());
    let factory = TestFactory::new(TestDelivery::cancel_aware());
    let host = AgentHost::bind(
        endpoint.clone(),
        IpcPolicy::default(),
        RuntimeRole::Relay,
        AgentPolicy::new(1, StdDuration::from_secs(1)).expect("policy"),
        queue.clone(),
        &factory,
    )
    .expect("host");
    let runtime = host.runtime();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let runner = tokio::spawn(async move {
        host.run_until(async {
            let _ = shutdown_rx.await;
        })
        .await
    });
    wait_ready(&runtime).await;
    IpcClient::new(endpoint.clone(), IpcPolicy::default())
        .submit(&event(2))
        .await
        .expect("submission");
    tokio::time::timeout(StdDuration::from_secs(2), async {
        while factory.delivery.active.load(Ordering::Acquire) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("active delivery");
    shutdown_tx.send(()).expect("shutdown");
    let report = runner.await.expect("runner").expect("host run");
    assert_eq!(report.agent.released, 1);
    assert_eq!(queue.counts(), (1, 0));
    assert!(matches!(
        IpcClient::new(endpoint, IpcPolicy::default())
            .submit(&event(3))
            .await,
        Err(IpcError::ConnectionFailed | IpcError::Timeout)
    ));
}
