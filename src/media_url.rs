use std::{fmt, str::FromStr};

use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaUrlError {
    Malformed,
    Playlist,
    UnsupportedHost,
}

impl fmt::Display for MediaUrlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Malformed => "unsupported URL; expected a YouTube URL using http:// or https://",
            Self::Playlist => "playlist URLs are not supported in the MVP",
            Self::UnsupportedHost => {
                "unsupported URL; expected youtube.com, music.youtube.com, or youtu.be"
            }
        };
        formatter.write_str(message)
    }
}

pub(crate) fn validate(url: &str) -> Result<String, MediaUrlError> {
    let Some(parsed) = ParsedUrl::parse(url) else {
        log_validation_failure(url, "malformed URL or unsupported scheme");
        return Err(MediaUrlError::Malformed);
    };

    if parsed.is_supported_youtube_video_url() {
        log_validation_success(&parsed);
        return Ok(parsed.without_playlist_context());
    }

    if parsed.has_playlist_context() {
        log_validation_failure(url, "playlist URLs are not supported in the MVP");
        return Err(MediaUrlError::Playlist);
    }

    log_validation_failure(
        url,
        "unsupported host; expected youtube.com, music.youtube.com, or youtu.be",
    );
    Err(MediaUrlError::UnsupportedHost)
}

fn log_validation_success(parsed: &ParsedUrl) {
    match parsed.sanitized_youtube_video_id() {
        Some(video_id) => info!(
            host = %parsed.host,
            video_id = %video_id,
            "URL validation succeeded"
        ),
        None => info!(host = %parsed.host, "URL validation succeeded"),
    }
}

fn log_validation_failure(url: &str, reason: &'static str) {
    match safe_host_from_url(url) {
        Some(host) => warn!(host = %host, reason, "URL validation failed"),
        None => warn!(reason, "URL validation failed"),
    }
}

fn safe_host_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let (_, rest) = trimmed.split_once("://")?;
    let authority_with_query = rest
        .split_once('/')
        .map(|(authority, _)| authority)
        .unwrap_or(rest);
    let authority = authority_with_query
        .split_once('?')
        .map(|(authority, _)| authority)
        .unwrap_or(authority_with_query);

    authority
        .parse::<Authority>()
        .ok()
        .map(|authority| authority.host)
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedUrl {
    scheme: String,
    authority: String,
    host: String,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

impl ParsedUrl {
    fn parse(url: &str) -> Option<Self> {
        let url = url.trim();
        if url.is_empty() || url.chars().any(char::is_whitespace) {
            return None;
        }

        let (scheme, rest) = url.split_once("://")?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return None;
        }

        let (rest, fragment) = match rest.split_once('#') {
            Some((rest, fragment)) => (rest, Some(fragment.to_string())),
            None => (rest, None),
        };
        let (authority, path_and_query) = match rest.split_once('/') {
            Some((authority, path_and_query)) => (authority, format!("/{path_and_query}")),
            None => (rest, "/".to_string()),
        };
        if authority.is_empty() {
            return None;
        }

        let host = authority.parse::<Authority>().ok()?.host;
        let (path, query) = match path_and_query.split_once('?') {
            Some((path, query)) => (path.to_string(), Some(query.to_string())),
            None => (path_and_query, None),
        };

        Some(Self {
            scheme,
            authority: authority.to_string(),
            host,
            path,
            query,
            fragment,
        })
    }

    fn is_supported_youtube_video_url(&self) -> bool {
        match self.host.as_str() {
            "youtube.com" | "www.youtube.com" | "music.youtube.com" => {
                self.path == "/watch" && self.has_non_empty_query_param("v")
            }
            "youtu.be" => !self.path.trim_start_matches('/').is_empty(),
            _ => false,
        }
    }

    fn has_playlist_context(&self) -> bool {
        self.path == "/playlist" || self.has_query_param("list")
    }

    fn has_query_param(&self, name: &str) -> bool {
        self.query
            .as_deref()
            .map(|query| query.split('&').any(|part| query_param_name(part) == name))
            .unwrap_or(false)
    }

    fn has_non_empty_query_param(&self, name: &str) -> bool {
        self.query
            .as_deref()
            .map(|query| {
                query.split('&').any(|part| {
                    part.split_once('=')
                        .map(|(param_name, value)| param_name == name && !value.is_empty())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    }

    fn query_param_value(&self, name: &str) -> Option<&str> {
        self.query.as_deref()?.split('&').find_map(|part| {
            part.split_once('=')
                .and_then(|(param_name, value)| (param_name == name).then_some(value))
        })
    }

    fn sanitized_youtube_video_id(&self) -> Option<String> {
        let raw = match self.host.as_str() {
            "youtube.com" | "www.youtube.com" | "music.youtube.com" => self.query_param_value("v"),
            "youtu.be" => self.path.trim_start_matches('/').split('/').next(),
            _ => None,
        }?;

        sanitize_video_id(raw)
    }

    fn without_playlist_context(&self) -> String {
        let query = self.query_without_param("list");
        let mut url = format!("{}://{}{}", self.scheme, self.authority, self.path);

        if let Some(query) = query {
            url.push('?');
            url.push_str(&query);
        }

        if let Some(fragment) = &self.fragment {
            url.push('#');
            url.push_str(fragment);
        }

        url
    }

    fn query_without_param(&self, name: &str) -> Option<String> {
        self.query
            .as_deref()
            .map(|query| {
                query
                    .split('&')
                    .filter(|part| query_param_name(part) != name)
                    .collect::<Vec<_>>()
                    .join("&")
            })
            .filter(|query| !query.is_empty())
    }
}

struct Authority {
    host: String,
}

impl FromStr for Authority {
    type Err = ();

    fn from_str(authority: &str) -> Result<Self, Self::Err> {
        if authority.contains('@') {
            return Err(());
        }

        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        if host.is_empty()
            || host
                .chars()
                .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '-'))
        {
            return Err(());
        }

        if let Some(port) = port {
            let parsed = port.parse::<u16>().map_err(|_| ())?;
            if parsed == 0 {
                return Err(());
            }
        }

        Ok(Self {
            host: host.to_ascii_lowercase(),
        })
    }
}

fn query_param_name(part: &str) -> &str {
    part.split_once('=').map(|(name, _)| name).unwrap_or(part)
}

fn sanitize_video_id(value: &str) -> Option<String> {
    let video_id: String = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .take(128)
        .collect();

    (!video_id.is_empty()).then_some(video_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_safe_host_for_rejected_url_logs() {
        assert_eq!(
            safe_host_from_url("ftp://Example.Test/audio"),
            Some("example.test".to_string())
        );
        assert_eq!(
            safe_host_from_url("https://example.test?watch=1"),
            Some("example.test".to_string())
        );
        assert_eq!(safe_host_from_url("https://user@example.test/watch"), None);
        assert_eq!(safe_host_from_url("not-a-url"), None);
    }

    #[test]
    fn extracts_sanitized_youtube_video_ids_for_logs() {
        let watch = ParsedUrl::parse("https://www.youtube.com/watch?v=ynsLjv1AyEg")
            .expect("parse watch URL");
        let short = ParsedUrl::parse("https://youtu.be/ynsLjv1AyEg").expect("parse short URL");
        let odd = ParsedUrl::parse("https://www.youtube.com/watch?v=abc<script>")
            .expect("parse odd watch URL");

        assert_eq!(
            watch.sanitized_youtube_video_id(),
            Some("ynsLjv1AyEg".to_string())
        );
        assert_eq!(
            short.sanitized_youtube_video_id(),
            Some("ynsLjv1AyEg".to_string())
        );
        assert_eq!(
            odd.sanitized_youtube_video_id(),
            Some("abcscript".to_string())
        );
    }
}
