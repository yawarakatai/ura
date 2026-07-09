use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::UraError;

pub struct MpvClient {
    stream: UnixStream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMode {
    Off,
    One,
    Queue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopStatus {
    Off,
    One,
    Queue,
    Custom {
        loop_file: Option<String>,
        loop_playlist: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MpvStatus {
    pub pause: Option<bool>,
    pub idle_active: Option<bool>,
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MpvResponse {
    error: String,
    data: Option<Value>,
}

impl MpvClient {
    pub fn connect(socket_path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket_path)?;
        Ok(Self { stream })
    }

    pub fn load_replace(&mut self, url: &str) -> Result<()> {
        self.send_command(loadfile_command(url, LoadMode::Replace))?;
        Ok(())
    }

    pub fn load_enqueue(&mut self, url: &str) -> Result<()> {
        self.send_command(loadfile_command(url, LoadMode::AppendPlay))?;
        Ok(())
    }

    pub fn pause(&mut self) -> Result<()> {
        self.set_pause(true)
    }

    pub fn resume(&mut self) -> Result<()> {
        self.set_pause(false)
    }

    pub fn toggle(&mut self) -> Result<()> {
        self.send_command(json!({ "command": ["cycle", "pause"] }))?;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        self.send_command(json!({ "command": ["stop"] }))?;
        Ok(())
    }

    pub fn set_loop_mode(&mut self, mode: LoopMode) -> Result<()> {
        for command in loop_mode_commands(mode) {
            self.send_command(command)?;
        }
        Ok(())
    }

    pub fn loop_status(&mut self) -> Result<LoopStatus> {
        Ok(normalize_loop_status(
            self.get_string_property("loop-file")?,
            self.get_string_property("loop-playlist")?,
        ))
    }

    pub fn status(&mut self) -> Result<MpvStatus> {
        Ok(MpvStatus {
            pause: self.get_bool_property("pause")?,
            idle_active: self.get_bool_property("idle-active")?,
            path: self.get_string_property("path")?,
        })
    }

    pub fn control(&mut self, command: &str) -> Result<()> {
        match command {
            "toggle" => self.toggle(),
            "stop" => self.stop(),
            "pause" => self.pause(),
            "resume" => self.resume(),
            other => Err(UraError::UnsupportedControlCommand(other.to_string()).into()),
        }
    }

    fn set_pause(&mut self, pause: bool) -> Result<()> {
        self.send_command(json!({ "command": ["set_property", "pause", pause] }))?;
        Ok(())
    }

    fn get_bool_property(&mut self, name: &str) -> Result<Option<bool>> {
        let response = self.send_command(json!({ "command": ["get_property", name] }))?;
        match response.data {
            Some(Value::Bool(value)) => Ok(Some(value)),
            Some(Value::Null) | None => Ok(None),
            Some(value) => bail!(UraError::InvalidMpvResponse(value.to_string())),
        }
    }

    fn get_string_property(&mut self, name: &str) -> Result<Option<String>> {
        let response = self.send_command(json!({ "command": ["get_property", name] }))?;
        match response.data {
            Some(Value::String(value)) => Ok(Some(value)),
            Some(Value::Null) | None => Ok(None),
            Some(value) => bail!(UraError::InvalidMpvResponse(value.to_string())),
        }
    }

    fn send_command(&mut self, command: Value) -> Result<MpvResponse> {
        let mut line = serde_json::to_vec(&command)?;
        line.push(b'\n');
        self.stream.write_all(&line)?;
        self.stream.flush()?;

        let mut reader = BufReader::new(self.stream.try_clone()?);
        let mut response = String::new();
        reader.read_line(&mut response)?;
        parse_response(&response)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadMode {
    Replace,
    AppendPlay,
}

impl LoadMode {
    fn as_mpv_arg(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::AppendPlay => "append-play",
        }
    }
}

fn loadfile_command(url: &str, mode: LoadMode) -> Value {
    json!({ "command": ["loadfile", url, mode.as_mpv_arg()] })
}

fn set_property_command(name: &str, value: &str) -> Value {
    json!({ "command": ["set_property", name, value] })
}

fn loop_mode_properties(mode: LoopMode) -> [(&'static str, &'static str); 2] {
    match mode {
        LoopMode::Off => [("loop-file", "no"), ("loop-playlist", "no")],
        LoopMode::One => [("loop-file", "inf"), ("loop-playlist", "no")],
        LoopMode::Queue => [("loop-file", "no"), ("loop-playlist", "inf")],
    }
}

fn loop_mode_commands(mode: LoopMode) -> [Value; 2] {
    loop_mode_properties(mode).map(|(name, value)| set_property_command(name, value))
}

fn normalize_loop_status(loop_file: Option<String>, loop_playlist: Option<String>) -> LoopStatus {
    match (loop_file.as_deref(), loop_playlist.as_deref()) {
        (Some("inf"), _) => LoopStatus::One,
        (_, Some("inf")) => LoopStatus::Queue,
        (Some("no"), Some("no")) => LoopStatus::Off,
        _ => LoopStatus::Custom {
            loop_file,
            loop_playlist,
        },
    }
}

fn parse_response(response: &str) -> Result<MpvResponse> {
    let response: MpvResponse = serde_json::from_str(response)
        .map_err(|_| UraError::InvalidMpvResponse(response.trim().to_string()))?;
    if response.error == "success" {
        Ok(response)
    } else {
        Err(UraError::MpvCommandFailed(response.error).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_replace_loadfile_command() {
        assert_eq!(
            loadfile_command("https://youtu.be/example", LoadMode::Replace),
            json!({ "command": ["loadfile", "https://youtu.be/example", "replace"] })
        );
    }

    #[test]
    fn builds_enqueue_loadfile_command() {
        assert_eq!(
            loadfile_command("https://youtu.be/example", LoadMode::AppendPlay),
            json!({ "command": ["loadfile", "https://youtu.be/example", "append-play"] })
        );
    }

    #[test]
    fn builds_loop_off_commands() {
        assert_eq!(
            loop_mode_commands(LoopMode::Off),
            [
                json!({ "command": ["set_property", "loop-file", "no"] }),
                json!({ "command": ["set_property", "loop-playlist", "no"] }),
            ]
        );
    }

    #[test]
    fn builds_loop_one_commands() {
        assert_eq!(
            loop_mode_commands(LoopMode::One),
            [
                json!({ "command": ["set_property", "loop-file", "inf"] }),
                json!({ "command": ["set_property", "loop-playlist", "no"] }),
            ]
        );
    }

    #[test]
    fn builds_loop_queue_commands() {
        assert_eq!(
            loop_mode_commands(LoopMode::Queue),
            [
                json!({ "command": ["set_property", "loop-file", "no"] }),
                json!({ "command": ["set_property", "loop-playlist", "inf"] }),
            ]
        );
    }

    #[test]
    fn normalizes_loop_status() {
        assert_eq!(
            normalize_loop_status(Some("no".to_string()), Some("no".to_string())),
            LoopStatus::Off
        );
        assert_eq!(
            normalize_loop_status(Some("inf".to_string()), Some("no".to_string())),
            LoopStatus::One
        );
        assert_eq!(
            normalize_loop_status(Some("no".to_string()), Some("inf".to_string())),
            LoopStatus::Queue
        );
        assert_eq!(
            normalize_loop_status(Some("2".to_string()), Some("no".to_string())),
            LoopStatus::Custom {
                loop_file: Some("2".to_string()),
                loop_playlist: Some("no".to_string()),
            }
        );
    }

    #[test]
    fn parses_success_response() {
        let response =
            parse_response(r#"{"error":"success","data":true}"#).expect("parse success response");

        assert_eq!(response.error, "success");
        assert_eq!(response.data, Some(Value::Bool(true)));
    }

    #[test]
    fn rejects_mpv_error_response() {
        let error = parse_response(r#"{"error":"property unavailable"}"#)
            .expect_err("mpv error should fail");

        assert!(error.to_string().contains("mpv command failed"));
    }

    #[test]
    fn unsupported_control_command_is_clear() {
        let error = UraError::UnsupportedControlCommand("next".to_string());

        assert_eq!(error.to_string(), "unsupported control command `next`");
    }
}
