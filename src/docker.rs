use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::io::Read;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cargo_metadata::Message;

use crate::cli::PullPolicy;
use crate::error::{Error, Result};

pub struct BuildSpec<'a> {
    pub image: &'a OsStr,
    pub pull: PullPolicy,
    pub user_mapping: &'a UserMapping,
    pub workspace_root: &'a Path,
    pub target_directory: &'a Path,
    pub cargo_home: &'a Path,
    pub container_workdir: &'a str,
    pub target: &'a str,
    pub cargo_args: &'a [OsString],
    pub env: &'a [String],
}

pub struct BuildResult {
    pub status_code: i32,
    pub executables: Vec<PathBuf>,
}

#[derive(Debug)]
pub enum UserMapping {
    KeepId,
    Root,
    Explicit { uid: String, gid: String },
}

pub fn detect_user_mapping() -> Result<UserMapping> {
    let podman_probe = Command::new("docker")
        .args(["info", "--format", "{{.Host.Security.Rootless}}"])
        .output()
        .map_err(|source| Error::Spawn {
            program: "docker",
            source,
        })?;

    // Podman's Docker-compatible CLI exposes the Host.Security.Rootless field.
    // keep-id works for both rootless and rootful Podman and preserves bind-mount
    // ownership.
    if podman_probe.status.success() {
        return Ok(UserMapping::KeepId);
    }

    let docker_probe = Command::new("docker")
        .args(["info", "--format", "{{json .SecurityOptions}}"])
        .output()
        .map_err(|source| Error::Spawn {
            program: "docker",
            source,
        })?;
    if !docker_probe.status.success() {
        return Err(Error::Message(format!(
            "Docker daemon is unavailable: {}",
            String::from_utf8_lossy(&docker_probe.stderr).trim()
        )));
    }

    let security_options = String::from_utf8_lossy(&docker_probe.stdout);
    if security_options.contains("rootless") {
        // Root in a rootless Docker container maps to the host user.
        Ok(UserMapping::Root)
    } else {
        let (uid, gid) = uid_gid()?;
        Ok(UserMapping::Explicit { uid, gid })
    }
}

pub fn build_command(spec: &BuildSpec<'_>) -> Result<Command> {
    let workspace_mount = bind_mount(spec.workspace_root, "/workspace")?;
    let target_mount = bind_mount(spec.target_directory, "/target")?;
    let cargo_home_mount = bind_mount(spec.cargo_home, "/cargo-home")?;

    let mut command = Command::new("docker");
    command
        .arg("run")
        .arg("--rm")
        .arg("--name")
        .arg(format!(
            "cargo-build-musl-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
        .arg("--pull")
        .arg(spec.pull.as_str())
        .arg("--mount")
        .arg(workspace_mount)
        .arg("--mount")
        .arg(target_mount)
        .arg("--mount")
        .arg(cargo_home_mount)
        .arg("--workdir")
        .arg(spec.container_workdir)
        .arg("--env")
        .arg("CARGO_HOME=/cargo-home")
        .arg("--env")
        .arg("CARGO_TARGET_DIR=/target");

    match spec.user_mapping {
        UserMapping::KeepId => {
            command.arg("--userns=keep-id");
        }
        UserMapping::Root => {}
        UserMapping::Explicit { uid, gid } => {
            command.arg("--user").arg(format!("{uid}:{gid}"));
        }
    }

    forward_proxy_environment(&mut command);

    for name in spec.env {
        if std::env::var_os(name).is_none() {
            return Err(Error::Message(format!(
                "requested environment variable `{name}` is not set on the host"
            )));
        }
        // Pass only the name: Docker inherits its value without exposing it in
        // the command line or dry-run output.
        command.arg("--env").arg(name);
    }

    command
        .arg(spec.image)
        .arg("cargo")
        .arg("build")
        .arg("--target")
        .arg(spec.target)
        .arg("--message-format")
        .arg("json-render-diagnostics")
        .args(spec.cargo_args);

    Ok(command)
}

pub fn run_build(mut command: Command, host_target: &Path) -> Result<BuildResult> {
    let name = command
        .get_args()
        .collect::<Vec<_>>()
        .windows(2)
        .find(|pair| pair[0] == "--name")
        .map(|pair| pair[1].to_owned())
        .ok_or_else(|| Error::Message("build container has no name".into()))?;
    let signals = BuildSignals::new()?;
    // Own cancellation centrally: terminal SIGINT must not kill the attached
    // engine client before we have removed its container.
    #[cfg(unix)]
    command.process_group(0);
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let mut child = command.spawn().map_err(|source| Error::Spawn {
        program: "docker",
        source,
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Message("failed to capture Docker stdout".to_owned()))?;

    let target = host_target.to_owned();
    let reader = thread::spawn(move || read_messages(stdout, &target));
    let status = loop {
        let signal = signals.received.load(Ordering::Relaxed);
        if signal != 0 {
            eprintln!(
                "   Cancelling build; removing container {}",
                name.to_string_lossy()
            );
            // First remove while the engine client is still attached. Then reap
            // the client (also cancels a pending image pull), and remove again to
            // cover a container being created concurrently with cancellation.
            let _ = remove_container(&name);
            let _ = child.kill();
            child.wait().map_err(Error::BuildOutput)?;
            remove_container(&name)?;
            return Ok(BuildResult {
                status_code: 128 + signal as i32,
                executables: Vec::new(),
            });
        }
        if let Some(status) = child.try_wait().map_err(Error::BuildOutput)? {
            break status;
        }
        thread::sleep(Duration::from_millis(50));
    };
    let executables = reader
        .join()
        .map_err(|_| Error::Message("build output reader panicked".into()))?;

    Ok(BuildResult {
        status_code: status.code().unwrap_or(1),
        executables: executables?,
    })
}

struct BuildSignals {
    received: Arc<AtomicUsize>,
    registrations: Vec<signal_hook::SigId>,
}

impl BuildSignals {
    fn new() -> Result<Self> {
        let mut signals = Self {
            received: Arc::new(AtomicUsize::new(0)),
            registrations: Vec::new(),
        };
        for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
            signals.registrations.push(
                signal_hook::flag::register_usize(
                    signal,
                    Arc::clone(&signals.received),
                    signal as usize,
                )
                .map_err(|error| {
                    Error::Message(format!("cannot install build signal handler: {error}"))
                })?,
            );
        }
        Ok(signals)
    }
}

impl Drop for BuildSignals {
    fn drop(&mut self) {
        for id in self.registrations.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}

fn remove_container(name: &OsStr) -> Result<()> {
    let mut command = Command::new("docker");
    #[cfg(unix)]
    command.process_group(0);
    let output = command
        .args([OsStr::new("rm"), OsStr::new("--force"), name])
        .output()
        .map_err(|source| Error::Spawn {
            program: "docker rm",
            source,
        })?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success()
        || stderr.contains("No such container")
        || stderr.contains("no such container")
    {
        Ok(())
    } else {
        Err(Error::Message(format!(
            "failed to remove build container `{}`: {}",
            name.to_string_lossy(),
            stderr.trim()
        )))
    }
}

fn read_messages(stdout: impl Read, host_target: &Path) -> Result<Vec<PathBuf>> {
    let mut executables = BTreeSet::new();
    for message in Message::parse_stream(std::io::BufReader::new(stdout)) {
        match message.map_err(Error::CargoMessage)? {
            Message::CompilerMessage(message) => {
                if let Some(rendered) = message.message.rendered {
                    eprint!("{rendered}");
                }
            }
            Message::CompilerArtifact(artifact) => {
                if let Some(path) = artifact.executable {
                    let container_path = Path::new(path.as_str());
                    let relative = container_path.strip_prefix("/target").map_err(|_| {
                        Error::Message(format!(
                            "Cargo reported an artifact outside /target: {container_path:?}"
                        ))
                    })?;
                    executables.insert(host_target.join(relative));
                }
            }
            Message::TextLine(line) => println!("{line}"),
            _ => {}
        }
    }

    Ok(executables.into_iter().collect())
}

fn bind_mount(source: &Path, target: &str) -> Result<OsString> {
    let source = source
        .to_str()
        .ok_or_else(|| Error::Message(format!("path `{}` is not valid UTF-8", source.display())))?;
    if source.contains(',') {
        return Err(Error::Message(format!(
            "Docker mount source contains an unsupported comma: `{source}`"
        )));
    }

    Ok(OsString::from(format!(
        "type=bind,source={source},target={target}"
    )))
}

fn uid_gid() -> Result<(String, String)> {
    Ok((id("-u")?, id("-g")?))
}

fn id(flag: &'static str) -> Result<String> {
    let output = Command::new("id")
        .arg(flag)
        .output()
        .map_err(|source| Error::Spawn {
            program: "id",
            source,
        })?;

    if !output.status.success() {
        return Err(Error::Message(format!("`id {flag}` failed")));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn forward_proxy_environment(command: &mut Command) {
    const VARIABLES: [&str; 6] = [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "no_proxy",
    ];

    for variable in VARIABLES {
        if std::env::var_os(variable).is_some() {
            command.arg("--env").arg(variable);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_docker_command_with_persistent_host_mounts() {
        let cargo_args = [OsString::from("--release")];
        let spec = BuildSpec {
            image: OsStr::new("rust:test"),
            pull: PullPolicy::Never,
            user_mapping: &UserMapping::Explicit {
                uid: "1000".to_owned(),
                gid: "100".to_owned(),
            },
            workspace_root: Path::new("/tmp/workspace"),
            target_directory: Path::new("/tmp/workspace/target"),
            cargo_home: Path::new("/tmp/cargo-home"),
            container_workdir: "/workspace",
            target: "x86_64-unknown-linux-musl",
            cargo_args: &cargo_args,
            env: &[],
        };

        let command = build_command(&spec).unwrap();
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(args.contains(&"CARGO_HOME=/cargo-home".to_owned()));
        assert!(args.contains(&"CARGO_TARGET_DIR=/target".to_owned()));
        assert!(args.contains(&"x86_64-unknown-linux-musl".to_owned()));
        assert_eq!(args.last().unwrap(), "--release");
    }
}
