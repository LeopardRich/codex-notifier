//! Agent role, lifecycle, backpressure, cancellation, and lease-safety tests.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use codex_notifier_application::{
    AgentError, AgentLease, AgentPolicy, AgentQueue, AgentQueueError, AgentRuntime, AgentState,
    CancellationToken, DeliveryFuture, DeliveryOutcome, EnqueueResult, EventDelivery,
    RoleDeliveryFactory, RuntimeRole, SafeErrorCode, SubmissionOutcome,
};
use codex_notifier_core::{
    CanonicalEvent, EventId, EventKind, EventSource, Extensions, Presentation, Privacy, Urgency,
};
use time::{Duration, OffsetDateTime};
use tokio::sync::oneshot;

const UUID_PREFIX: &str = "01890f4d-e000-7000-8000-";

fn event(index: usize) -> CanonicalEvent {
    let now = OffsetDateTime::now_utc();
    let id = format!("{UUID_PREFIX}{index:012x}");
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

#[derive(Clone, Copy)]
enum DeliveryMode {
    Immediate,
    CancelAware,
    NeverCompletes,
}

struct FakeDelivery {
    mode: DeliveryMode,
    active: AtomicUsize,
    peak: AtomicUsize,
}

impl FakeDelivery {
    fn new(mode: DeliveryMode) -> Self {
        Self {
            mode,
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
}

impl EventDelivery for FakeDelivery {
    fn deliver<'a>(
        &'a self,
        _event: &'a CanonicalEvent,
        cancellation: CancellationToken,
    ) -> DeliveryFuture<'a> {
        Box::pin(async move {
            let active = self.active.fetch_add(1, Ordering::AcqRel) + 1;
            self.peak.fetch_max(active, Ordering::AcqRel);
            let _guard = DeliveryActiveGuard(&self.active);
            match self.mode {
                DeliveryMode::Immediate => DeliveryOutcome::Delivered,
                DeliveryMode::CancelAware => {
                    cancellation.cancelled().await;
                    DeliveryOutcome::Cancelled
                }
                DeliveryMode::NeverCompletes => future::pending().await,
            }
        })
    }
}

struct DeliveryActiveGuard<'a>(&'a AtomicUsize);

impl Drop for DeliveryActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct FakeFactory {
    delivery: Arc<FakeDelivery>,
    desktop_calls: AtomicUsize,
    relay_calls: AtomicUsize,
}

impl FakeFactory {
    fn new(mode: DeliveryMode) -> Self {
        Self {
            delivery: Arc::new(FakeDelivery::new(mode)),
            desktop_calls: AtomicUsize::new(0),
            relay_calls: AtomicUsize::new(0),
        }
    }
}

impl RoleDeliveryFactory for FakeFactory {
    fn desktop(&self) -> Result<Arc<dyn EventDelivery>, AgentError> {
        self.desktop_calls.fetch_add(1, Ordering::AcqRel);
        Ok(self.delivery.clone())
    }

    fn relay(&self) -> Result<Arc<dyn EventDelivery>, AgentError> {
        self.relay_calls.fetch_add(1, Ordering::AcqRel);
        Ok(self.delivery.clone())
    }
}

struct FakeQueue {
    capacity: usize,
    state: Mutex<QueueState>,
    next_lease: AtomicUsize,
}

#[derive(Default)]
struct QueueState {
    known: BTreeSet<EventId>,
    queued: VecDeque<CanonicalEvent>,
    leased: BTreeMap<String, CanonicalEvent>,
    delivered: usize,
    dead_lettered: usize,
    release_codes: Vec<String>,
}

impl FakeQueue {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            state: Mutex::new(QueueState::default()),
            next_lease: AtomicUsize::new(0),
        }
    }

    fn counts(&self) -> (usize, usize, usize, usize) {
        let state = self.state.lock().expect("queue state");
        (
            state.queued.len(),
            state.leased.len(),
            state.delivered,
            state.dead_lettered,
        )
    }
}

impl AgentQueue for FakeQueue {
    fn enqueue(&self, event: &CanonicalEvent) -> Result<EnqueueResult, AgentQueueError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AgentQueueError::Unavailable)?;
        if state.known.contains(&event.event_id()) {
            return Ok(EnqueueResult::Duplicate);
        }
        if state.queued.len() + state.leased.len() >= self.capacity {
            return Err(AgentQueueError::Full);
        }
        state.known.insert(event.event_id());
        state.queued.push_back(event.clone());
        Ok(EnqueueResult::Enqueued)
    }

    fn lease(&self, worker: usize) -> Result<Option<AgentLease>, AgentQueueError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AgentQueueError::Unavailable)?;
        let Some(event) = state.queued.pop_front() else {
            return Ok(None);
        };
        let sequence = self.next_lease.fetch_add(1, Ordering::Relaxed);
        let token = format!("w{worker}_{sequence}");
        state.leased.insert(token.clone(), event.clone());
        AgentLease::new(event, token).map(Some)
    }

    fn acknowledge(&self, lease: &AgentLease) -> Result<(), AgentQueueError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AgentQueueError::Unavailable)?;
        state
            .leased
            .remove(lease.token())
            .ok_or(AgentQueueError::Corrupt)?;
        state.delivered += 1;
        Ok(())
    }

    fn retry(&self, lease: &AgentLease, code: &SafeErrorCode) -> Result<(), AgentQueueError> {
        self.release(lease, code)
    }

    fn release(&self, lease: &AgentLease, code: &SafeErrorCode) -> Result<(), AgentQueueError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AgentQueueError::Unavailable)?;
        let event = state
            .leased
            .remove(lease.token())
            .ok_or(AgentQueueError::Corrupt)?;
        state.queued.push_back(event);
        state.release_codes.push(code.as_str().to_owned());
        Ok(())
    }

    fn dead_letter(
        &self,
        lease: &AgentLease,
        _code: &SafeErrorCode,
    ) -> Result<(), AgentQueueError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AgentQueueError::Unavailable)?;
        state
            .leased
            .remove(lease.token())
            .ok_or(AgentQueueError::Corrupt)?;
        state.dead_lettered += 1;
        Ok(())
    }
}

async fn wait_for_state(runtime: &AgentRuntime, expected: AgentState) {
    tokio::time::timeout(StdDuration::from_secs(2), async {
        while runtime.state() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("state transition");
}

async fn wait_for_active(delivery: &FakeDelivery, expected: usize) {
    tokio::time::timeout(StdDuration::from_secs(2), async {
        while delivery.active() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("active delivery count");
}

#[test]
fn explicit_role_initializes_only_its_adapter_graph() {
    let desktop_factory = FakeFactory::new(DeliveryMode::Immediate);
    AgentRuntime::compose(
        RuntimeRole::Desktop,
        AgentPolicy::default(),
        Arc::new(FakeQueue::new(1)),
        &desktop_factory,
    )
    .expect("desktop composition");
    assert_eq!(desktop_factory.desktop_calls.load(Ordering::Acquire), 1);
    assert_eq!(desktop_factory.relay_calls.load(Ordering::Acquire), 0);

    let relay_factory = FakeFactory::new(DeliveryMode::Immediate);
    AgentRuntime::compose(
        RuntimeRole::Relay,
        AgentPolicy::default(),
        Arc::new(FakeQueue::new(1)),
        &relay_factory,
    )
    .expect("relay composition");
    assert_eq!(relay_factory.desktop_calls.load(Ordering::Acquire), 0);
    assert_eq!(relay_factory.relay_calls.load(Ordering::Acquire), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ready_submission_delivers_and_stops_cleanly() {
    let queue = Arc::new(FakeQueue::new(8));
    let factory = FakeFactory::new(DeliveryMode::Immediate);
    let runtime = AgentRuntime::compose(
        RuntimeRole::Desktop,
        AgentPolicy::new(2, StdDuration::from_secs(1)).expect("policy"),
        queue.clone(),
        &factory,
    )
    .expect("runtime");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let runner = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .run_until(async {
                    let _ = shutdown_rx.await;
                })
                .await
        })
    };
    wait_for_state(&runtime, AgentState::Ready).await;
    assert_eq!(runtime.submit(&event(1)), Ok(SubmissionOutcome::Accepted));
    tokio::time::timeout(StdDuration::from_secs(2), async {
        while queue.counts().2 != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("delivery");
    shutdown_tx.send(()).expect("shutdown");
    let report = runner.await.expect("runner").expect("clean run");
    assert_eq!(runtime.state(), AgentState::Stopped);
    assert_eq!(report.workers_spawned, 2);
    assert_eq!(report.delivered, 1);
    assert_eq!(report.forced_cancellations, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn capacity_backpressure_and_worker_peak_are_hard_bounded() {
    let queue = Arc::new(FakeQueue::new(12));
    let factory = FakeFactory::new(DeliveryMode::CancelAware);
    let runtime = AgentRuntime::compose(
        RuntimeRole::Relay,
        AgentPolicy::new(3, StdDuration::from_secs(1)).expect("policy"),
        queue.clone(),
        &factory,
    )
    .expect("runtime");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let runner = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .run_until(async {
                    let _ = shutdown_rx.await;
                })
                .await
        })
    };
    wait_for_state(&runtime, AgentState::Ready).await;
    let mut accepted = 0;
    let mut full = 0;
    for index in 1..=100 {
        match runtime.submit(&event(index)) {
            Ok(SubmissionOutcome::Accepted) => accepted += 1,
            Err(AgentError::Queue(AgentQueueError::Full)) => full += 1,
            other => panic!("unexpected submission: {other:?}"),
        }
    }
    assert_eq!(accepted, 12);
    assert_eq!(full, 88);
    wait_for_active(&factory.delivery, 3).await;
    assert_eq!(queue.counts().0 + queue.counts().1, 12);
    shutdown_tx.send(()).expect("shutdown");
    let report = runner.await.expect("runner").expect("clean run");
    assert_eq!(report.workers_spawned, 3);
    assert_eq!(report.peak_active_deliveries, 3);
    assert_eq!(queue.counts().1, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cooperative_shutdown_rejects_new_work_and_releases_inflight_lease() {
    let queue = Arc::new(FakeQueue::new(4));
    let factory = FakeFactory::new(DeliveryMode::CancelAware);
    let runtime = AgentRuntime::compose(
        RuntimeRole::Desktop,
        AgentPolicy::new(1, StdDuration::from_secs(1)).expect("policy"),
        queue.clone(),
        &factory,
    )
    .expect("runtime");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let runner = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .run_until(async {
                    let _ = shutdown_rx.await;
                })
                .await
        })
    };
    wait_for_state(&runtime, AgentState::Ready).await;
    runtime.submit(&event(1)).expect("submission");
    wait_for_active(&factory.delivery, 1).await;
    shutdown_tx.send(()).expect("shutdown");
    wait_for_state(&runtime, AgentState::Draining).await;
    assert_eq!(runtime.submit(&event(2)), Err(AgentError::Draining));
    let report = runner.await.expect("runner").expect("clean run");
    assert_eq!(report.released, 1);
    assert_eq!(queue.counts(), (1, 0, 0, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forced_cancellation_drop_guard_returns_lease_to_queue() {
    let queue = Arc::new(FakeQueue::new(2));
    let factory = FakeFactory::new(DeliveryMode::NeverCompletes);
    let runtime = AgentRuntime::compose(
        RuntimeRole::Relay,
        AgentPolicy::new(1, StdDuration::from_millis(20)).expect("policy"),
        queue.clone(),
        &factory,
    )
    .expect("runtime");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let runner = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .run_until(async {
                    let _ = shutdown_rx.await;
                })
                .await
        })
    };
    wait_for_state(&runtime, AgentState::Ready).await;
    runtime.submit(&event(1)).expect("submission");
    wait_for_active(&factory.delivery, 1).await;
    shutdown_tx.send(()).expect("shutdown");
    let report = runner.await.expect("runner").expect("clean run");
    assert_eq!(report.forced_cancellations, 1);
    assert_eq!(queue.counts(), (1, 0, 0, 0));
}
