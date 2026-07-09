use std::{
    io::{Read, Write},
    net::TcpStream,
};

use anyhow::{Context, Result, bail};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    db::HistoryEntry,
    mpv::{LoopStatus, MpvStatus},
};

pub struct HttpClient {
    base: ReceiverBase,
    token: String,
}

impl HttpClient {
    pub fn new(receiver_url: String, token: String) -> Result<Self> {
        Ok(Self {
            base: ReceiverBase::parse(&receiver_url)?,
            token,
        })
    }

    pub fn play(&self, url: &str) -> Result<()> {
        self.post_json("/v1/play", &PlayRequest { url, source: "cli" })
    }

    pub fn enqueue(&self, url: &str) -> Result<()> {
        self.post_json("/v1/enqueue", &PlayRequest { url, source: "cli" })
    }

    pub fn control(&self, command: &str) -> Result<()> {
        let _: ControlResponse =
            self.post_json_response("/v1/control", &ControlRequest { command })?;
        Ok(())
    }

    pub fn loop_status(&self) -> Result<LoopStatus> {
        let response: ControlResponse = self.post_json_response(
            "/v1/control",
            &ControlRequest {
                command: "loop-status",
            },
        )?;
        response
            .loop_status
            .ok_or_else(|| anyhow::anyhow!("receiver did not return loop status"))
    }

    pub fn status(&self) -> Result<MpvStatus> {
        self.get_json("/v1/status")
    }

    pub fn history(&self) -> Result<Vec<HistoryEntry>> {
        self.get_json("/v1/history")
    }

    fn post_json<T>(&self, path: &str, body: &T) -> Result<()>
    where
        T: Serialize,
    {
        let _: serde_json::Value = self.post_json_response(path, body)?;
        Ok(())
    }

    fn post_json_response<T, U>(&self, path: &str, body: &T) -> Result<U>
    where
        T: Serialize,
        U: DeserializeOwned,
    {
        let body = serde_json::to_string(body)?;
        let response = self.send("POST", path, Some(&body))?;
        response.json()
    }

    fn get_json<T>(&self, path: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let response = self.send("GET", path, None)?;
        response.json()
    }

    fn send(&self, method: &str, path: &str, body: Option<&str>) -> Result<HttpResponse> {
        let mut stream = TcpStream::connect(&self.base.address)
            .with_context(|| format!("failed to connect to receiver at {}", self.base.address))?;
        let body = body.unwrap_or("");
        let request = format!(
            "{method} {}{path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.base.path_prefix,
            self.base.host_header,
            self.token,
            body.len(),
            body
        );

        stream
            .write_all(request.as_bytes())
            .with_context(|| "failed to send HTTP request to receiver")?;

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .with_context(|| "failed to read HTTP response from receiver")?;
        HttpResponse::parse(&response)
    }
}

#[derive(Debug, Serialize)]
struct PlayRequest<'a> {
    url: &'a str,
    source: &'a str,
}

#[derive(Debug, Serialize)]
struct ControlRequest<'a> {
    command: &'a str,
}

#[derive(Debug, serde::Deserialize)]
struct ControlResponse {
    loop_status: Option<LoopStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReceiverBase {
    address: String,
    host_header: String,
    path_prefix: String,
}

impl ReceiverBase {
    fn parse(receiver_url: &str) -> Result<Self> {
        let rest = receiver_url
            .strip_prefix("http://")
            .ok_or_else(|| anyhow::anyhow!("receiver_url must start with http://"))?;
        let (authority, path_prefix) = match rest.split_once('/') {
            Some((authority, path)) => (
                authority,
                format!("/{path}").trim_end_matches('/').to_string(),
            ),
            None => (rest, String::new()),
        };
        if authority.is_empty() {
            bail!("receiver_url is missing host");
        }

        Ok(Self {
            address: authority.to_string(),
            host_header: authority.to_string(),
            path_prefix,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct HttpResponse {
    status: u16,
    body: String,
}

impl HttpResponse {
    fn parse(response: &str) -> Result<Self> {
        let (head, body) = response
            .split_once("\r\n\r\n")
            .ok_or_else(|| anyhow::anyhow!("invalid HTTP response from receiver"))?;
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .ok_or_else(|| anyhow::anyhow!("invalid HTTP status from receiver"))?
            .parse()
            .with_context(|| "invalid HTTP status from receiver")?;

        Ok(Self {
            status,
            body: body.to_string(),
        })
    }

    fn ensure_success(&self) -> Result<()> {
        if (200..300).contains(&self.status) {
            Ok(())
        } else {
            bail!("receiver returned HTTP {}: {}", self.status, self.body)
        }
    }

    fn json<T>(self) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.ensure_success()?;
        serde_json::from_str(&self.body).with_context(|| "failed to parse receiver JSON response")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_receiver_base_url() {
        assert_eq!(
            ReceiverBase::parse("http://127.0.0.1:8765").expect("parse receiver URL"),
            ReceiverBase {
                address: "127.0.0.1:8765".to_string(),
                host_header: "127.0.0.1:8765".to_string(),
                path_prefix: String::new(),
            }
        );
    }

    #[test]
    fn rejects_non_http_receiver_url() {
        let error = ReceiverBase::parse("https://127.0.0.1:8765")
            .expect_err("https receiver URL should fail");

        assert!(error.to_string().contains("http://"));
    }

    #[test]
    fn parses_http_response() {
        let response =
            HttpResponse::parse("HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"ok\":true}")
                .expect("parse HTTP response");

        assert_eq!(
            response,
            HttpResponse {
                status: 200,
                body: "{\"ok\":true}".to_string(),
            }
        );
    }
}
