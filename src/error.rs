use std::{error::Error, fmt};

#[derive(Debug)]
pub enum UraError {
    MissingHome,
    MissingRuntimeDir,
    UnsupportedControlCommand(String),
    MpvCommandFailed(String),
    InvalidMpvResponse(String),
}

impl fmt::Display for UraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHome => write!(f, "HOME is not set; cannot locate config file"),
            Self::MissingRuntimeDir => {
                write!(
                    f,
                    "XDG_RUNTIME_DIR is not set; cannot locate mpv IPC socket"
                )
            }
            Self::UnsupportedControlCommand(command) => {
                write!(f, "unsupported control command `{command}`")
            }
            Self::MpvCommandFailed(error) => write!(f, "mpv command failed: {error}"),
            Self::InvalidMpvResponse(response) => {
                write!(f, "invalid mpv IPC response: {response}")
            }
        }
    }
}

impl Error for UraError {}
