use std::{
    fs,
    io::ErrorKind,
    os::unix::{fs::FileTypeExt, fs::PermissionsExt, net::UnixStream},
    path::{Path, PathBuf},
    process::{Command as StdCommand, ExitStatus, Stdio},
};

use anyhow::{Context, Result};
use tokio::{
    process::{Child, Command as TokioCommand},
    sync::oneshot,
    task::JoinHandle,
};
use tracing::info;

pub(crate) struct PlaybackRuntime {
    socket_path: PathBuf,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<ExitStatus>>>,
}

impl PlaybackRuntime {
    pub(crate) fn start(socket_path: PathBuf) -> Result<Self> {
        prepare_mpv_socket_path(&socket_path)?;
        let mpv = TokioCommand::new("mpv")
            .args(mpv_args(&socket_path))
            .stdin(Stdio::null())
            .spawn()
            .with_context(|| "failed to start mpv")?;
        match mpv.id() {
            Some(pid) => info!(pid, "mpv child started"),
            None => info!("mpv child started"),
        }
        let (shutdown, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(supervise_mpv_child(mpv, shutdown_rx));

        Ok(Self {
            socket_path,
            shutdown: Some(shutdown),
            task: Some(task),
        })
    }

    pub(crate) fn into_parts(
        mut self,
    ) -> (
        PathBuf,
        Option<oneshot::Sender<()>>,
        JoinHandle<Result<ExitStatus>>,
    ) {
        let socket_path = std::mem::take(&mut self.socket_path);
        let shutdown = self.shutdown.take();
        let task = self
            .task
            .take()
            .expect("playback process task should exist");
        (socket_path, shutdown, task)
    }
}

impl Drop for PlaybackRuntime {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

async fn supervise_mpv_child(
    mut mpv: Child,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<ExitStatus> {
    tokio::select! {
        result = mpv.wait() => {
            result.with_context(|| "failed to wait for mpv child process")
        }
        _ = &mut shutdown => {
            terminate_mpv_child(&mut mpv).await
        }
    }
}

async fn terminate_mpv_child(mpv: &mut Child) -> Result<ExitStatus> {
    if let Some(status) = mpv
        .try_wait()
        .with_context(|| "failed to check mpv child process status")?
    {
        return Ok(status);
    }

    mpv.start_kill()
        .with_context(|| "failed to terminate mpv child process")?;
    mpv.wait()
        .await
        .with_context(|| "failed to wait for mpv child process after termination")
}

pub(crate) fn exit_status_result(status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("mpv exited with status {status}"))
    }
}

pub(crate) fn create_runtime_dir(socket_path: &Path) -> Result<()> {
    let runtime_dir = socket_path
        .parent()
        .expect("default mpv socket path should include a runtime directory");
    fs::create_dir_all(runtime_dir).with_context(|| {
        format!(
            "failed to create runtime directory {}",
            runtime_dir.display()
        )
    })?;
    fs::set_permissions(runtime_dir, fs::Permissions::from_mode(0o700)).with_context(|| {
        format!(
            "failed to set runtime directory permissions on {}",
            runtime_dir.display()
        )
    })
}

pub(crate) fn prepare_mpv_socket_path(socket_path: &Path) -> Result<()> {
    prepare_live_socket_path(socket_path, "mpv IPC")
}

pub(crate) fn prepare_control_socket_path(socket_path: &Path) -> Result<()> {
    create_runtime_dir(socket_path)?;
    prepare_live_socket_path(socket_path, "control")
}

fn prepare_live_socket_path(socket_path: &Path, label: &str) -> Result<()> {
    let metadata = match fs::symlink_metadata(socket_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect {label} socket path {}",
                    socket_path.display()
                )
            });
        }
    };

    if !metadata.file_type().is_socket() {
        return Err(anyhow::anyhow!(
            "{label} socket path {} exists but is not a Unix socket",
            socket_path.display()
        ));
    }

    match UnixStream::connect(socket_path) {
        Ok(_) => Err(anyhow::anyhow!(
            "{label} socket {} is already in use; stop the existing node before starting a new one",
            socket_path.display()
        )),
        Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
            fs::remove_file(socket_path).with_context(|| {
                format!(
                    "failed to remove stale {label} socket {}",
                    socket_path.display()
                )
            })?;
            Ok(())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to connect to existing {label} socket {}",
                socket_path.display()
            )
        }),
    }
}

pub(crate) fn ensure_program_in_path(program: &str) -> Result<()> {
    let status = StdCommand::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to find `{program}` in PATH"))?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "`{program} --version` failed with status {status}"
        ))
    }
}

pub(crate) fn mpv_args(socket_path: &Path) -> Vec<String> {
    vec![
        "--idle=yes".to_string(),
        "--no-video".to_string(),
        "--force-window=no".to_string(),
        "--terminal=no".to_string(),
        format!("--input-ipc-server={}", socket_path.display()),
        "--ytdl-format=bestaudio/best".to_string(),
    ]
}
