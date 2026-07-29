//! Real IPC/SQLite composition, single-instance, and shutdown contract tests.

use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use codex_notifier::{AgentHost, EmitError, HostError, TaskCompletedEmitter};
use codex_notifier_application::{
    AgentError, AgentLease, AgentPolicy, AgentQueue, AgentQueueError, AgentRuntime, AgentState,
    CancellationToken, DeliveryFuture, DeliveryOutcome, EnqueueResult, EventDelivery,
    RoleDeliveryFactory, RuntimeRole, SafeErrorCode,
};
use codex_notifier_codex_source::{SourceError, TaskCompletedContext};
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
    active: AtomicUsize,
    delivered: AtomicUsize,
}

impl TestDelivery {
    fn immediate() -> Self {
        Self {
            cancel_aware: false,
            active: AtomicUsize::new(0),
            delivered: AtomicUsize::new(0),
        }
    }

    fn cancel_aware() -> Self {
        Self {
            cancel_aware: true,
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
            self.active.fetch_add(1, Ordering::AcqRel);
            if self.cancel_aware {
                cancellation.cancelled().await;
                self.active.fetch_sub(1, Ordering::AcqRel);
                DeliveryOutcome::Cancelled
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
        AgentLease::new(event, token).map(Some)
    }

    fn acknowledge(&self, lease: &AgentLease) -> Result<(), AgentQueueError> {
        self.leased
            .lock()
            .map_err(|_| AgentQueueError::Unavailable)?
            .remove(lease.token())
            .ok_or(AgentQueueError::Corrupt)?;
        Ok(())
    }

    fn retry(&self, lease: &AgentLease, code: &SafeErrorCode) -> Result<(), AgentQueueError> {
        self.release(lease, code)
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

#[derive(Clone, Copy)]
struct WritableProbe;

impl StateDirectoryProbe for WritableProbe {
    fn is_writable(&self, _path: &std::path::Path) -> bool {
        true
    }
}

fn config(directory: &TempDir, role: &str, profile: &str) -> Config {
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
        .with_max_queue_entries(16);
    if role == "relay" {
        cli = cli.with_relay_host("desktop-test");
    }
    ConfigLoader::load(&paths, None, None, cli, &WritableProbe).expect("configuration")
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
async fn codex_hook_stdin_emit_reaches_agent_without_sensitive_values() {
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

    let payload = task_completed_payload();
    let mut child = Command::new(env!("CARGO_BIN_EXE_codex-notifier"))
        .args([
            "emit",
            "task-completed",
            "--codex-version",
            "0.144.5",
            "--state-dir",
        ])
        .arg(directory.path())
        .args([
            "--ipc-profile",
            &ipc_profile,
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
        .write_all(&payload)
        .expect("write hook payload");
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

    let submissions = queue.submissions();
    assert_eq!(submissions.len(), 1);
    let event = &submissions[0];
    assert_eq!(event.kind(), EventKind::TaskCompleted);
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
        "<redacted-path>",
        "<redacted-model>",
        "<redacted-turn-id>",
        "<redacted-message>",
    ] {
        assert!(!canonical.contains(sensitive));
    }

    shutdown_tx.send(()).expect("shutdown");
    runner.await.expect("runner").expect("host run");
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
    let emitter =
        TaskCompletedEmitter::new("0.144.5", endpoint, context, short_policy).expect("emitter");
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
