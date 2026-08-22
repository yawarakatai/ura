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

pub struct PeerClient {
    base: PeerBase,
    token: String,
}

pub struct PeerPairingClient {
    base: PeerBase,
}

impl PeerPairingClient {
    pub fn new(peer_url: String) -> Result<Self> {
        Ok(Self {
            base: PeerBase::parse(&peer_url)?,
        })
    }

    pub fn info(&self) -> Result<PairInfo> {
        self.send("GET", "/v1/pair/info", None)?.json()
    }

    pub fn claim(&self, code: &str, device_name: &str) -> Result<PairClaim> {
        let body = serde_json::to_string(&PairClaimRequest { code, device_name })?;
        self.send("POST", "/v1/pair/claim", Some(&body))?.json()
    }

    fn send(&self, method: &str, path: &str, body: Option<&str>) -> Result<HttpResponse> {
        let mut stream = TcpStream::connect(&self.base.address)
            .with_context(|| format!("failed to connect to peer at {}", self.base.address))?;
        let body = body.unwrap_or("");
        let request = format!(
            "{method} {}{path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.base.path_prefix,
            self.base.host_header,
            body.len(),
            body
        );

        stream
            .write_all(request.as_bytes())
            .with_context(|| "failed to send HTTP request to peer")?;

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .with_context(|| "failed to read HTTP response from peer")?;
        HttpResponse::parse(&response)
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct PairInfo {
    #[serde(rename = "receiver_name")]
    pub node_name: String,
    pub expires_in: u64,
}

#[derive(Debug, serde::Deserialize)]
pub struct PairClaim {
    pub protocol_version: u8,
    #[serde(rename = "receiver_name")]
    pub node_name: String,
    pub token: String,
}

impl PeerClient {
    pub fn new(peer_url: String, token: String) -> Result<Self> {
        Ok(Self {
            base: PeerBase::parse(&peer_url)?,
            token,
        })
    }

    pub fn play(&self, url: &str) -> Result<()> {
        self.post_json("/v1/play", &PlayRequest { url, source: "cli" })
    }

    pub fn queue(&self, url: &str) -> Result<()> {
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
            .ok_or_else(|| anyhow::anyhow!("peer did not return loop status"))
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
            .with_context(|| format!("failed to connect to peer at {}", self.base.address))?;
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
            .with_context(|| "failed to send HTTP request to peer")?;

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .with_context(|| "failed to read HTTP response from peer")?;
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

#[derive(Debug, Serialize)]
struct PairClaimRequest<'a> {
    code: &'a str,
    device_name: &'a str,
}

#[derive(Debug, serde::Deserialize)]
struct ControlResponse {
    loop_status: Option<LoopStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PeerBase {
    address: String,
    host_header: String,
    path_prefix: String,
}

impl PeerBase {
    fn parse(peer_url: &str) -> Result<Self> {
        let rest = peer_url
            .strip_prefix("http://")
            .ok_or_else(|| anyhow::anyhow!("peer URL must start with http://"))?;
        let (authority, path_prefix) = match rest.split_once('/') {
            Some((authority, path)) => (
                authority,
                format!("/{path}").trim_end_matches('/').to_string(),
            ),
            None => (rest, String::new()),
        };
        if authority.is_empty() {
            bail!("peer URL is missing host");
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
            .ok_or_else(|| anyhow::anyhow!("invalid HTTP response from peer"))?;
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .ok_or_else(|| anyhow::anyhow!("invalid HTTP status from peer"))?
            .parse()
            .with_context(|| "invalid HTTP status from peer")?;

        Ok(Self {
            status,
            body: body.to_string(),
        })
    }

    fn ensure_success(&self) -> Result<()> {
        if (200..300).contains(&self.status) {
            Ok(())
        } else {
            bail!("peer returned HTTP {}: {}", self.status, self.body)
        }
    }

    fn json<T>(self) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.ensure_success()?;
        serde_json::from_str(&self.body).with_context(|| "failed to parse peer JSON response")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn parses_peer_base_url() {
        assert_eq!(
            PeerBase::parse("http://127.0.0.1:8765").expect("parse peer URL"),
            PeerBase {
                address: "127.0.0.1:8765".to_string(),
                host_header: "127.0.0.1:8765".to_string(),
                path_prefix: String::new(),
            }
        );
    }

    #[test]
    fn rejects_non_http_peer_url() {
        let error =
            PeerBase::parse("https://127.0.0.1:8765").expect_err("https peer URL should fail");

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

    #[test]
    fn play_uses_http_api() {
        let request = capture_request(r#"{"ok":true}"#, |client| {
            client.play("https://youtu.be/example")
        });

        assert_request_line(&request, "POST /v1/play HTTP/1.1");
        assert_authorization_header(&request);
        assert!(request.contains(r#""url":"https://youtu.be/example""#));
        assert!(request.contains(r#""source":"cli""#));
    }

    #[test]
    fn queue_uses_append_http_api() {
        let request = capture_request(r#"{"ok":true}"#, |client| {
            client.queue("https://youtu.be/example")
        });

        assert_request_line(&request, "POST /v1/enqueue HTTP/1.1");
        assert_authorization_header(&request);
        assert!(request.contains(r#""url":"https://youtu.be/example""#));
        assert!(request.contains(r#""source":"cli""#));
    }

    #[test]
    fn control_commands_use_http_api() {
        for command in ["toggle", "stop", "loop-off", "loop-one", "loop-queue"] {
            let request = capture_request(r#"{"ok":true,"loop_status":null}"#, |client| {
                client.control(command)
            });

            assert_request_line(&request, "POST /v1/control HTTP/1.1");
            assert_authorization_header(&request);
            assert!(request.contains(&format!(r#""command":"{command}""#)));
        }
    }

    #[test]
    fn loop_status_uses_http_api() {
        let request = capture_request(r#"{"ok":true,"loop_status":"Off"}"#, |client| {
            client.loop_status()
        });

        assert_request_line(&request, "POST /v1/control HTTP/1.1");
        assert_authorization_header(&request);
        assert!(request.contains(r#""command":"loop-status""#));
    }

    #[test]
    fn status_uses_http_api() {
        let request = capture_request(
            r#"{"pause":false,"idle_active":true,"path":null}"#,
            |client| client.status(),
        );

        assert_request_line(&request, "GET /v1/status HTTP/1.1");
        assert_authorization_header(&request);
    }

    #[test]
    fn history_uses_http_api() {
        let request = capture_request("[]", |client| client.history());

        assert_request_line(&request, "GET /v1/history HTTP/1.1");
        assert_authorization_header(&request);
    }

    fn capture_request<T, F>(response_body: &str, send: F) -> String
    where
        F: FnOnce(&PeerClient) -> Result<T>,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test peer");
        let address = listener.local_addr().expect("read test peer address");
        let response_body = response_body.to_string();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept client request");
            let request = read_http_request(&mut stream);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write test response");
            request
        });

        let client = PeerClient::new(
            format!("http://{address}"),
            "0123456789abcdef0123456789abcdef".to_string(),
        )
        .expect("create client");
        send(&client).expect("send client request");
        handle.join().expect("join test peer")
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut request = Vec::new();
        let mut buffer = [0; 512];

        loop {
            let read = stream.read(&mut buffer).expect("read request");
            assert!(
                read > 0,
                "client closed connection before request completed"
            );
            request.extend_from_slice(&buffer[..read]);

            if let Some(header_end) = header_end(&request) {
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = content_length(&headers);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }

        String::from_utf8(request).expect("request should be UTF-8")
    }

    fn header_end(request: &[u8]) -> Option<usize> {
        request.windows(4).position(|window| window == b"\r\n\r\n")
    }

    fn content_length(headers: &str) -> usize {
        headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    }

    fn assert_request_line(request: &str, expected: &str) {
        assert!(
            request.starts_with(expected),
            "expected request line `{expected}`, got:\n{request}"
        );
    }

    fn assert_authorization_header(request: &str) {
        assert!(request.contains("Authorization: Bearer 0123456789abcdef0123456789abcdef\r\n"));
    }
}
