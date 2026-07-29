//! Bounded role-aware agent lifecycle and adapter ports.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::time::Duration;

use codex_notifier_core::CanonicalEvent;
use thiserror::Error;
use tokio::sync::{Notify, watch};
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::SafeErrorCode;

const MAX_WORKERS: usize = 64;
const MIN_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(10);
const MAX_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_QUEUE_WAKE_DELAY: Duration = Duration::from_millis(1);
const MAX_QUEUE_WAKE_DELAY: Duration = Duration::from_secs(60 * 60);

/// Explicit runtime role selected by validated configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeRole {
    /// Deliver events through the desktop notification port.
    Desktop,
    /// Forward events through the relay transport port.
    Relay,
}

/// Observable agent lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentState {
    /// Ports are composed but workers are not accepting events.
    Starting,
    /// IPC submissions and worker delivery are active.
    Ready,
    /// New submissions are rejected while leased work is resolved.
    Draining,
    /// All worker tasks have exited.
    Stopped,
}

impl AgentState {
    const fn as_u8(self) -> u8 {
        match self {
            Self::Starting => 0,
            Self::Ready => 1,
            Self::Draining => 2,
            Self::Stopped => 3,
        }
    }

    const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Starting,
            1 => Self::Ready,
            2 => Self::Draining,
            _ => Self::Stopped,
        }
    }
}

/// Fixed upper bounds for worker tasks and graceful shutdown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentPolicy {
    workers: usize,
    shutdown_timeout: Duration,
}

impl AgentPolicy {
    /// Creates a hard-bounded runtime policy.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::InvalidPolicy`] outside 1-64 workers or 10 ms to
    /// 30 seconds of graceful shutdown time.
    pub fn new(workers: usize, shutdown_timeout: Duration) -> Result<Self, AgentError> {
        if workers == 0
            || workers > MAX_WORKERS
            || shutdown_timeout < MIN_SHUTDOWN_TIMEOUT
            || shutdown_timeout > MAX_SHUTDOWN_TIMEOUT
        {
            return Err(AgentError::InvalidPolicy);
        }
        Ok(Self {
            workers,
            shutdown_timeout,
        })
    }

    /// Returns the exact number of long-lived worker tasks.
    #[must_use]
    pub const fn workers(self) -> usize {
        self.workers
    }

    /// Returns the graceful drain deadline.
    #[must_use]
    pub const fn shutdown_timeout(self) -> Duration {
        self.shutdown_timeout
    }
}

impl Default for AgentPolicy {
    fn default() -> Self {
        Self {
            workers: 4,
            shutdown_timeout: Duration::from_secs(5),
        }
    }
}

/// Cooperative cancellation signal passed into role-specific delivery.
#[derive(Clone, Debug)]
pub struct CancellationToken {
    sender: watch::Sender<bool>,
}

impl CancellationToken {
    fn new() -> Self {
        let (sender, _) = watch::channel(false);
        Self { sender }
    }

    /// Returns whether shutdown has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.sender.borrow()
    }

    /// Waits until shutdown is requested.
    pub async fn cancelled(&self) {
        let mut receiver = self.sender.subscribe();
        if *receiver.borrow() {
            return;
        }
        let _ = receiver.changed().await;
    }

    fn cancel(&self) {
        self.sender.send_replace(true);
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of transactionally accepting an event into the durable queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnqueueResult {
    /// A new event was committed.
    Enqueued,
    /// The event identifier was already known.
    Duplicate,
}

/// User-facing result of one local IPC submission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmissionOutcome {
    /// A new event is durable and scheduled for a worker.
    Accepted,
    /// A durable row or receipt already exists for the event.
    Duplicate,
}

/// A canonical event protected by an opaque compare-and-set lease token.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentLease {
    event: CanonicalEvent,
    token: String,
    attempt: u32,
}

impl AgentLease {
    /// Creates a lease returned by an [`AgentQueue`] implementation.
    ///
    /// # Errors
    ///
    /// Returns [`AgentQueueError::Corrupt`] for an unsafe token.
    pub fn new(
        event: CanonicalEvent,
        token: impl Into<String>,
        attempt: u32,
    ) -> Result<Self, AgentQueueError> {
        let token = token.into();
        if attempt == 0
            || token.is_empty()
            || token.len() > 64
            || !token.is_ascii()
            || !token.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || (index > 0 && matches!(byte, b'_' | b'-'))
            })
        {
            return Err(AgentQueueError::Corrupt);
        }
        Ok(Self {
            event,
            token,
            attempt,
        })
    }

    /// Returns the leased canonical event.
    #[must_use]
    pub const fn event(&self) -> &CanonicalEvent {
        &self.event
    }

    /// Returns the opaque lease token.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Returns the one-based delivery attempt number retained by the queue.
    #[must_use]
    pub const fn attempt(&self) -> u32 {
        self.attempt
    }
}

/// Outcome of consuming one retryable delivery attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryResult {
    /// The event remains durable and has a future availability time.
    Scheduled,
    /// The retry bound was reached and only safe dead-letter metadata remains.
    DeadLettered,
}

/// Stable queue failure classifications used for backpressure and recovery.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum AgentQueueError {
    /// The configured durable queue capacity is reached.
    #[error("agent queue is full")]
    Full,
    /// The event is outside the accepted age window.
    #[error("agent event is expired")]
    Expired,
    /// Another process or transaction currently owns the database lock.
    #[error("agent queue is locked")]
    Locked,
    /// Stored state or a lease transition is inconsistent.
    #[error("agent queue state is invalid")]
    Corrupt,
    /// Another durable queue operation failed safely.
    #[error("agent queue is unavailable")]
    Unavailable,
}

impl AgentQueueError {
    /// Returns the stable safe acknowledgement code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Full => "agent_queue_full",
            Self::Expired => "agent_event_expired",
            Self::Locked => "agent_queue_locked",
            Self::Corrupt => "agent_queue_corrupt",
            Self::Unavailable => "agent_queue_unavailable",
        }
    }

    /// Returns whether a later submission may succeed.
    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(self, Self::Full | Self::Locked | Self::Unavailable)
    }
}

/// Durable queue operations required by the lifecycle runtime.
pub trait AgentQueue: Send + Sync {
    /// Commits a canonical event before acknowledging local submission.
    ///
    /// # Errors
    ///
    /// Returns a classified capacity, expiry, lock, corruption, or availability
    /// failure.
    fn enqueue(&self, event: &CanonicalEvent) -> Result<EnqueueResult, AgentQueueError>;
    /// Atomically leases the next available event for one worker.
    ///
    /// # Errors
    ///
    /// Returns a classified lock, corruption, or availability failure.
    fn lease(&self, worker: usize) -> Result<Option<AgentLease>, AgentQueueError>;
    /// Returns the delay until durable work may next become leaseable.
    ///
    /// `None` means no queued or recoverable leased work is currently known.
    /// Implementations may return an approximate delay because a concurrent
    /// enqueue also wakes the runtime explicitly.
    ///
    /// # Errors
    ///
    /// Returns a classified lock, corruption, or availability failure.
    fn next_wake(&self) -> Result<Option<Duration>, AgentQueueError> {
        Ok(None)
    }
    /// Commits successful delivery and removes the leased payload.
    ///
    /// # Errors
    ///
    /// Returns a classified stale-lease, corruption, lock, or availability
    /// failure.
    fn acknowledge(&self, lease: &AgentLease) -> Result<(), AgentQueueError>;
    /// Schedules a failed delivery while consuming its current attempt.
    ///
    /// # Errors
    ///
    /// Returns a classified stale-lease, corruption, lock, or availability
    /// failure.
    fn retry(
        &self,
        lease: &AgentLease,
        code: &SafeErrorCode,
    ) -> Result<RetryResult, AgentQueueError>;
    /// Safely returns retryable or cancelled work to durable availability.
    ///
    /// # Errors
    ///
    /// Returns a classified stale-lease, corruption, lock, or availability
    /// failure.
    fn release(&self, lease: &AgentLease, code: &SafeErrorCode) -> Result<(), AgentQueueError>;
    /// Removes a permanent failure while retaining metadata only.
    ///
    /// # Errors
    ///
    /// Returns a classified stale-lease, corruption, lock, or availability
    /// failure.
    fn dead_letter(&self, lease: &AgentLease, code: &SafeErrorCode) -> Result<(), AgentQueueError>;
}

/// Validated role-delivery failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryFailure {
    code: SafeErrorCode,
    retryable: bool,
}

impl DeliveryFailure {
    /// Creates a failure from a validated safe code.
    #[must_use]
    pub const fn new(code: SafeErrorCode, retryable: bool) -> Self {
        Self { code, retryable }
    }

    /// Returns the safe machine-readable code.
    #[must_use]
    pub const fn code(&self) -> &SafeErrorCode {
        &self.code
    }

    /// Returns whether the durable queue should schedule a retry.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }
}

/// One role-specific delivery attempt outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryOutcome {
    /// The selected adapter accepted the event.
    Delivered,
    /// Shutdown cancelled delivery before an external acknowledgement.
    Cancelled,
    /// Delivery failed with safe retry semantics.
    Failed(DeliveryFailure),
}

/// Boxed asynchronous delivery future returned by an adapter port.
pub type DeliveryFuture<'a> = Pin<Box<dyn Future<Output = DeliveryOutcome> + Send + 'a>>;

/// Desktop-notification or relay-transport delivery boundary.
pub trait EventDelivery: Send + Sync {
    /// Attempts one leased event while observing cooperative cancellation.
    fn deliver<'a>(
        &'a self,
        event: &'a CanonicalEvent,
        cancellation: CancellationToken,
    ) -> DeliveryFuture<'a>;
}

/// Initializes exactly one role-specific adapter graph.
pub trait RoleDeliveryFactory: Send + Sync {
    /// Initializes the native desktop notification path.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::DeliveryInitialization`] without initializing the
    /// relay path when the desktop adapter is unavailable.
    fn desktop(&self) -> Result<Arc<dyn EventDelivery>, AgentError>;
    /// Initializes the SSH relay path.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::DeliveryInitialization`] without initializing the
    /// desktop path when the relay adapter is unavailable.
    fn relay(&self) -> Result<Arc<dyn EventDelivery>, AgentError>;
}

/// Stable lifecycle and composition failures.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum AgentError {
    /// Worker or shutdown bounds are invalid.
    #[error("agent runtime policy is invalid")]
    InvalidPolicy,
    /// The selected role adapter could not initialize.
    #[error("agent role adapter initialization failed")]
    DeliveryInitialization,
    /// The runtime has not reached readiness.
    #[error("agent is not ready")]
    NotReady,
    /// Shutdown has stopped accepting new events.
    #[error("agent is draining")]
    Draining,
    /// The durable queue rejected or could not process an operation.
    #[error("agent queue operation failed")]
    Queue(AgentQueueError),
    /// A worker or task join failed.
    #[error("agent worker failed")]
    RuntimeFailure,
}

/// Bounded task and delivery counts from one completed run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AgentRunReport {
    /// Long-lived worker tasks created at startup.
    pub workers_spawned: usize,
    /// Maximum simultaneous delivery attempts.
    pub peak_active_deliveries: usize,
    /// Successfully acknowledged events.
    pub delivered: usize,
    /// Events returned to the durable queue.
    pub released: usize,
    /// Failed delivery attempts scheduled for retry.
    pub retried: usize,
    /// Events reduced to metadata-only dead letters.
    pub dead_lettered: usize,
    /// Worker tasks aborted after the graceful deadline.
    pub forced_cancellations: usize,
}

/// Cloneable role-aware agent handle and lifecycle controller.
#[derive(Clone)]
pub struct AgentRuntime {
    inner: Arc<AgentInner>,
}

struct AgentInner {
    policy: AgentPolicy,
    queue: Arc<dyn AgentQueue>,
    delivery: Arc<dyn EventDelivery>,
    state: AtomicU8,
    notify: Notify,
    cancellation: CancellationToken,
    shutdown_code: SafeErrorCode,
    active: AtomicUsize,
    peak: AtomicUsize,
    delivered: AtomicUsize,
    released: AtomicUsize,
    retried: AtomicUsize,
    dead_lettered: AtomicUsize,
}

impl AgentRuntime {
    /// Composes the selected role without initializing the opposite adapter.
    ///
    /// # Errors
    ///
    /// Returns a fixed initialization error from the selected factory method.
    pub fn compose(
        role: RuntimeRole,
        policy: AgentPolicy,
        queue: Arc<dyn AgentQueue>,
        factory: &dyn RoleDeliveryFactory,
    ) -> Result<Self, AgentError> {
        let delivery = match role {
            RuntimeRole::Desktop => factory.desktop()?,
            RuntimeRole::Relay => factory.relay()?,
        };
        let shutdown_code =
            SafeErrorCode::parse("agent_shutdown").map_err(|_| AgentError::InvalidPolicy)?;
        Ok(Self {
            inner: Arc::new(AgentInner {
                policy,
                queue,
                delivery,
                state: AtomicU8::new(AgentState::Starting.as_u8()),
                notify: Notify::new(),
                cancellation: CancellationToken::new(),
                shutdown_code,
                active: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                delivered: AtomicUsize::new(0),
                released: AtomicUsize::new(0),
                retried: AtomicUsize::new(0),
                dead_lettered: AtomicUsize::new(0),
            }),
        })
    }

    /// Returns the current lifecycle state.
    #[must_use]
    pub fn state(&self) -> AgentState {
        AgentState::from_u8(self.inner.state.load(Ordering::Acquire))
    }

    /// Durably accepts one event only while the runtime is ready.
    ///
    /// # Errors
    ///
    /// Returns a readiness, draining, capacity, expiry, lock, corruption, or
    /// queue availability classification.
    pub fn submit(&self, event: &CanonicalEvent) -> Result<SubmissionOutcome, AgentError> {
        match self.state() {
            AgentState::Starting => return Err(AgentError::NotReady),
            AgentState::Draining | AgentState::Stopped => return Err(AgentError::Draining),
            AgentState::Ready => {}
        }
        let outcome = self.inner.queue.enqueue(event).map_err(AgentError::Queue)?;
        self.inner.notify.notify_one();
        Ok(match outcome {
            EnqueueResult::Enqueued => SubmissionOutcome::Accepted,
            EnqueueResult::Duplicate => SubmissionOutcome::Duplicate,
        })
    }

    /// Stops new submissions before an ingress adapter begins shutdown.
    pub fn begin_draining(&self) {
        let _ = self.inner.state.compare_exchange(
            AgentState::Ready.as_u8(),
            AgentState::Draining.as_u8(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// Starts the fixed worker set, waits for shutdown, then drains or safely
    /// releases every in-flight lease.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::RuntimeFailure`] if called more than once or a
    /// worker cannot complete a queue transition.
    pub async fn run_until<S>(&self, shutdown: S) -> Result<AgentRunReport, AgentError>
    where
        S: Future<Output = ()> + Send,
    {
        self.inner
            .state
            .compare_exchange(
                AgentState::Starting.as_u8(),
                AgentState::Ready.as_u8(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| AgentError::RuntimeFailure)?;

        let mut workers = JoinSet::new();
        for worker in 0..self.inner.policy.workers {
            let inner = Arc::clone(&self.inner);
            workers.spawn(async move { worker_loop(inner, worker).await });
        }

        shutdown.await;
        self.begin_draining();
        self.inner.cancellation.cancel();
        self.inner.notify.notify_waiters();

        let mut worker_failure = false;
        let graceful = timeout(self.inner.policy.shutdown_timeout, async {
            while let Some(result) = workers.join_next().await {
                if !matches!(result, Ok(Ok(()))) {
                    worker_failure = true;
                }
            }
        })
        .await;
        let forced_cancellations = if graceful.is_err() {
            let count = workers.len();
            workers.abort_all();
            while workers.join_next().await.is_some() {}
            count
        } else {
            0
        };
        self.inner
            .state
            .store(AgentState::Stopped.as_u8(), Ordering::Release);
        if worker_failure {
            return Err(AgentError::RuntimeFailure);
        }
        Ok(AgentRunReport {
            workers_spawned: self.inner.policy.workers,
            peak_active_deliveries: self.inner.peak.load(Ordering::Acquire),
            delivered: self.inner.delivered.load(Ordering::Acquire),
            released: self.inner.released.load(Ordering::Acquire),
            retried: self.inner.retried.load(Ordering::Acquire),
            dead_lettered: self.inner.dead_lettered.load(Ordering::Acquire),
            forced_cancellations,
        })
    }
}

async fn worker_loop(inner: Arc<AgentInner>, worker: usize) -> Result<(), AgentQueueError> {
    loop {
        if inner.cancellation.is_cancelled() {
            return Ok(());
        }
        let notified = inner.notify.notified();
        let Some(lease) = inner.queue.lease(worker)? else {
            if let Some(delay) = inner.queue.next_wake()? {
                let delay = delay.clamp(MIN_QUEUE_WAKE_DELAY, MAX_QUEUE_WAKE_DELAY);
                tokio::select! {
                    () = inner.cancellation.cancelled() => return Ok(()),
                    () = notified => continue,
                    () = tokio::time::sleep(delay) => continue,
                }
            } else {
                tokio::select! {
                    () = inner.cancellation.cancelled() => return Ok(()),
                    () = notified => continue,
                }
            }
        };
        let mut lease = LeaseGuard::new(Arc::clone(&inner.queue), lease, &inner.shutdown_code);
        let _active = ActiveGuard::new(&inner);
        let outcome = inner
            .delivery
            .deliver(lease.event(), inner.cancellation.clone())
            .await;
        match outcome {
            DeliveryOutcome::Delivered => {
                inner.queue.acknowledge(lease.get())?;
                lease.disarm();
                inner.delivered.fetch_add(1, Ordering::AcqRel);
            }
            DeliveryOutcome::Cancelled => {
                inner.queue.release(lease.get(), &inner.shutdown_code)?;
                lease.disarm();
                inner.released.fetch_add(1, Ordering::AcqRel);
            }
            DeliveryOutcome::Failed(failure) if failure.retryable() => {
                let retry = inner.queue.retry(lease.get(), failure.code())?;
                lease.disarm();
                match retry {
                    RetryResult::Scheduled => {
                        inner.retried.fetch_add(1, Ordering::AcqRel);
                    }
                    RetryResult::DeadLettered => {
                        inner.dead_lettered.fetch_add(1, Ordering::AcqRel);
                    }
                }
            }
            DeliveryOutcome::Failed(failure) => {
                inner.queue.dead_letter(lease.get(), failure.code())?;
                lease.disarm();
                inner.dead_lettered.fetch_add(1, Ordering::AcqRel);
            }
        }
    }
}

struct LeaseGuard<'a> {
    queue: Arc<dyn AgentQueue>,
    lease: Option<AgentLease>,
    shutdown_code: &'a SafeErrorCode,
}

impl<'a> LeaseGuard<'a> {
    fn new(
        queue: Arc<dyn AgentQueue>,
        lease: AgentLease,
        shutdown_code: &'a SafeErrorCode,
    ) -> Self {
        Self {
            queue,
            lease: Some(lease),
            shutdown_code,
        }
    }

    fn get(&self) -> &AgentLease {
        self.lease.as_ref().expect("armed lease guard")
    }

    fn event(&self) -> &CanonicalEvent {
        self.get().event()
    }

    fn disarm(&mut self) {
        self.lease = None;
    }
}

impl Drop for LeaseGuard<'_> {
    fn drop(&mut self) {
        if let Some(lease) = &self.lease {
            let _ = self.queue.release(lease, self.shutdown_code);
        }
    }
}

struct ActiveGuard<'a> {
    inner: &'a AgentInner,
}

impl<'a> ActiveGuard<'a> {
    fn new(inner: &'a AgentInner) -> Self {
        let active = inner.active.fetch_add(1, Ordering::AcqRel) + 1;
        inner.peak.fetch_max(active, Ordering::AcqRel);
        Self { inner }
    }
}

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.inner.active.fetch_sub(1, Ordering::AcqRel);
    }
}
