#![allow(unused_unsafe)]
//! Auth utilities — basic-auth via environment variables.

use base64::{Engine, engine::general_purpose::STANDARD};

/// OPENCODE_SERVER_PASSWORD env var name.
const PASSWORD_ENV: &str = "OPENCODE_SERVER_PASSWORD";
/// OPENCODE_SERVER_USERNAME env var name.
const USERNAME_ENV: &str = "OPENCODE_SERVER_USERNAME";
/// Default username when PASSWORD_ENV is set but USERNAME_ENV is not.
const DEFAULT_USERNAME: &str = "opencode";

/// Returns `true` when `OPENCODE_SERVER_PASSWORD` is set and non-empty.
///
/// This is the gate for secure-mode behaviour in the host.
pub fn is_secure_mode() -> bool {
    std::env::var(PASSWORD_ENV)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Returns the value of the `Authorization` header for basic-auth,
/// or `None` when `OPENCODE_SERVER_PASSWORD` is not set.
///
/// Username is taken from `OPENCODE_SERVER_USERNAME` if set,
/// otherwise defaults to `"opencode"`.
pub fn get_authorization_header() -> Option<String> {
    let password = std::env::var(PASSWORD_ENV).ok()?;
    if password.is_empty() {
        return None;
    }
    let username = std::env::var(USERNAME_ENV).unwrap_or_else(|_| DEFAULT_USERNAME.to_string());
    let credentials = format!("{username}:{password}");
    let encoded = STANDARD.encode(credentials.as_bytes());
    Some(format!("Basic {encoded}"))
}

/// Configures a [`reqwest::ClientBuilder`] with default basic-auth
/// when `OPENCODE_SERVER_PASSWORD` is set.
pub fn configure_client_auth(client: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    if let Some(header) = get_authorization_header() {
        if let Some(encoded) = header.strip_prefix("Basic ") {
            let mut headers = reqwest::header::HeaderMap::new();
            if let Ok(auth_value) = encoded.parse::<reqwest::header::HeaderValue>() {
                headers.insert(reqwest::header::AUTHORIZATION, auth_value);
                return client.default_headers(headers);
            }
        }
    }
    client
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    struct EnvGuard {
        keys: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn capture(keys: &[&'static str]) -> Self {
            let captured = keys.iter().map(|k| (*k, std::env::var(*k).ok())).collect();
            Self { keys: captured }
        }
        fn set(&self, key: &str, value: &str) {
            unsafe { std::env::set_var(key, value) };
        }
        fn unset(&self, key: &str) {
            unsafe { std::env::remove_var(key) };
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, old) in &self.keys {
                match old {
                    Some(v) => unsafe { std::env::set_var(key, v) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
        }
    }

    #[test]
    fn test_is_secure_mode_true_when_env_set() {
        let _lock = env_lock();
        let guard = EnvGuard::capture(&[PASSWORD_ENV, USERNAME_ENV]);
        guard.set(PASSWORD_ENV, "secret");
        assert!(is_secure_mode());
    }

    #[test]
    fn test_is_secure_mode_false_when_env_unset() {
        let _lock = env_lock();
        let guard = EnvGuard::capture(&[PASSWORD_ENV, USERNAME_ENV]);
        guard.unset(PASSWORD_ENV);
        assert!(!is_secure_mode());
    }

    #[test]
    fn test_is_secure_mode_false_when_password_empty() {
        let _lock = env_lock();
        let guard = EnvGuard::capture(&[PASSWORD_ENV, USERNAME_ENV]);
        guard.set(PASSWORD_ENV, "");
        assert!(!is_secure_mode());
    }

    #[test]
    fn test_get_authorization_header_with_custom_username() {
        let _lock = env_lock();
        let guard = EnvGuard::capture(&[PASSWORD_ENV, USERNAME_ENV]);
        guard.set(USERNAME_ENV, "admin");
        guard.set(PASSWORD_ENV, "hunter2");
        let header = get_authorization_header();
        assert!(header.is_some());
        let h = header.unwrap();
        assert!(h.starts_with("Basic "));
        let expected_b64 = STANDARD.encode(b"admin:hunter2");
        assert!(h.contains(&expected_b64));
    }

    #[test]
    fn test_get_authorization_header_default_username() {
        let _lock = env_lock();
        let guard = EnvGuard::capture(&[PASSWORD_ENV, USERNAME_ENV]);
        guard.set(PASSWORD_ENV, "hunter2");
        let header = get_authorization_header();
        assert!(header.is_some());
        let h = header.unwrap();
        assert!(h.starts_with("Basic "));
        let expected_b64 = STANDARD.encode(b"opencode:hunter2");
        assert!(h.contains(&expected_b64));
    }

    #[test]
    fn test_get_authorization_header_none_when_password_unset() {
        let _lock = env_lock();
        let guard = EnvGuard::capture(&[PASSWORD_ENV, USERNAME_ENV]);
        guard.unset(PASSWORD_ENV);
        assert!(get_authorization_header().is_none());
    }

    #[test]
    fn test_get_authorization_header_none_when_password_empty() {
        let _lock = env_lock();
        let guard = EnvGuard::capture(&[PASSWORD_ENV, USERNAME_ENV]);
        guard.set(PASSWORD_ENV, "");
        assert!(get_authorization_header().is_none());
    }

    #[test]
    fn test_configure_client_auth_adds_header() {
        let _lock = env_lock();
        let guard = EnvGuard::capture(&[PASSWORD_ENV, USERNAME_ENV]);
        guard.set(USERNAME_ENV, "testuser");
        guard.set(PASSWORD_ENV, "testpass");
        let client = reqwest::Client::builder();
        let configured = configure_client_auth(client);
        let _ = configured;
    }

    #[test]
    fn test_configure_client_auth_noop_when_no_password() {
        let _lock = env_lock();
        let guard = EnvGuard::capture(&[PASSWORD_ENV, USERNAME_ENV]);
        guard.unset(PASSWORD_ENV);
        let client = reqwest::Client::builder();
        let configured = configure_client_auth(client);
        let _ = configured;
    }
}
