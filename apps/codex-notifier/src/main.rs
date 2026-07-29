//! Command-line entry point for codex-notifier.

use std::io::Read;
use std::path::PathBuf;

use codex_notifier::{ApprovalRequestedEmitter, EmitError, TaskCompletedEmitter};
use codex_notifier_codex_source::{
    MAX_APPROVAL_REQUESTED_INPUT_BYTES, MAX_TASK_COMPLETED_INPUT_BYTES, SourceContext,
};
use codex_notifier_ipc::{IpcEndpoint, IpcPolicy};

const ARGUMENT_ERROR: &str = "command_arguments_invalid";
const STDIN_ERROR: &str = "command_stdin_failed";

#[derive(Clone, Copy)]
enum EmitSource {
    TaskCompleted,
    ApprovalRequested,
}

impl EmitSource {
    const fn input_limit(self) -> usize {
        match self {
            Self::TaskCompleted => MAX_TASK_COMPLETED_INPUT_BYTES,
            Self::ApprovalRequested => MAX_APPROVAL_REQUESTED_INPUT_BYTES,
        }
    }
}

struct EmitCommand {
    source: EmitSource,
    codex_version: String,
    state_dir: PathBuf,
    ipc_profile: String,
    host_label: String,
    project_label: Option<String>,
    routing_profile: Option<String>,
}

enum CommandError {
    Arguments,
    Stdin,
    Emit(EmitError),
}

impl CommandError {
    fn code(&self) -> &str {
        match self {
            Self::Arguments => ARGUMENT_ERROR,
            Self::Stdin => STDIN_ERROR,
            Self::Emit(error) => error.code(),
        }
    }

    const fn exit_code(&self) -> i32 {
        match self {
            Self::Arguments | Self::Stdin => 2,
            Self::Emit(EmitError::Source(_)) => 3,
            Self::Emit(_) => 4,
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{}", error.code());
        std::process::exit(error.exit_code());
    }
}

async fn run() -> Result<(), CommandError> {
    let command = parse_emit(std::env::args().skip(1))?;
    let endpoint = IpcEndpoint::new(command.state_dir.join("run"), command.ipc_profile)
        .map_err(EmitError::from)
        .map_err(CommandError::Emit)?;
    let context = SourceContext::new(
        command.host_label,
        command.project_label,
        command.routing_profile,
    )
    .map_err(EmitError::from)
    .map_err(CommandError::Emit)?;
    let input = read_stdin(command.source.input_limit())?;
    match command.source {
        EmitSource::TaskCompleted => {
            TaskCompletedEmitter::new(
                &command.codex_version,
                endpoint,
                context,
                IpcPolicy::default(),
            )
            .map_err(CommandError::Emit)?
            .emit(&input)
            .await
            .map_err(CommandError::Emit)?;
        }
        EmitSource::ApprovalRequested => {
            ApprovalRequestedEmitter::new(
                &command.codex_version,
                endpoint,
                context,
                IpcPolicy::default(),
            )
            .map_err(CommandError::Emit)?
            .emit(&input)
            .await
            .map_err(CommandError::Emit)?;
        }
    }
    Ok(())
}

fn parse_emit(mut arguments: impl Iterator<Item = String>) -> Result<EmitCommand, CommandError> {
    if arguments.next().as_deref() != Some("emit") {
        return Err(CommandError::Arguments);
    }
    let source = match arguments.next().as_deref() {
        Some("task-completed") => EmitSource::TaskCompleted,
        Some("approval-requested") => EmitSource::ApprovalRequested,
        _ => return Err(CommandError::Arguments),
    };

    let mut codex_version = None;
    let mut state_dir = None;
    let mut ipc_profile = None;
    let mut host_label = None;
    let mut project_label = None;
    let mut routing_profile = None;
    while let Some(flag) = arguments.next() {
        let value = arguments.next().ok_or(CommandError::Arguments)?;
        let target = match flag.as_str() {
            "--codex-version" => &mut codex_version,
            "--state-dir" => &mut state_dir,
            "--ipc-profile" => &mut ipc_profile,
            "--host-label" => &mut host_label,
            "--project-label" => &mut project_label,
            "--routing-profile" => &mut routing_profile,
            _ => return Err(CommandError::Arguments),
        };
        if target.replace(value).is_some() {
            return Err(CommandError::Arguments);
        }
    }

    Ok(EmitCommand {
        source,
        codex_version: codex_version.ok_or(CommandError::Arguments)?,
        state_dir: PathBuf::from(state_dir.ok_or(CommandError::Arguments)?),
        ipc_profile: ipc_profile.ok_or(CommandError::Arguments)?,
        host_label: host_label.ok_or(CommandError::Arguments)?,
        project_label,
        routing_profile,
    })
}

fn read_stdin(maximum: usize) -> Result<Vec<u8>, CommandError> {
    let stdin = std::io::stdin();
    let mut input = Vec::new();
    stdin
        .lock()
        .take(u64::try_from(maximum + 1).expect("bounded input limit"))
        .read_to_end(&mut input)
        .map_err(|_| CommandError::Stdin)?;
    Ok(input)
}
