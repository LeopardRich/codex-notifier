//! Command-line entry point for codex-notifier.

use std::io::Read;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use codex_notifier::desktop::{
    DesktopError, load_current_config, run_agent, submit_local_test, submit_remote_event,
};
use codex_notifier::installer::{InstallerError, install, status, uninstall};
use codex_notifier::lifecycle::RemovalDisposition;
use codex_notifier::{ApprovalRequestedEmitter, EmitError, TaskCompletedEmitter};
use codex_notifier_codex_source::{
    CodexCapabilityReport, CodexCliVersion, CodexInterface, MAX_APPROVAL_REQUESTED_INPUT_BYTES,
    MAX_TASK_COMPLETED_INPUT_BYTES, SourceContext,
};
use codex_notifier_core::EventKind;
use codex_notifier_ipc::{IpcEndpoint, IpcPolicy};
use codex_notifier_ssh_transport::{
    ReceiveError, diagnose_authorized_keys, diagnose_host_key, ipc_rejection,
    rejection_acknowledgement, safe_rejection, validate_receive_session, write_acknowledgement,
};
use time::OffsetDateTime;

const ARGUMENT_ERROR: &str = "command_arguments_invalid";
const STDIN_ERROR: &str = "command_stdin_failed";
const CODEX_VERSION_ERROR: &str = "install_codex_version_unavailable";
const SSH_PATH_ERROR: &str = "ssh_paths_unavailable";

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

struct CodexDoctorCommand {
    codex_version: String,
    interface: CodexInterface,
}

enum DoctorCommand {
    Codex(CodexDoctorCommand),
    Ssh(SshDoctorCommand),
}

struct SshDoctorCommand {
    ssh_config: Option<PathBuf>,
    known_hosts: Option<PathBuf>,
    authorized_keys: Option<PathBuf>,
}

enum Command {
    Emit(EmitCommand),
    Doctor(DoctorCommand),
    Receive,
    Agent,
    Install { codex_version: Option<String> },
    Uninstall,
    Status,
    Test(EventKind),
}

enum CommandError {
    Arguments,
    Stdin,
    SshPaths,
    CodexVersion,
    Emit(EmitError),
    Desktop(DesktopError),
    Installer(InstallerError),
    Receive(ReceiveError),
}

impl CommandError {
    fn code(&self) -> &str {
        match self {
            Self::Arguments => ARGUMENT_ERROR,
            Self::Stdin => STDIN_ERROR,
            Self::SshPaths => SSH_PATH_ERROR,
            Self::CodexVersion => CODEX_VERSION_ERROR,
            Self::Emit(error) => error.code(),
            Self::Desktop(error) => error.code(),
            Self::Installer(error) => error.code(),
            Self::Receive(error) => error.code(),
        }
    }

    const fn exit_code(&self) -> i32 {
        match self {
            Self::Arguments | Self::Stdin | Self::SshPaths | Self::CodexVersion => 2,
            Self::Emit(EmitError::Source(_)) => 3,
            Self::Emit(_) | Self::Desktop(_) | Self::Installer(_) | Self::Receive(_) => 4,
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
        Command::Doctor(DoctorCommand::Codex(command)) => {
            run_codex_doctor(&command);
            Ok(())
        }
        Command::Doctor(DoctorCommand::Ssh(command)) => run_ssh_doctor(&command),
        Command::Receive => run_receive().await,
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

async fn run_receive() -> Result<(), CommandError> {
    let session = validate_receive_session(
        std::env::var_os("SSH_ORIGINAL_COMMAND").as_deref(),
        std::env::var_os("SSH_CONNECTION").as_deref(),
        std::env::var_os("SSH_TTY").as_deref(),
    );
    let acknowledgement = if let Err(error) = session {
        rejection_acknowledgement(None, &error).map_err(CommandError::Receive)?
    } else {
        let stdin = std::io::stdin();
        match codex_notifier_ssh_transport::read_event(&mut stdin.lock(), OffsetDateTime::now_utc())
        {
            Err(error) => rejection_acknowledgement(None, &error).map_err(CommandError::Receive)?,
            Ok(event) => match submit_remote_event(&event).await {
                Ok(acknowledgement) => acknowledgement,
                Err(DesktopError::Ipc(error)) => {
                    ipc_rejection(event.event_id(), &error).map_err(CommandError::Receive)?
                }
                Err(error) => safe_rejection(
                    event.event_id(),
                    error.code(),
                    false,
                    "Desktop agent submission failed",
                )
                .map_err(CommandError::Receive)?,
            },
        }
    };

    write_acknowledgement(&mut std::io::stdout().lock(), &acknowledgement)
        .map_err(CommandError::Receive)
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

fn run_codex_doctor(command: &CodexDoctorCommand) {
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

fn run_ssh_doctor(command: &SshDoctorCommand) -> Result<(), CommandError> {
    let (_, config) = load_current_config().map_err(CommandError::Desktop)?;
    let ssh_directory = ssh_directory()?;
    let default_known_hosts = ssh_directory.join("known_hosts");
    let default_authorized_keys = ssh_directory.join("authorized_keys");
    let known_hosts = command
        .known_hosts
        .as_deref()
        .unwrap_or(&default_known_hosts);
    let authorized_keys = command
        .authorized_keys
        .as_deref()
        .unwrap_or(&default_authorized_keys);
    let host_key = diagnose_host_key(
        config.relay().ssh_host_alias(),
        known_hosts,
        command.ssh_config.as_deref(),
    );
    let authorized_keys = diagnose_authorized_keys(authorized_keys);
    println!(
        "host_key={}\nauthorized_keys={}",
        host_key.as_str(),
        authorized_keys.as_str()
    );
    Ok(())
}

fn ssh_directory() -> Result<PathBuf, CommandError> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    let home = home.map(PathBuf::from).ok_or(CommandError::SshPaths)?;
    if !home.is_absolute() {
        return Err(CommandError::SshPaths);
    }
    Ok(home.join(".ssh"))
}

fn parse_command(mut arguments: impl Iterator<Item = String>) -> Result<Command, CommandError> {
    match arguments.next().as_deref() {
        Some("emit") => parse_emit(arguments).map(Command::Emit),
        Some("doctor") => parse_doctor(arguments).map(Command::Doctor),
        Some("receive") if arguments.next().is_none() => Ok(Command::Receive),
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
    match arguments.next().as_deref() {
        Some("ssh") => return parse_ssh_doctor(arguments),
        Some("codex") => {}
        _ => return Err(CommandError::Arguments),
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
    Ok(DoctorCommand::Codex(CodexDoctorCommand {
        codex_version: codex_version.ok_or(CommandError::Arguments)?,
        interface,
    }))
}

fn parse_ssh_doctor(
    mut arguments: impl Iterator<Item = String>,
) -> Result<DoctorCommand, CommandError> {
    let mut known_hosts = None;
    let mut authorized_keys = None;
    let mut ssh_config = None;
    while let Some(flag) = arguments.next() {
        let value = PathBuf::from(arguments.next().ok_or(CommandError::Arguments)?);
        let target = match flag.as_str() {
            "--ssh-config" => &mut ssh_config,
            "--known-hosts" => &mut known_hosts,
            "--authorized-keys" => &mut authorized_keys,
            _ => return Err(CommandError::Arguments),
        };
        if !value.is_absolute() || target.replace(value).is_some() {
            return Err(CommandError::Arguments);
        }
    }
    Ok(DoctorCommand::Ssh(SshDoctorCommand {
        ssh_config,
        known_hosts,
        authorized_keys,
    }))
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
    fn stage_15_commands_reject_trailing_and_duplicate_arguments() {
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
        assert!(matches!(
            parse_command(arguments(&["receive"])),
            Ok(Command::Receive)
        ));
        assert!(matches!(
            parse_command(arguments(&["receive", "extra"])),
            Err(CommandError::Arguments)
        ));
        assert!(matches!(
            parse_command(arguments(&["doctor", "ssh"])),
            Ok(Command::Doctor(DoctorCommand::Ssh(_)))
        ));
        assert!(matches!(
            parse_command(arguments(&["doctor", "ssh", "extra"])),
            Err(CommandError::Arguments)
        ));
        assert!(matches!(
            parse_command(arguments(&[
                "doctor",
                "ssh",
                "--authorized-keys",
                "relative"
            ])),
            Err(CommandError::Arguments)
        ));
    }
}
