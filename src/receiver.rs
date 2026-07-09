use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
};

use anyhow::{Context, Result};

use crate::config::default_mpv_socket_path;

pub fn run_receive() -> Result<()> {
    ensure_program_in_path("mpv")?;
    ensure_program_in_path("yt-dlp")?;

    let socket_path = default_mpv_socket_path()?;
    create_runtime_dir(&socket_path)?;

    let mut receiver = Receiver::start(socket_path)?;
    println!(
        "receive: mpv started with IPC socket at {}",
        receiver.socket_path.display()
    );
    let status = receiver.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("mpv exited with status {status}"))
    }
}

struct Receiver {
    mpv: Child,
    socket_path: PathBuf,
}

impl Receiver {
    fn start(socket_path: PathBuf) -> Result<Self> {
        let mpv = Command::new("mpv")
            .args(mpv_args(&socket_path))
            .stdin(Stdio::null())
            .spawn()
            .with_context(|| "failed to start mpv")?;

        Ok(Self { mpv, socket_path })
    }

    fn wait(&mut self) -> Result<ExitStatus> {
        self.mpv
            .wait()
            .with_context(|| "failed to wait for mpv child process")
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        if let Ok(None) = self.mpv.try_wait() {
            let _ = self.mpv.kill();
            let _ = self.mpv.wait();
        }
    }
}

fn create_runtime_dir(socket_path: &Path) -> Result<()> {
    let runtime_dir = socket_path
        .parent()
        .expect("default mpv socket path should include a runtime directory");
    fs::create_dir_all(runtime_dir).with_context(|| {
        format!(
            "failed to create runtime directory {}",
            runtime_dir.display()
        )
    })
}

fn ensure_program_in_path(program: &str) -> Result<()> {
    let status = Command::new(program)
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

fn mpv_args(socket_path: &Path) -> Vec<String> {
    vec![
        "--idle=yes".to_string(),
        "--no-video".to_string(),
        "--force-window=no".to_string(),
        "--terminal=no".to_string(),
        format!("--input-ipc-server={}", socket_path.display()),
        "--ytdl-format=bestaudio/best".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before UNIX_EPOCH")
            .as_nanos();
        std::env::temp_dir().join(format!("ura-{name}-{nanos}"))
    }

    #[test]
    fn builds_audio_only_mpv_startup_args() {
        let socket_path = Path::new("/run/user/1000/ura/mpv.sock");

        assert_eq!(
            mpv_args(socket_path),
            vec![
                "--idle=yes",
                "--no-video",
                "--force-window=no",
                "--terminal=no",
                "--input-ipc-server=/run/user/1000/ura/mpv.sock",
                "--ytdl-format=bestaudio/best",
            ]
        );
    }

    #[test]
    fn creates_runtime_directory_for_socket() {
        let runtime_dir = unique_path("runtime-dir");
        let socket_path = runtime_dir.join("ura/mpv.sock");

        create_runtime_dir(&socket_path).expect("create runtime directory");

        assert!(runtime_dir.join("ura").is_dir());

        let _ = fs::remove_dir_all(runtime_dir);
    }
}
