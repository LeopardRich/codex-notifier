//! Executable composition root and user-level agent host.

pub mod desktop;
pub mod installer;
pub mod lifecycle;
pub mod platform;

use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codex_notifier_application::{
    AgentError, AgentLease, AgentPolicy, AgentQueue, AgentQueueError, AgentRunReport, AgentRuntime,
    EnqueueResult, RetryResult, RoleDeliveryFactory, RuntimeRole, SafeErrorCode, SubmissionOutcome,
};
use codex_notifier_codex_source::{
    ApprovalRequestedAdapter, ApprovalRequestedContext, CodexCliVersion, CodexInterface,
    SourceError, TaskCompletedAdapter, TaskCompletedContext,
};
use codex_notifier_config::{Config, Role};
use codex_notifier_core::{CanonicalEvent, EventId};
use codex_notifier_ipc::{
    AckError, AckStatus, Acknowledgement, IpcClient, IpcEndpoint, IpcError, IpcPolicy, IpcServer,
    ServeReport,
};
use codex_notifier_persistence::{
    EnqueueOutcome, PersistenceError, RetryOutcome, SqliteStore, StorePolicy,
};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::sync::watch;

const DEFAULT_WORKERS: usize = 4;
const DATABASE_FILE: &str = "events.sqlite3";
const DEFAULT_RETRY_INITIAL_DELAY_MS: u64 = 1_000;
const DEFAULT_RETRY_MAX_DELAY_MS: u64 = 60_000;
const RELAY_LEASE_ALLOWANCE_MS: u64 = 10_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RetrySchedule {
    initial_delay_ms: u64,
    max_delay_ms: u64,
}

impl RetrySchedule {
    const fn new(initial_delay_ms: u64, max_delay_ms: u64) -> Result<Self, AgentError> {
        if initial_delay_ms == 0 || initial_delay_ms > max_delay_ms {
            return Err(AgentError::InvalidPolicy);
        }
        Ok(Self {
            initial_delay_ms,
            max_delay_ms,
        })
    }

    fn delay_ms(self, attempt: u32) -> u64 {
        let shift = attempt.saturating_sub(1).min(63);
        let multiplier = 1_u64.checked_shl(shift).unwrap_or(u64::MAX);
        let base = self
            .initial_delay_ms
            .saturating_mul(multiplier)
            .min(self.max_delay_ms);
        let jitter_window = base / 4;
        let floor = base.saturating_sub(jitter_window);
        floor.saturating_add(fastrand::u64(0..=jitter_window))
    }
}

impl Default for RetrySchedule {
    fn default() -> Self {
        Self {
            initial_delay_ms: DEFAULT_RETRY_INITIAL_DELAY_MS,
            max_delay_ms: DEFAULT_RETRY_MAX_DELAY_MS,
        }
    }
}

/// Safe Codex event emission failures.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum EmitError {
    /// Codex version, interface, payload, or trusted context is incompatible.
    #[error("Codex event input is incompatible")]
    Source(#[from] SourceError),
    /// Local agent IPC could not complete safely.
    #[error("local agent submission failed")]
    Ipc(#[from] IpcError),
    /// The local agent returned a validated structured rejection.
    #[error("local agent rejected the event")]
    Rejected(AckError),
}

impl EmitError {
    /// Returns a stable safe diagnostic code without payload or endpoint data.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::Source(error) => error.code().as_str(),
            Self::Ipc(error) => error.code().as_str(),
            Self::Rejected(error) => error.code(),
        }
    }

    /// Returns whether a later submission may succeed without changing input.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        match self {
            Self::Source(_) => false,
            Self::Ipc(error) => matches!(
                error,
                IpcError::ConnectionFailed | IpcError::Timeout | IpcError::TransportFailure
            ),
            Self::Rejected(error) => error.retryable(),
        }
    }
}

/// Fixture-gated Codex task-completion normalizer and local IPC client.
#[derive(Clone, Debug)]
pub struct TaskCompletedEmitter {
    adapter: TaskCompletedAdapter,
    context: TaskCompletedContext,
    client: IpcClient,
}

impl TaskCompletedEmitter {
    /// Selects the exact versioned CLI hook adapter and local endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`EmitError::Source`] when `codex_version` has no verified
    /// task-completion fixture.
    pub fn new(
        codex_version: &str,
        endpoint: IpcEndpoint,
        context: TaskCompletedContext,
        ipc_policy: IpcPolicy,
    ) -> Result<Self, EmitError> {
        let version = codex_version.parse::<CodexCliVersion>()?;
        let adapter = TaskCompletedAdapter::new(version, CodexInterface::CliHook)?;
        Ok(Self {
            adapter,
            context,
            client: IpcClient::new(endpoint, ipc_policy),
        })
    }

    /// Normalizes one bounded hook payload and submits it to the local agent.
    ///
    /// A fresh `UUIDv7` and receive time are assigned before any IPC attempt.
    ///
    /// # Errors
    ///
    /// Returns a source compatibility error, local IPC error, or validated
    /// agent rejection. Error display text never contains input data.
    pub async fn emit(&self, input: &[u8]) -> Result<Acknowledgement, EmitError> {
        let received_at = OffsetDateTime::now_utc();
        let event = self
            .adapter
            .normalize(input, &self.context, EventId::new_v7(), received_at)?;
        submit_event(&self.client, &event).await
    }
}

/// Fixture-gated Codex approval-request normalizer and local IPC client.
#[derive(Clone, Debug)]
pub struct ApprovalRequestedEmitter {
    adapter: ApprovalRequestedAdapter,
    context: ApprovalRequestedContext,
    client: IpcClient,
}

impl ApprovalRequestedEmitter {
    /// Selects the exact versioned app-server adapter and local endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`EmitError::Source`] when `codex_version` has no verified
    /// app-server approval-request fixture.
    pub fn new(
        codex_version: &str,
        endpoint: IpcEndpoint,
        context: ApprovalRequestedContext,
        ipc_policy: IpcPolicy,
    ) -> Result<Self, EmitError> {
        let version = codex_version.parse::<CodexCliVersion>()?;
        let adapter = ApprovalRequestedAdapter::new(version, CodexInterface::AppServer)?;
        Ok(Self {
            adapter,
            context,
            client: IpcClient::new(endpoint, ipc_policy),
        })
    }

    /// Normalizes one bounded app-server request and submits it locally.
    ///
    /// A fresh `UUIDv7` is assigned while the app-server `startedAtMs` value
    /// supplies the canonical occurrence time.
    ///
    /// # Errors
    ///
    /// Returns a source compatibility error, local IPC error, or validated
    /// agent rejection. Command and approval data never enter error text.
    pub async fn emit(&self, input: &[u8]) -> Result<Acknowledgement, EmitError> {
        let received_at = OffsetDateTime::now_utc();
        let event = self
            .adapter
            .normalize(input, &self.context, EventId::new_v7(), received_at)?;
        submit_event(&self.client, &event).await
    }
}

async fn submit_event(
    client: &IpcClient,
    event: &CanonicalEvent,
) -> Result<Acknowledgement, EmitError> {
    let acknowledgement = client.submit(event).await?;
    if acknowledgement.status() == AckStatus::Rejected {
        let error = acknowledgement
            .error()
            .cloned()
            .ok_or(IpcError::MalformedAcknowledgement)?;
        return Err(EmitError::Rejected(error));
    }
    Ok(acknowledgement)
}

/// Stable composition failures that do not expose paths or payloads.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HostError {
    /// Local IPC endpoint binding or serving failed.
    #[error("agent local IPC failed")]
    Ipc(#[from] IpcError),
    /// Durable state initialization failed.
    #[error("agent durable state failed")]
    Persistence(#[from] PersistenceError),
    /// Role or lifecycle composition failed.
    #[error("agent runtime composition failed")]
    Agent(#[from] AgentError),
    /// The configured state directory could not be created.
    #[error("agent state directory is unavailable")]
    StateDirectory,
    /// A fixed acknowledgement template could not be constructed.
    #[error("agent acknowledgement configuration is invalid")]
    Acknowledgement,
}

/// Completed local IPC and worker lifecycle counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostRunReport {
    /// Local connection completion and rejection counts.
    pub ipc: ServeReport,
    /// Worker, delivery, retry, and cancellation counts.
    pub agent: AgentRunReport,
}

/// User-level single-instance host composed around real local IPC.
pub struct AgentHost {
    server: IpcServer,
    runtime: AgentRuntime,
    rejection: RejectionTemplates,
}

impl AgentHost {
    /// Binds the per-profile endpoint before initializing the selected role.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Ipc`] with `ipc_already_running` for a second agent,
    /// without calling either role factory method. Other failures are stable
    /// IPC, queue, or selected-role initialization classifications.
    pub fn bind(
        endpoint: IpcEndpoint,
        ipc_policy: IpcPolicy,
        role: RuntimeRole,
        agent_policy: AgentPolicy,
        queue: Arc<dyn AgentQueue>,
        factory: &dyn RoleDeliveryFactory,
    ) -> Result<Self, HostError> {
        let server = IpcServer::bind(endpoint, ipc_policy)?;
        let runtime = AgentRuntime::compose(role, agent_policy, queue, factory)?;
        Ok(Self {
            server,
            runtime,
            rejection: RejectionTemplates::new()?,
        })
    }

    /// Composes validated configuration, local IPC, `SQLite`, and the selected
    /// role adapter.
    ///
    /// # Errors
    ///
    /// Returns a stable directory, IPC, persistence, policy, or selected-role
    /// initialization failure.
    pub fn from_config(
        config: &Config,
        factory: &dyn RoleDeliveryFactory,
    ) -> Result<Self, HostError> {
        let state_dir = config.storage().state_dir();
        std::fs::create_dir_all(state_dir).map_err(|_| HostError::StateDirectory)?;
        let endpoint_label = config
            .agent()
            .ipc_endpoint()
            .name()
            .unwrap_or_else(|| config.agent().profile());
        let endpoint = IpcEndpoint::new(state_dir.join("run"), endpoint_label)?;
        let server = IpcServer::bind(endpoint, IpcPolicy::default())?;

        let role = match config.agent().role() {
            Role::Desktop => RuntimeRole::Desktop,
            Role::Relay => RuntimeRole::Relay,
        };
        let mut store_policy =
            StorePolicy::default().with_queue_limit(config.storage().max_queue_entries())?;
        let retry_schedule = if role == RuntimeRole::Relay {
            store_policy = store_policy
                .with_lease_duration_ms(relay_lease_duration_ms(
                    config.relay().connect_timeout_ms(),
                ))?
                .with_max_attempts(config.relay().retry_max_attempts())?;
            RetrySchedule::new(
                config.relay().retry_initial_delay_ms(),
                config.relay().retry_max_delay_ms(),
            )?
        } else {
            RetrySchedule::default()
        };
        let store = SqliteStore::open(&state_dir.join(DATABASE_FILE), store_policy)?;
        let queue: Arc<dyn AgentQueue> =
            Arc::new(SqliteAgentQueue::with_retry_schedule(store, retry_schedule));
        let shutdown_timeout = Duration::from_millis(config.agent().shutdown_timeout_ms());
        let policy = AgentPolicy::new(DEFAULT_WORKERS, shutdown_timeout)?;
        let runtime = AgentRuntime::compose(role, policy, queue, factory)?;
        Ok(Self {
            server,
            runtime,
            rejection: RejectionTemplates::new()?,
        })
    }

    /// Returns a cloneable runtime handle for readiness and local tests.
    #[must_use]
    pub fn runtime(&self) -> AgentRuntime {
        self.runtime.clone()
    }

    /// Runs IPC and workers until shutdown, stopping acceptance before drain.
    ///
    /// # Errors
    ///
    /// Returns a classified IPC or worker failure after both sides have been
    /// stopped and all in-flight leases have been resolved or released.
    pub async fn run_until<S>(self, shutdown: S) -> Result<HostRunReport, HostError>
    where
        S: Future<Output = ()> + Send,
    {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let runtime = self.runtime.clone();
        let handler_runtime = self.runtime;
        let rejection = self.rejection;
        let handler = Arc::new(move |event: CanonicalEvent| {
            acknowledgement_for(&handler_runtime, &rejection, &event)
        });
        let runtime_shutdown = wait_for_shutdown(shutdown_rx.clone());
        let server_shutdown = wait_for_shutdown(shutdown_rx);
        let runtime_future = runtime.run_until(runtime_shutdown);
        let server_future = self.server.serve_until(handler, server_shutdown);
        tokio::pin!(runtime_future);
        tokio::pin!(server_future);
        tokio::pin!(shutdown);

        let mut runtime_result = None;
        let mut server_result = None;
        tokio::select! {
            () = &mut shutdown => {}
            result = &mut runtime_future => runtime_result = Some(result),
            result = &mut server_future => server_result = Some(result),
        }
        runtime.begin_draining();
        shutdown_tx.send_replace(true);
        let agent = match runtime_result {
            Some(result) => result?,
            None => runtime_future.await?,
        };
        let ipc = match server_result {
            Some(result) => result?,
            None => server_future.await?,
        };
        Ok(HostRunReport { ipc, agent })
    }
}

async fn wait_for_shutdown(mut receiver: watch::Receiver<bool>) {
    if *receiver.borrow() {
        return;
    }
    let _ = receiver.changed().await;
}

#[derive(Clone)]
struct RejectionTemplates {
    not_ready: AckError,
    draining: AckError,
    queue_full: AckError,
    queue_expired: AckError,
    queue_locked: AckError,
    queue_corrupt: AckError,
    queue_unavailable: AckError,
    runtime: AckError,
}

impl RejectionTemplates {
    fn new() -> Result<Self, HostError> {
        Ok(Self {
            not_ready: ack_error("agent_not_ready", true, "Agent is not ready")?,
            draining: ack_error("agent_draining", true, "Agent is shutting down")?,
            queue_full: ack_error("agent_queue_full", true, "Agent queue is full")?,
            queue_expired: ack_error("agent_event_expired", false, "Event is too old")?,
            queue_locked: ack_error("agent_queue_locked", true, "Agent queue is busy")?,
            queue_corrupt: ack_error("agent_queue_corrupt", false, "Agent state is invalid")?,
            queue_unavailable: ack_error(
                "agent_queue_unavailable",
                true,
                "Agent queue is unavailable",
            )?,
            runtime: ack_error("agent_runtime_failed", true, "Agent runtime failed")?,
        })
    }

    fn for_error(&self, error: AgentError) -> &AckError {
        match error {
            AgentError::NotReady => &self.not_ready,
            AgentError::Draining => &self.draining,
            AgentError::Queue(AgentQueueError::Full) => &self.queue_full,
            AgentError::Queue(AgentQueueError::Expired) => &self.queue_expired,
            AgentError::Queue(AgentQueueError::Locked) => &self.queue_locked,
            AgentError::Queue(AgentQueueError::Corrupt) => &self.queue_corrupt,
            AgentError::Queue(AgentQueueError::Unavailable) => &self.queue_unavailable,
            _ => &self.runtime,
        }
    }
}

fn ack_error(code: &str, retryable: bool, message: &str) -> Result<AckError, HostError> {
    AckError::new(code, retryable, message).map_err(|_| HostError::Acknowledgement)
}

fn acknowledgement_for(
    runtime: &AgentRuntime,
    templates: &RejectionTemplates,
    event: &CanonicalEvent,
) -> Acknowledgement {
    match runtime.submit(event) {
        Ok(SubmissionOutcome::Accepted) => Acknowledgement::accepted(event.event_id()),
        Ok(SubmissionOutcome::Duplicate) => Acknowledgement::duplicate(event.event_id()),
        Err(error) => {
            Acknowledgement::rejected(event.event_id(), templates.for_error(error).clone())
        }
    }
}

/// Thread-safe `SQLite` implementation of the durable agent queue port.
pub struct SqliteAgentQueue {
    store: Mutex<SqliteStore>,
    next_lease: AtomicU64,
    retry_schedule: RetrySchedule,
}

impl SqliteAgentQueue {
    /// Wraps an initialized `SQLite` store for bounded multi-worker use.
    #[must_use]
    pub const fn new(store: SqliteStore) -> Self {
        Self {
            store: Mutex::new(store),
            next_lease: AtomicU64::new(0),
            retry_schedule: RetrySchedule {
                initial_delay_ms: DEFAULT_RETRY_INITIAL_DELAY_MS,
                max_delay_ms: DEFAULT_RETRY_MAX_DELAY_MS,
            },
        }
    }

    const fn with_retry_schedule(store: SqliteStore, retry_schedule: RetrySchedule) -> Self {
        Self {
            store: Mutex::new(store),
            next_lease: AtomicU64::new(0),
            retry_schedule,
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, SqliteStore>, AgentQueueError> {
        self.store.lock().map_err(|_| AgentQueueError::Unavailable)
    }
}

impl AgentQueue for SqliteAgentQueue {
    fn enqueue(&self, event: &CanonicalEvent) -> Result<EnqueueResult, AgentQueueError> {
        let outcome = self
            .lock()?
            .enqueue(event, now_ms())
            .map_err(|error| map_persistence_error(&error))?;
        Ok(match outcome {
            EnqueueOutcome::Enqueued => EnqueueResult::Enqueued,
            EnqueueOutcome::Duplicate => EnqueueResult::Duplicate,
        })
    }

    fn lease(&self, worker: usize) -> Result<Option<AgentLease>, AgentQueueError> {
        let sequence = self.next_lease.fetch_add(1, Ordering::Relaxed);
        let token = format!("w{worker}_{sequence}");
        self.lock()?
            .lease_next(now_ms(), &token)
            .map_err(|error| map_persistence_error(&error))?
            .map(|leased| {
                AgentLease::new(
                    leased.event().clone(),
                    leased.lease_token(),
                    leased.attempt(),
                )
            })
            .transpose()
    }

    fn next_wake(&self) -> Result<Option<Duration>, AgentQueueError> {
        let now = now_ms();
        self.lock()?
            .next_wake_at_ms(now)
            .map_err(|error| map_persistence_error(&error))
            .map(|available| {
                available.map(|available| {
                    Duration::from_millis(
                        u64::try_from(available.saturating_sub(now)).unwrap_or_default(),
                    )
                })
            })
    }

    fn acknowledge(&self, lease: &AgentLease) -> Result<(), AgentQueueError> {
        self.lock()?
            .acknowledge(lease.event().event_id(), lease.token(), now_ms())
            .map_err(|error| map_persistence_error(&error))
    }

    fn retry(
        &self,
        lease: &AgentLease,
        code: &SafeErrorCode,
    ) -> Result<RetryResult, AgentQueueError> {
        let now = now_ms();
        let delay = i64::try_from(self.retry_schedule.delay_ms(lease.attempt()))
            .map_err(|_| AgentQueueError::Corrupt)?;
        let available = now.saturating_add(delay);
        self.lock()?
            .retry(
                lease.event().event_id(),
                lease.token(),
                now,
                available,
                code.as_str(),
            )
            .map(|outcome| match outcome {
                RetryOutcome::Scheduled => RetryResult::Scheduled,
                RetryOutcome::DeadLettered => RetryResult::DeadLettered,
            })
            .map_err(|error| map_persistence_error(&error))
    }

    fn release(&self, lease: &AgentLease, code: &SafeErrorCode) -> Result<(), AgentQueueError> {
        self.lock()?
            .release_lease(
                lease.event().event_id(),
                lease.token(),
                now_ms(),
                code.as_str(),
            )
            .map_err(|error| map_persistence_error(&error))
    }

    fn dead_letter(&self, lease: &AgentLease, code: &SafeErrorCode) -> Result<(), AgentQueueError> {
        self.lock()?
            .dead_letter(
                lease.event().event_id(),
                lease.token(),
                code.as_str(),
                now_ms(),
            )
            .map_err(|error| map_persistence_error(&error))
    }
}

fn map_persistence_error(error: &PersistenceError) -> AgentQueueError {
    match error {
        PersistenceError::QueueFull => AgentQueueError::Full,
        PersistenceError::EventExpired => AgentQueueError::Expired,
        PersistenceError::DatabaseLocked => AgentQueueError::Locked,
        PersistenceError::LeaseConflict
        | PersistenceError::NotFound
        | PersistenceError::CorruptData
        | PersistenceError::UnsupportedSchema
        | PersistenceError::MigrationFailed
        | PersistenceError::InvalidValue => AgentQueueError::Corrupt,
        PersistenceError::StorageUnwritable | PersistenceError::DatabaseFailure => {
            AgentQueueError::Unavailable
        }
        _ => AgentQueueError::Unavailable,
    }
}

fn now_ms() -> i64 {
    let value = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
    i64::try_from(value).unwrap_or_else(|_| {
        if value.is_negative() {
            i64::MIN
        } else {
            i64::MAX
        }
    })
}

const fn relay_lease_duration_ms(connect_timeout_ms: u64) -> u64 {
    connect_timeout_ms.saturating_add(RELAY_LEASE_ALLOWANCE_MS)
}

/// Returns the database path owned by the agent for diagnostics and tests.
#[must_use]
pub fn database_path(state_dir: &Path) -> std::path::PathBuf {
    state_dir.join(DATABASE_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_schedule_is_exponential_jittered_and_capped() {
        let schedule = RetrySchedule::new(100, 800).expect("retry schedule");
        for (attempt, base) in [(1, 100), (2, 200), (3, 400), (4, 800), (8, 800)] {
            for _ in 0..64 {
                let delay = schedule.delay_ms(attempt);
                assert!(delay >= base - base / 4, "attempt {attempt}: {delay}");
                assert!(delay <= base, "attempt {attempt}: {delay}");
            }
        }
        assert_eq!(RetrySchedule::new(0, 100), Err(AgentError::InvalidPolicy));
        assert_eq!(RetrySchedule::new(101, 100), Err(AgentError::InvalidPolicy));
    }

    #[test]
    fn relay_lease_outlives_the_maximum_ssh_operation() {
        assert_eq!(relay_lease_duration_ms(100), 10_100);
        assert_eq!(relay_lease_duration_ms(120_000), 130_000);
        assert!(relay_lease_duration_ms(120_000) > 125_000);
    }
}
