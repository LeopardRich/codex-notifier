//! Executable composition root and user-level agent host.

use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codex_notifier_application::{
    AgentError, AgentLease, AgentPolicy, AgentQueue, AgentQueueError, AgentRunReport, AgentRuntime,
    EnqueueResult, RoleDeliveryFactory, RuntimeRole, SafeErrorCode, SubmissionOutcome,
};
use codex_notifier_config::{Config, Role};
use codex_notifier_core::CanonicalEvent;
use codex_notifier_ipc::{
    AckError, Acknowledgement, IpcEndpoint, IpcError, IpcPolicy, IpcServer, ServeReport,
};
use codex_notifier_persistence::{EnqueueOutcome, PersistenceError, SqliteStore, StorePolicy};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::sync::watch;

const DEFAULT_WORKERS: usize = 4;
const DATABASE_FILE: &str = "events.sqlite3";
const INITIAL_RETRY_DELAY_MS: i64 = 250;

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

    /// Composes validated configuration, local IPC, SQLite, and the selected
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

        let store_policy =
            StorePolicy::default().with_queue_limit(config.storage().max_queue_entries())?;
        let store = SqliteStore::open(&state_dir.join(DATABASE_FILE), store_policy)?;
        let queue: Arc<dyn AgentQueue> = Arc::new(SqliteAgentQueue::new(store));
        let role = match config.agent().role() {
            Role::Desktop => RuntimeRole::Desktop,
            Role::Relay => RuntimeRole::Relay,
        };
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

/// Thread-safe SQLite implementation of the durable agent queue port.
pub struct SqliteAgentQueue {
    store: Mutex<SqliteStore>,
    next_lease: AtomicU64,
}

impl SqliteAgentQueue {
    /// Wraps an initialized SQLite store for bounded multi-worker use.
    #[must_use]
    pub const fn new(store: SqliteStore) -> Self {
        Self {
            store: Mutex::new(store),
            next_lease: AtomicU64::new(0),
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
            .map_err(map_persistence_error)?;
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
            .map_err(map_persistence_error)?
            .map(|leased| AgentLease::new(leased.event().clone(), leased.lease_token()))
            .transpose()
    }

    fn acknowledge(&self, lease: &AgentLease) -> Result<(), AgentQueueError> {
        self.lock()?
            .acknowledge(lease.event().event_id(), lease.token(), now_ms())
            .map_err(map_persistence_error)
    }

    fn retry(&self, lease: &AgentLease, code: &SafeErrorCode) -> Result<(), AgentQueueError> {
        let now = now_ms();
        let available = now.saturating_add(INITIAL_RETRY_DELAY_MS);
        self.lock()?
            .retry(
                lease.event().event_id(),
                lease.token(),
                now,
                available,
                code.as_str(),
            )
            .map(|_| ())
            .map_err(map_persistence_error)
    }

    fn release(&self, lease: &AgentLease, code: &SafeErrorCode) -> Result<(), AgentQueueError> {
        self.lock()?
            .release_lease(
                lease.event().event_id(),
                lease.token(),
                now_ms(),
                code.as_str(),
            )
            .map_err(map_persistence_error)
    }

    fn dead_letter(&self, lease: &AgentLease, code: &SafeErrorCode) -> Result<(), AgentQueueError> {
        self.lock()?
            .dead_letter(
                lease.event().event_id(),
                lease.token(),
                code.as_str(),
                now_ms(),
            )
            .map_err(map_persistence_error)
    }
}

fn map_persistence_error(error: PersistenceError) -> AgentQueueError {
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

/// Returns the database path owned by the agent for diagnostics and tests.
#[must_use]
pub fn database_path(state_dir: &Path) -> std::path::PathBuf {
    state_dir.join(DATABASE_FILE)
}
