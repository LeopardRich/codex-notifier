//! Command-line entry point for codex-notifier.

use std::io::Read;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use codex_notifier::desktop::{DesktopError, run_agent, submit_local_test};
use codex_notifier::installer::{InstallerError, install, status, uninstall};
use codex_notifier::lifecycle::RemovalDisposition;
use codex_notifier::{ApprovalRequestedEmitter, EmitError, TaskCompletedEmitter};
use codex_notifier_codex_source::{
    CodexCapabilityReport, CodexCliVersion, CodexInterface, MAX_APPROVAL_REQUESTED_INPUT_BYTES,
    MAX_TASK_COMPLETED_INPUT_BYTES, SourceContext,
};
use codex_notifier_core::EventKind;
use codex_notifier_ipc::{IpcEndpoint, IpcPolicy};

const ARGUMENT_ERROR: &str = "command_arguments_invalid";
const STDIN_ERROR: &str = "command_stdin_failed";
const CODEX_VERSION_ERROR: &str = "install_codex_version_unavailable";

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

struct DoctorCommand {
    codex_version: String,
    interface: CodexInterface,
}

enum Command {
    Emit(EmitCommand),
    Doctor(DoctorCommand),
    Agent,
    Install { codex_version: Option<String> },
    Uninstall,
    Status,
    Test(EventKind),
}

enum CommandError {
    Arguments,
    Stdin,
    CodexVersion,
    Emit(EmitError),
    Desktop(DesktopError),
    Installer(InstallerError),
}

impl CommandError {
    fn code(&self) -> &str {
        match self {
            Self::Arguments => ARGUMENT_ERROR,
            Self::Stdin => STDIN_ERROR,
            Self::CodexVersion => CODEX_VERSION_ERROR,
            Self::Emit(error) => error.code(),
            Self::Desktop(error) => error.code(),
            Self::Installer(error) => error.code(),
        }
    }

    const fn exit_code(&self) -> i32 {
        match self {
            Self::Arguments | Self::Stdin | Self::CodexVersion => 2,
            Self::Emit(EmitError::Source(_)) => 3,
            Self::Emit(_) | Self::Desktop(_) | Self::Installer(_) => 4,
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
    match parse_command(std::env::args().skip(1))? {
        Command::Emit(command) => run_emit(command).await,
        Command::Doctor(command) => {
            run_doctor(&command);
            Ok(())
        }
        Command::Agent => run_agent().await.map_err(CommandError::Desktop),
        Command::Install { codex_version } => {
            let codex_version = codex_version.map_or_else(detect_codex_version, Ok)?;
            let report = install(&codex_version)
                .await
                .map_err(CommandError::Installer)?;
            println!(
                "installed=true\nupgraded={}\nagent_running={}\nnotification={}\nfocus={}\nhook_trust={}\napproval_installation=report_unavailable\napproval_notice={}",
                report.upgraded,
                report.agent_running,
                report.notification.status().as_str(),
                report.notification.focus().as_str(),
                report.hook_trust,
                report.approval_notice,
            );
            Ok(())
        }
        Command::Uninstall => {
            let report = uninstall().await.map_err(CommandError::Installer)?;
            println!(
                "installed=false\nhook_removed={}\nhook_preserved={}\nconfig_removed={}\nconfig_preserved={}\nstate_preserved={}",
                report.managed.hook == RemovalDisposition::Removed,
                report.managed.hook == RemovalDisposition::Preserved,
                report.managed.config == RemovalDisposition::Removed,
                report.managed.config == RemovalDisposition::Preserved,
                report.state_preserved,
            );
            Ok(())
        }
        Command::Status => {
            let report = status().map_err(CommandError::Installer)?;
            println!(
                "installed={}\nversion={}\nstartup_registered={}\nagent_running={}\nagent_stale={}\nprofile={}\nqueue_pending={}\ndelivery_receipts={}\ndead_letters={}\nnotification={}\nfocus={}",
                report.installed,
                report.version.as_deref().unwrap_or("none"),
                report.startup_registered,
                report.agent.running,
                report.agent.stale,
                report.agent.profile.as_deref().unwrap_or("none"),
                optional_count(report.queue_pending),
                optional_count(report.delivery_receipts),
                optional_count(report.dead_letters),
                report
                    .notification
                    .map_or("unavailable", |value| value.status().as_str()),
                report
                    .notification
                    .map_or("unavailable", |value| value.focus().as_str()),
            );
            Ok(())
        }
        Command::Test(kind) => {
            let (event_id, acknowledgement) = submit_local_test(kind)
                .await
                .map_err(CommandError::Desktop)?;
            println!(
                "event_id={event_id}\nacknowledgement={}",
                acknowledgement_name(acknowledgement)
            );
            Ok(())
        }
    }
}

async fn run_emit(command: EmitCommand) -> Result<(), CommandError> {
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

fn run_doctor(command: &DoctorCommand) {
    let report = CodexCapabilityReport::inspect(&command.codex_version, command.interface);
    let version = report
        .version()
        .map_or("unsupported", CodexCliVersion::as_str);
    println!(
        "codex_version={version}\ninterface={}\ntask_completed={}\napproval_requested={}\napproval_installation={}\napproval_notice={}",
        report.interface().as_str(),
        report.task_completed().as_str(),
        report.approval_requested().as_str(),
        report.approval_installation().as_str(),
        report.approval_installation_notice(),
    );
}

fn parse_command(mut arguments: impl Iterator<Item = String>) -> Result<Command, CommandError> {
    match arguments.next().as_deref() {
        Some("emit") => parse_emit(arguments).map(Command::Emit),
        Some("doctor") => parse_doctor(arguments).map(Command::Doctor),
        Some("agent") if arguments.next().is_none() => Ok(Command::Agent),
        Some("install") => parse_install(arguments),
        Some("uninstall") if arguments.next().is_none() => Ok(Command::Uninstall),
        Some("status") if arguments.next().is_none() => Ok(Command::Status),
        Some("test") => parse_test(arguments),
        _ => Err(CommandError::Arguments),
    }
}

fn parse_install(mut arguments: impl Iterator<Item = String>) -> Result<Command, CommandError> {
    let mut codex_version = None;
    while let Some(flag) = arguments.next() {
        let value = arguments.next().ok_or(CommandError::Arguments)?;
        if flag != "--codex-version" || codex_version.replace(value).is_some() {
            return Err(CommandError::Arguments);
        }
    }
    Ok(Command::Install { codex_version })
}

fn parse_test(mut arguments: impl Iterator<Item = String>) -> Result<Command, CommandError> {
    let kind = match arguments.next().as_deref() {
        None | Some("task-completed") => EventKind::TaskCompleted,
        Some("approval-requested") => EventKind::ApprovalRequested,
        Some(_) => return Err(CommandError::Arguments),
    };
    if arguments.next().is_some() {
        return Err(CommandError::Arguments);
    }
    Ok(Command::Test(kind))
}

fn parse_emit(mut arguments: impl Iterator<Item = String>) -> Result<EmitCommand, CommandError> {
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

fn parse_doctor(
    mut arguments: impl Iterator<Item = String>,
) -> Result<DoctorCommand, CommandError> {
    if arguments.next().as_deref() != Some("codex") {
        return Err(CommandError::Arguments);
    }
    let mut codex_version = None;
    let mut interface = None;
    while let Some(flag) = arguments.next() {
        let value = arguments.next().ok_or(CommandError::Arguments)?;
        let target = match flag.as_str() {
            "--codex-version" => &mut codex_version,
            "--interface" => &mut interface,
            _ => return Err(CommandError::Arguments),
        };
        if target.replace(value).is_some() {
            return Err(CommandError::Arguments);
        }
    }
    let interface = match interface.as_deref() {
        Some("cli-hook") => CodexInterface::CliHook,
        Some("app-server") => CodexInterface::AppServer,
        _ => return Err(CommandError::Arguments),
    };
    Ok(DoctorCommand {
        codex_version: codex_version.ok_or(CommandError::Arguments)?,
        interface,
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

fn detect_codex_version() -> Result<String, CommandError> {
    #[cfg(windows)]
    let output = ProcessCommand::new("cmd.exe")
        .args(["/d", "/c", "codex", "--version"])
        .output()
        .map_err(|_| CommandError::CodexVersion)?;
    #[cfg(not(windows))]
    let output = ProcessCommand::new("codex")
        .arg("--version")
        .output()
        .map_err(|_| CommandError::CodexVersion)?;
    if !output.status.success() || output.stdout.len() > 128 {
        return Err(CommandError::CodexVersion);
    }
    let output = std::str::from_utf8(&output.stdout).map_err(|_| CommandError::CodexVersion)?;
    output
        .trim()
        .strip_prefix("codex-cli ")
        .filter(|value| !value.is_empty() && value.is_ascii())
        .map(str::to_owned)
        .ok_or(CommandError::CodexVersion)
}

fn optional_count(value: Option<usize>) -> String {
    value.map_or_else(|| "unavailable".to_owned(), |value| value.to_string())
}

const fn acknowledgement_name(status: codex_notifier_ipc::AckStatus) -> &'static str {
    match status {
        codex_notifier_ipc::AckStatus::Accepted => "accepted",
        codex_notifier_ipc::AckStatus::Duplicate => "duplicate",
        codex_notifier_ipc::AckStatus::Delivered => "delivered",
        codex_notifier_ipc::AckStatus::Rejected => "rejected",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> impl Iterator<Item = String> {
        values
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn stage_14_commands_reject_trailing_and_duplicate_arguments() {
        assert!(matches!(
            parse_command(arguments(&["agent"])),
            Ok(Command::Agent)
        ));
        assert!(matches!(
            parse_command(arguments(&["agent", "extra"])),
            Err(CommandError::Arguments)
        ));
        assert!(matches!(
            parse_command(arguments(&[
                "install",
                "--codex-version",
                "0.144.5",
                "--codex-version",
                "0.144.5"
            ])),
            Err(CommandError::Arguments)
        ));
        assert!(matches!(
            parse_command(arguments(&["test", "approval-requested"])),
            Ok(Command::Test(EventKind::ApprovalRequested))
        ));
        assert!(matches!(
            parse_command(arguments(&["uninstall", "extra"])),
            Err(CommandError::Arguments)
        ));
    }
}
