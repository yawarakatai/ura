use std::{
    fs,
    io::Read,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, watch};

pub const PAIRING_LIFETIME: Duration = Duration::from_secs(120);
pub const PAIRING_MAX_ATTEMPTS: u8 = 5;

#[derive(Debug)]
pub struct PairingManager {
    state: Mutex<PairingState>,
    completion_tx: watch::Sender<PairingCompletion>,
}

impl PairingManager {
    pub fn new(receiver_name: String) -> Self {
        let (completion_tx, _) = watch::channel(PairingCompletion::Inactive);
        Self {
            state: Mutex::new(PairingState {
                receiver_name,
                session: None,
                completion: PairingCompletion::Inactive,
            }),
            completion_tx,
        }
    }

    pub async fn start(&self) -> Result<PairingStart> {
        self.start_with_code(generate_pairing_code()?).await
    }

    pub async fn status(&self) -> PairingStatus {
        let mut state = self.state.lock().await;
        state.expire_if_needed(&self.completion_tx);
        state.status()
    }

    pub async fn completion(&self) -> PairingCompletion {
        let mut state = self.state.lock().await;
        state.expire_if_needed(&self.completion_tx);
        state.completion.clone()
    }

    pub async fn begin_claim(&self, code: &str) -> ClaimDecision {
        if !valid_pairing_code(code) {
            return ClaimDecision::InvalidRequest;
        }

        let mut state = self.state.lock().await;
        state.expire_if_needed(&self.completion_tx);
        let Some(session) = &mut state.session else {
            return ClaimDecision::Inactive;
        };
        if session.claiming {
            return ClaimDecision::Inactive;
        }
        if session.code != code {
            session.remaining_attempts = session.remaining_attempts.saturating_sub(1);
            let remaining_attempts = session.remaining_attempts;
            if remaining_attempts == 0 {
                state.session = None;
                state.set_completion(PairingCompletion::AttemptsExhausted, &self.completion_tx);
                return ClaimDecision::AttemptsExhausted;
            }
            return ClaimDecision::WrongCode { remaining_attempts };
        }

        session.claiming = true;
        ClaimDecision::Accepted
    }

    pub async fn complete_claim(&self, device_name: String) {
        let mut state = self.state.lock().await;
        state.session = None;
        state.set_completion(
            PairingCompletion::Paired { device_name },
            &self.completion_tx,
        );
    }

    pub async fn fail_claim(&self) {
        let mut state = self.state.lock().await;
        if let Some(session) = &mut state.session {
            session.claiming = false;
        }
    }

    pub async fn cancel(&self) {
        let mut state = self.state.lock().await;
        if state.session.take().is_some() {
            state.set_completion(PairingCompletion::Cancelled, &self.completion_tx);
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<PairingCompletion> {
        self.completion_tx.subscribe()
    }

    #[cfg(test)]
    pub async fn start_for_test(&self, code: &str, lifetime: Duration) -> PairingStart {
        self.start_with_parts(code.to_string(), lifetime).await
    }

    async fn start_with_code(&self, code: String) -> Result<PairingStart> {
        Ok(self.start_with_parts(code, PAIRING_LIFETIME).await)
    }

    async fn start_with_parts(&self, code: String, lifetime: Duration) -> PairingStart {
        let now = Instant::now();
        let session = PairingSession {
            code: code.clone(),
            expires_at: now + lifetime,
            remaining_attempts: PAIRING_MAX_ATTEMPTS,
            claiming: false,
        };
        let expires_in = session.expires_in(now);
        let mut state = self.state.lock().await;
        state.session = Some(session);
        state.set_completion(PairingCompletion::Active, &self.completion_tx);
        PairingStart { code, expires_in }
    }
}

#[derive(Debug)]
struct PairingState {
    receiver_name: String,
    session: Option<PairingSession>,
    completion: PairingCompletion,
}

impl PairingState {
    fn expire_if_needed(&mut self, completion_tx: &watch::Sender<PairingCompletion>) {
        if self
            .session
            .as_ref()
            .is_some_and(|session| Instant::now() >= session.expires_at)
        {
            self.session = None;
            self.set_completion(PairingCompletion::Expired, completion_tx);
        }
    }

    fn status(&self) -> PairingStatus {
        let Some(session) = &self.session else {
            return PairingStatus::Inactive;
        };
        PairingStatus::Active {
            receiver_name: self.receiver_name.clone(),
            expires_in: session.expires_in(Instant::now()),
            remaining_attempts: session.remaining_attempts,
        }
    }

    fn set_completion(
        &mut self,
        completion: PairingCompletion,
        completion_tx: &watch::Sender<PairingCompletion>,
    ) {
        self.completion = completion.clone();
        let _ = completion_tx.send(completion);
    }
}

#[derive(Debug)]
struct PairingSession {
    code: String,
    expires_at: Instant,
    remaining_attempts: u8,
    claiming: bool,
}

impl PairingSession {
    fn expires_in(&self, now: Instant) -> u64 {
        self.expires_at.saturating_duration_since(now).as_secs()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingStart {
    pub code: String,
    pub expires_in: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairingStatus {
    Active {
        receiver_name: String,
        expires_in: u64,
        remaining_attempts: u8,
    },
    Inactive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PairingCompletion {
    Active,
    Paired { device_name: String },
    Expired,
    AttemptsExhausted,
    Cancelled,
    Inactive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimDecision {
    Accepted,
    Inactive,
    InvalidRequest,
    WrongCode { remaining_attempts: u8 },
    AttemptsExhausted,
}

pub fn valid_pairing_code(code: &str) -> bool {
    code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit())
}

pub fn generate_pairing_code() -> Result<String> {
    let mut bytes = [0_u8; 4];
    fs::File::open("/dev/urandom")
        .with_context(|| "failed to open /dev/urandom")?
        .read_exact(&mut bytes)
        .with_context(|| "failed to read random pairing code bytes")?;
    let value = u32::from_ne_bytes(bytes) % 1_000_000;
    Ok(format!("{value:06}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_code_is_six_decimal_digits() {
        for _ in 0..128 {
            let code = generate_pairing_code().expect("generate code");
            assert!(valid_pairing_code(&code), "{code}");
        }
    }

    #[tokio::test]
    async fn leading_zero_codes_are_preserved() {
        let manager = PairingManager::new("kamo".to_string());
        let started = manager
            .start_for_test("000123", Duration::from_secs(120))
            .await;

        assert_eq!(started.code, "000123");
        assert_eq!(manager.begin_claim("000123").await, ClaimDecision::Accepted);
    }

    #[tokio::test]
    async fn session_expires_after_lifetime() {
        let manager = PairingManager::new("kamo".to_string());
        manager
            .start_for_test("123456", Duration::from_millis(1))
            .await;
        tokio::time::sleep(Duration::from_millis(5)).await;

        assert_eq!(manager.status().await, PairingStatus::Inactive);
        assert_eq!(manager.completion().await, PairingCompletion::Expired);
    }

    #[tokio::test]
    async fn correct_code_succeeds_and_consumes_session() {
        let manager = PairingManager::new("kamo".to_string());
        manager
            .start_for_test("123456", Duration::from_secs(120))
            .await;

        assert_eq!(manager.begin_claim("123456").await, ClaimDecision::Accepted);
        manager.complete_claim("desuwa".to_string()).await;
        assert_eq!(manager.status().await, PairingStatus::Inactive);
        assert_eq!(manager.begin_claim("123456").await, ClaimDecision::Inactive);
    }

    #[tokio::test]
    async fn wrong_code_decrements_attempts_and_malformed_code_does_not() {
        let manager = PairingManager::new("kamo".to_string());
        manager
            .start_for_test("123456", Duration::from_secs(120))
            .await;

        assert_eq!(
            manager.begin_claim("bad").await,
            ClaimDecision::InvalidRequest
        );
        assert_eq!(
            manager.begin_claim("654321").await,
            ClaimDecision::WrongCode {
                remaining_attempts: 4
            }
        );
    }

    #[tokio::test]
    async fn fifth_wrong_attempt_closes_session() {
        let manager = PairingManager::new("kamo".to_string());
        manager
            .start_for_test("123456", Duration::from_secs(120))
            .await;

        for _ in 0..4 {
            assert!(matches!(
                manager.begin_claim("654321").await,
                ClaimDecision::WrongCode { .. }
            ));
        }
        assert_eq!(
            manager.begin_claim("654321").await,
            ClaimDecision::AttemptsExhausted
        );
        assert_eq!(manager.status().await, PairingStatus::Inactive);
    }

    #[tokio::test]
    async fn new_session_invalidates_old_session_and_cancel_closes_session() {
        let manager = PairingManager::new("kamo".to_string());
        manager
            .start_for_test("123456", Duration::from_secs(120))
            .await;
        manager
            .start_for_test("654321", Duration::from_secs(120))
            .await;

        assert_eq!(
            manager.begin_claim("123456").await,
            ClaimDecision::WrongCode {
                remaining_attempts: 4
            }
        );
        manager.cancel().await;
        assert_eq!(manager.begin_claim("654321").await, ClaimDecision::Inactive);
    }
}
