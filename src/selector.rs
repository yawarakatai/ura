use std::{
    fs::{File, OpenOptions},
    io::{self, IsTerminal, Read, Write},
    process::{Command, Stdio},
};

use anyhow::{Context, Result};

use crate::node::{DeviceKind, DeviceSet, DeviceSummary};

pub fn select_device(devices: &DeviceSet) -> Result<String> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!("device name is required when stdin or stdout is not a terminal");
    }
    if devices.devices.is_empty() {
        anyhow::bail!("no playback devices are available");
    }

    let tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .with_context(|| "failed to open /dev/tty")?;
    let mut terminal = RawTerminal::enter(tty)?;
    let mut query = String::new();
    let mut cursor = initial_cursor(devices, &query);

    loop {
        let filtered = filtered_indices(devices, &query);
        if filtered.is_empty() {
            cursor = 0;
        } else if cursor >= filtered.len() {
            cursor = filtered.len() - 1;
        }
        render(terminal.file_mut(), devices, &filtered, cursor, &query)?;

        let mut buffer = [0_u8; 16];
        let read = terminal.file_mut().read(&mut buffer)?;
        if read == 0 {
            continue;
        }
        let input = &buffer[..read];

        if input == b"\r" || input == b"\n" {
            if let Some(index) = filtered.get(cursor) {
                return Ok(devices.devices[*index].name.clone());
            }
            continue;
        }
        if input == [3] || input == [27] {
            anyhow::bail!("device selection cancelled");
        }
        if input.starts_with(&[27, b'[', b'A']) {
            cursor = cursor.saturating_sub(1);
            continue;
        }
        if input.starts_with(&[27, b'[', b'B']) {
            if !filtered.is_empty() {
                cursor = (cursor + 1).min(filtered.len() - 1);
            }
            continue;
        }
        if input == [127] || input == [8] {
            query.pop();
            cursor = initial_cursor(devices, &query);
            continue;
        }

        for byte in input {
            if byte.is_ascii_graphic() || *byte == b' ' {
                query.push(*byte as char);
            }
        }
        cursor = initial_cursor(devices, &query);
    }
}

fn initial_cursor(devices: &DeviceSet, query: &str) -> usize {
    let filtered = filtered_indices(devices, query);
    filtered
        .iter()
        .position(|index| devices.devices[*index].name == devices.selected)
        .unwrap_or(0)
}

fn filtered_indices(devices: &DeviceSet, query: &str) -> Vec<usize> {
    let query = query.trim().to_ascii_lowercase();
    devices
        .devices
        .iter()
        .enumerate()
        .filter(|(_, device)| matches_query(device, &query))
        .map(|(index, _)| index)
        .collect()
}

fn matches_query(device: &DeviceSummary, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let kind = match device.kind {
        DeviceKind::ThisDevice => "this device",
        DeviceKind::Peer => "peer",
    };
    device.name.to_ascii_lowercase().contains(query)
        || device
            .address
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains(query)
        || kind.contains(query)
}

fn render(
    tty: &mut File,
    devices: &DeviceSet,
    filtered: &[usize],
    cursor: usize,
    query: &str,
) -> Result<()> {
    write!(tty, "\x1b[2J\x1b[H")?;
    writeln!(tty, "Select playback device")?;
    if query.is_empty() {
        writeln!(tty, "Filter: ")?;
    } else {
        writeln!(tty, "Filter: {query}")?;
    }
    writeln!(tty)?;

    if filtered.is_empty() {
        writeln!(tty, "  No matching devices")?;
    } else {
        for (position, index) in filtered.iter().enumerate() {
            let device = &devices.devices[*index];
            let pointer = if position == cursor { ">" } else { " " };
            let current = if device.name == devices.selected {
                " (current)"
            } else {
                ""
            };
            match device.kind {
                DeviceKind::ThisDevice => {
                    writeln!(
                        tty,
                        "{pointer} {:<18} This device{current}",
                        device.name
                    )?;
                }
                DeviceKind::Peer => {
                    writeln!(
                        tty,
                        "{pointer} {:<18} {}{current}",
                        device.name,
                        device.address.as_deref().unwrap_or("peer")
                    )?;
                }
            }
        }
    }
    writeln!(tty)?;
    writeln!(tty, "↑/↓ move  type to filter  enter select  esc cancel")?;
    tty.flush()?;
    Ok(())
}

struct RawTerminal {
    file: File,
    saved_state: String,
}

impl RawTerminal {
    fn enter(file: File) -> Result<Self> {
        let saved = Command::new("stty")
            .arg("-g")
            .stdin(Stdio::from(file.try_clone()?))
            .output()
            .with_context(|| "failed to read terminal mode with stty")?;
        if !saved.status.success() {
            anyhow::bail!("stty could not read terminal mode");
        }
        let saved_state = String::from_utf8(saved.stdout)?.trim().to_string();
        let status = Command::new("stty")
            .args(["-icanon", "-echo", "-isig", "min", "0", "time", "1"])
            .stdin(Stdio::from(file.try_clone()?))
            .status()
            .with_context(|| "failed to enter raw terminal mode")?;
        if !status.success() {
            anyhow::bail!("stty could not enter raw terminal mode");
        }

        let mut terminal = Self { file, saved_state };
        write!(terminal.file, "\x1b[?1049h\x1b[?25l")?;
        terminal.file.flush()?;
        Ok(terminal)
    }

    fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        let _ = write!(self.file, "\x1b[?25h\x1b[?1049l");
        let _ = self.file.flush();
        if let Ok(stdin) = self.file.try_clone() {
            let _ = Command::new("stty")
                .arg(&self.saved_state)
                .stdin(Stdio::from(stdin))
                .status();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices() -> DeviceSet {
        DeviceSet {
            selected: "living-room".to_string(),
            devices: vec![
                DeviceSummary {
                    name: "desktop".to_string(),
                    kind: DeviceKind::ThisDevice,
                    address: None,
                },
                DeviceSummary {
                    name: "living-room".to_string(),
                    kind: DeviceKind::Peer,
                    address: Some("http://192.168.1.42:8765".to_string()),
                },
            ],
        }
    }

    #[test]
    fn filter_matches_name_address_and_kind() {
        let devices = devices();
        assert_eq!(filtered_indices(&devices, "desk"), vec![0]);
        assert_eq!(filtered_indices(&devices, "192.168"), vec![1]);
        assert_eq!(filtered_indices(&devices, "this"), vec![0]);
    }

    #[test]
    fn cursor_starts_on_current_device() {
        assert_eq!(initial_cursor(&devices(), ""), 1);
    }
}
