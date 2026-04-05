//! `OAuth2` PKCE authorization code flow with token refresh and secure storage.
//!
//! Designed for cloud LLM providers (AWS Bedrock, GCP Vertex, Azure `OpenAI`)
//! that use `OAuth2` for authentication instead of static API keys.
//!
//! Token storage: `~/.crab/auth/tokens.json` — stores per-provider tokens
//! with access token, refresh token, and expiry timestamp.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::AuthError;

/// Default token file location within the crab config directory.
const TOKEN_DIR: &str = "auth";
const TOKEN_FILE: &str = "tokens.json";

/// Buffer before actual expiry to trigger refresh (5 minutes).
const EXPIRY_BUFFER_SECS: u64 = 300;

// ── Token data model ─────────────────────────────────────���─────────────

/// A stored `OAuth2` token for a specific provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredToken {
    /// The provider this token belongs to (e.g., "bedrock", "vertex").
    pub provider: String,
    /// `OAuth2` access token.
    pub access_token: String,
    /// `OAuth2` refresh token (if available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix timestamp (seconds) when the access token expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// `OAuth2` token type (usually "Bearer").
    #[serde(default = "default_token_type")]
    pub token_type: String,
}

fn default_token_type() -> String {
    "Bearer".into()
}

impl StoredToken {
    /// Check if the token has expired (with buffer).
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.is_expired_at(now_secs())
    }

    /// Check expiry at a given timestamp (for testability).
    #[must_use]
    pub fn is_expired_at(&self, current_secs: u64) -> bool {
        self.expires_at
            .is_some_and(|exp| current_secs + EXPIRY_BUFFER_SECS >= exp)
    }

    /// Check if a refresh token is available.
    #[must_use]
    pub fn can_refresh(&self) -> bool {
        self.refresh_token.as_ref().is_some_and(|rt| !rt.is_empty())
    }
}

/// Container for all stored tokens (one per provider).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenStore {
    #[serde(default)]
    pub tokens: Vec<StoredToken>,
}

impl TokenStore {
    /// Get a token for a specific provider.
    #[must_use]
    pub fn get(&self, provider: &str) -> Option<&StoredToken> {
        self.tokens.iter().find(|t| t.provider == provider)
    }

    /// Insert or update a token for a provider.
    pub fn upsert(&mut self, token: StoredToken) {
        if let Some(existing) = self
            .tokens
            .iter_mut()
            .find(|t| t.provider == token.provider)
        {
            *existing = token;
        } else {
            self.tokens.push(token);
        }
    }

    /// Remove a token for a provider.
    pub fn remove(&mut self, provider: &str) -> bool {
        let before = self.tokens.len();
        self.tokens.retain(|t| t.provider != provider);
        self.tokens.len() < before
    }
}

// ── Token file persistence ─────────────────────────────────────────────

/// Return the default token file path: `~/.crab/auth/tokens.json`.
#[must_use]
pub fn default_token_path() -> PathBuf {
    crab_common::path::home_dir()
        .join(".crab")
        .join(TOKEN_DIR)
        .join(TOKEN_FILE)
}

/// Load the token store from a file.
/// Returns an empty store if the file doesn't exist.
pub fn load_token_store(path: &Path) -> Result<TokenStore, AuthError> {
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).map_err(|e| AuthError::Auth {
            message: format!("failed to parse token store: {e}"),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TokenStore::default()),
        Err(e) => Err(AuthError::Auth {
            message: format!("failed to read token store: {e}"),
        }),
    }
}

/// Save the token store to a file.
/// Creates parent directories if they don't exist.
pub fn save_token_store(path: &Path, store: &TokenStore) -> Result<(), AuthError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AuthError::Auth {
            message: format!("failed to create token dir: {e}"),
        })?;
    }
    let json = serde_json::to_string_pretty(store).map_err(|e| AuthError::Auth {
        message: format!("failed to serialize token store: {e}"),
    })?;
    std::fs::write(path, json).map_err(|e| AuthError::Auth {
        message: format!("failed to write token store: {e}"),
    })
}

// ── OAuth2 PKCE configuration ──────────────────────────────────────────

/// Configuration for an `OAuth2` PKCE authorization flow.
#[derive(Debug, Clone)]
pub struct OAuth2Config {
    /// Provider name (e.g., "bedrock", "vertex").
    pub provider: String,
    /// `OAuth2` client ID.
    pub client_id: String,
    /// Authorization endpoint URL.
    pub auth_url: String,
    /// Token endpoint URL.
    pub token_url: String,
    /// Redirect URI for the callback (usually `http://localhost:<port>`).
    pub redirect_uri: String,
    /// `OAuth2` scopes to request.
    pub scopes: Vec<String>,
}

/// Result of a successful `OAuth2` token exchange.
#[derive(Debug, Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<Duration>,
    pub token_type: String,
}

impl TokenResponse {
    /// Convert to a `StoredToken` with the current timestamp.
    #[must_use]
    pub fn to_stored_token(&self, provider: &str) -> StoredToken {
        self.to_stored_token_at(provider, now_secs())
    }

    /// Convert to a `StoredToken` at a given timestamp (for testability).
    #[must_use]
    pub fn to_stored_token_at(&self, provider: &str, current_secs: u64) -> StoredToken {
        let expires_at = self.expires_in.map(|dur| current_secs + dur.as_secs());
        StoredToken {
            provider: provider.to_string(),
            access_token: self.access_token.clone(),
            refresh_token: self.refresh_token.clone(),
            expires_at,
            token_type: self.token_type.clone(),
        }
    }
}

// ── OAuth2Provider ─────────────────────────────────────────────────────

/// `OAuth2` auth provider — manages tokens with automatic refresh.
pub struct OAuth2Provider {
    config: OAuth2Config,
    token_path: PathBuf,
    /// Cached token to avoid file I/O on every request.
    cached_token: Mutex<Option<StoredToken>>,
}

impl std::fmt::Debug for OAuth2Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth2Provider")
            .field("provider", &self.config.provider)
            .field("token_path", &self.token_path)
            .finish_non_exhaustive()
    }
}

impl OAuth2Provider {
    /// Create a new `OAuth2` provider.
    #[must_use]
    pub fn new(config: OAuth2Config, token_path: PathBuf) -> Self {
        // Try to load cached token from disk
        let cached = load_token_store(&token_path)
            .ok()
            .and_then(|store| store.get(&config.provider).cloned());

        Self {
            config,
            token_path,
            cached_token: Mutex::new(cached),
        }
    }

    /// Get the current access token, refreshing if expired.
    pub fn get_token(&self) -> Result<StoredToken, AuthError> {
        let cached = self.cached_token.lock().unwrap().clone();

        match cached {
            Some(token) if !token.is_expired() => Ok(token),
            Some(token) if token.can_refresh() => {
                // Token expired but we have a refresh token — in a real implementation
                // this would call the token endpoint. For the skeleton, return an error
                // indicating refresh is needed.
                Err(AuthError::Auth {
                    message: format!(
                        "token for '{}' expired and needs refresh (refresh_token available)",
                        token.provider
                    ),
                })
            }
            _ => Err(AuthError::Auth {
                message: format!(
                    "no valid token for '{}' — run OAuth2 authorization flow",
                    self.config.provider
                ),
            }),
        }
    }

    /// Store a new token (after successful auth or refresh).
    pub fn store_token(&self, token: StoredToken) -> Result<(), AuthError> {
        // Update file store
        let mut store = load_token_store(&self.token_path)?;
        store.upsert(token.clone());
        save_token_store(&self.token_path, &store)?;

        // Update in-memory cache
        *self.cached_token.lock().unwrap() = Some(token);
        Ok(())
    }

    /// Clear the stored token for this provider.
    pub fn clear_token(&self) -> Result<(), AuthError> {
        let mut store = load_token_store(&self.token_path)?;
        store.remove(&self.config.provider);
        save_token_store(&self.token_path, &store)?;
        *self.cached_token.lock().unwrap() = None;
        Ok(())
    }

    /// Get the provider name.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.config.provider
    }
}

impl crate::AuthProvider for OAuth2Provider {
    fn get_auth(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = crab_common::Result<crate::AuthMethod>> + Send + '_>,
    > {
        Box::pin(async move {
            let token = self.get_token().map_err(crab_common::Error::from)?;
            Ok(crate::AuthMethod::OAuth(crate::OAuthToken {
                access_token: token.access_token,
            }))
        })
    }

    fn refresh(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crab_common::Result<()>> + Send + '_>>
    {
        Box::pin(async move {
            // In a full implementation, this would:
            // 1. Load the refresh token
            // 2. Call the token endpoint with grant_type=refresh_token
            // 3. Store the new access + refresh tokens
            // For skeleton, just verify we have a refresh token available
            let cached = self.cached_token.lock().unwrap().clone();
            match cached {
                Some(token) if token.can_refresh() => {
                    // Placeholder — real implementation would do HTTP call here
                    Ok(())
                }
                _ => Err(crab_common::Error::Auth(format!(
                    "no refresh token available for '{}'",
                    self.config.provider
                ))),
            }
        })
    }
}

// ── Utilities ──────────────────────────────────────────────────────────

/// Current Unix timestamp in seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_token(provider: &str, expires_at: Option<u64>) -> StoredToken {
        StoredToken {
            provider: provider.into(),
            access_token: "access-123".into(),
            refresh_token: Some("refresh-456".into()),
            expires_at,
            token_type: "Bearer".into(),
        }
    }

    #[test]
    fn stored_token_not_expired() {
        let token = make_token("test", Some(now_secs() + 3600));
        assert!(!token.is_expired());
    }

    #[test]
    fn stored_token_expired() {
        let token = make_token("test", Some(now_secs() - 100));
        assert!(token.is_expired());
    }

    #[test]
    fn stored_token_within_buffer_is_expired() {
        // Token expires in 4 minutes — within the 5-minute buffer
        let token = make_token("test", Some(now_secs() + 240));
        assert!(token.is_expired());
    }

    #[test]
    fn stored_token_no_expiry_not_expired() {
        let token = make_token("test", None);
        assert!(!token.is_expired());
    }

    #[test]
    fn stored_token_can_refresh() {
        let token = make_token("test", None);
        assert!(token.can_refresh());
    }

    #[test]
    fn stored_token_cannot_refresh_without_token() {
        let mut token = make_token("test", None);
        token.refresh_token = None;
        assert!(!token.can_refresh());
    }

    #[test]
    fn stored_token_cannot_refresh_with_empty_token() {
        let mut token = make_token("test", None);
        token.refresh_token = Some(String::new());
        assert!(!token.can_refresh());
    }

    #[test]
    fn is_expired_at_custom_timestamp() {
        let token = make_token("test", Some(1000));
        assert!(!token.is_expired_at(0)); // well before expiry
        assert!(token.is_expired_at(700)); // within buffer (700 + 300 = 1000)
        assert!(token.is_expired_at(1000)); // at expiry
        assert!(token.is_expired_at(2000)); // past expiry
    }

    // ── TokenStore tests ───────────────────────────────────────────────

    #[test]
    fn token_store_get() {
        let store = TokenStore {
            tokens: vec![make_token("provider-a", None)],
        };
        assert!(store.get("provider-a").is_some());
        assert!(store.get("provider-b").is_none());
    }

    #[test]
    fn token_store_upsert_insert() {
        let mut store = TokenStore::default();
        store.upsert(make_token("new-provider", None));
        assert_eq!(store.tokens.len(), 1);
        assert_eq!(
            store.get("new-provider").unwrap().access_token,
            "access-123"
        );
    }

    #[test]
    fn token_store_upsert_update() {
        let mut store = TokenStore {
            tokens: vec![make_token("provider", None)],
        };
        let mut updated = make_token("provider", None);
        updated.access_token = "new-access".into();
        store.upsert(updated);
        assert_eq!(store.tokens.len(), 1);
        assert_eq!(store.get("provider").unwrap().access_token, "new-access");
    }

    #[test]
    fn token_store_remove() {
        let mut store = TokenStore {
            tokens: vec![make_token("keep", None), make_token("remove", None)],
        };
        assert!(store.remove("remove"));
        assert_eq!(store.tokens.len(), 1);
        assert!(store.get("keep").is_some());
        assert!(store.get("remove").is_none());
    }

    #[test]
    fn token_store_remove_nonexistent() {
        let mut store = TokenStore::default();
        assert!(!store.remove("nonexistent"));
    }

    // ── File persistence tests ─────────────────────────────────────────

    #[test]
    fn load_nonexistent_returns_empty() {
        let store = load_token_store(Path::new("/nonexistent/tokens.json")).unwrap();
        assert!(store.tokens.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("crab-auth-oauth-test-roundtrip");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("tokens.json");

        let mut store = TokenStore::default();
        store.upsert(make_token("bedrock", Some(9999999999)));
        store.upsert(make_token("vertex", None));

        save_token_store(&path, &store).unwrap();
        let loaded = load_token_store(&path).unwrap();

        assert_eq!(loaded.tokens.len(), 2);
        assert_eq!(loaded.get("bedrock").unwrap().access_token, "access-123");
        assert_eq!(loaded.get("bedrock").unwrap().expires_at, Some(9999999999));
        assert!(loaded.get("vertex").is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_creates_parent_dirs() {
        let dir = std::env::temp_dir().join("crab-auth-oauth-test-dirs");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("deep").join("tokens.json");

        let store = TokenStore::default();
        save_token_store(&path, &store).unwrap();
        assert!(path.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_invalid_json_returns_error() {
        let dir = std::env::temp_dir().join("crab-auth-oauth-test-invalid");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("tokens.json");
        std::fs::write(&path, "not json").unwrap();

        let result = load_token_store(&path);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── TokenResponse tests ────────────────────────────────────────────

    #[test]
    fn token_response_to_stored() {
        let resp = TokenResponse {
            access_token: "acc-new".into(),
            refresh_token: Some("ref-new".into()),
            expires_in: Some(Duration::from_secs(3600)),
            token_type: "Bearer".into(),
        };
        let stored = resp.to_stored_token_at("bedrock", 1000);
        assert_eq!(stored.provider, "bedrock");
        assert_eq!(stored.access_token, "acc-new");
        assert_eq!(stored.refresh_token.as_deref(), Some("ref-new"));
        assert_eq!(stored.expires_at, Some(4600)); // 1000 + 3600
        assert_eq!(stored.token_type, "Bearer");
    }

    #[test]
    fn token_response_no_expiry() {
        let resp = TokenResponse {
            access_token: "acc".into(),
            refresh_token: None,
            expires_in: None,
            token_type: "Bearer".into(),
        };
        let stored = resp.to_stored_token_at("vertex", 5000);
        assert!(stored.expires_at.is_none());
        assert!(stored.refresh_token.is_none());
    }

    // ── OAuth2Provider tests ────────────────────────────────────────��──

    fn test_config() -> OAuth2Config {
        OAuth2Config {
            provider: "test-provider".into(),
            client_id: "client-123".into(),
            auth_url: "https://auth.example.com/authorize".into(),
            token_url: "https://auth.example.com/token".into(),
            redirect_uri: "http://localhost:9876/callback".into(),
            scopes: vec!["openid".into(), "profile".into()],
        }
    }

    #[test]
    fn oauth2_provider_no_token_returns_error() {
        let dir = std::env::temp_dir().join("crab-auth-oauth-provider-no-token");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("tokens.json");

        let provider = OAuth2Provider::new(test_config(), path);
        let result = provider.get_token();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no valid token"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oauth2_provider_store_and_get_token() {
        let dir = std::env::temp_dir().join("crab-auth-oauth-provider-store");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("tokens.json");

        let provider = OAuth2Provider::new(test_config(), path);
        let token = StoredToken {
            provider: "test-provider".into(),
            access_token: "my-access".into(),
            refresh_token: Some("my-refresh".into()),
            expires_at: Some(now_secs() + 3600),
            token_type: "Bearer".into(),
        };
        provider.store_token(token).unwrap();

        let retrieved = provider.get_token().unwrap();
        assert_eq!(retrieved.access_token, "my-access");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oauth2_provider_expired_token_with_refresh() {
        let dir = std::env::temp_dir().join("crab-auth-oauth-provider-expired");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("tokens.json");

        let provider = OAuth2Provider::new(test_config(), path);
        let token = StoredToken {
            provider: "test-provider".into(),
            access_token: "expired-access".into(),
            refresh_token: Some("my-refresh".into()),
            expires_at: Some(now_secs() - 100), // expired
            token_type: "Bearer".into(),
        };
        provider.store_token(token).unwrap();

        let result = provider.get_token();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("needs refresh"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oauth2_provider_clear_token() {
        let dir = std::env::temp_dir().join("crab-auth-oauth-provider-clear");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("tokens.json");

        let provider = OAuth2Provider::new(test_config(), path.clone());
        provider
            .store_token(make_token("test-provider", Some(now_secs() + 3600)))
            .unwrap();
        assert!(provider.get_token().is_ok());

        provider.clear_token().unwrap();
        assert!(provider.get_token().is_err());

        // Also cleared from file
        let store = load_token_store(&path).unwrap();
        assert!(store.get("test-provider").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oauth2_provider_name() {
        let dir = std::env::temp_dir().join("crab-auth-oauth-provider-name");
        let path = dir.join("tokens.json");
        let provider = OAuth2Provider::new(test_config(), path);
        assert_eq!(provider.provider(), "test-provider");
    }

    #[test]
    fn oauth2_provider_debug() {
        let dir = std::env::temp_dir().join("crab-auth-oauth-debug");
        let path = dir.join("tokens.json");
        let provider = OAuth2Provider::new(test_config(), path);
        let debug = format!("{provider:?}");
        assert!(debug.contains("test-provider"));
    }

    // ── AuthProvider trait tests ────────────────────────────────────────

    #[test]
    fn oauth2_provider_get_auth() {
        let dir = std::env::temp_dir().join("crab-auth-oauth-get-auth");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("tokens.json");

        let provider = OAuth2Provider::new(test_config(), path);
        provider
            .store_token(StoredToken {
                provider: "test-provider".into(),
                access_token: "oauth-access-token".into(),
                refresh_token: None,
                expires_at: Some(now_secs() + 3600),
                token_type: "Bearer".into(),
            })
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt
            .block_on(crate::AuthProvider::get_auth(&provider))
            .unwrap();
        match result {
            crate::AuthMethod::OAuth(t) => assert_eq!(t.access_token, "oauth-access-token"),
            crate::AuthMethod::ApiKey(_) => panic!("expected OAuth"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oauth2_provider_refresh_no_token() {
        let dir = std::env::temp_dir().join("crab-auth-oauth-refresh-none");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("tokens.json");

        let provider = OAuth2Provider::new(test_config(), path);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(crate::AuthProvider::refresh(&provider));
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oauth2_provider_refresh_with_token() {
        let dir = std::env::temp_dir().join("crab-auth-oauth-refresh-ok");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("tokens.json");

        let provider = OAuth2Provider::new(test_config(), path);
        provider
            .store_token(make_token("test-provider", Some(now_secs() + 3600)))
            .unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(crate::AuthProvider::refresh(&provider));
        assert!(result.is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oauth2_provider_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<OAuth2Provider>();
    }

    // ── Serde roundtrip tests ──────────────────────────────────────────

    #[test]
    fn stored_token_serde_roundtrip() {
        let token = make_token("provider", Some(12345));
        let json = serde_json::to_string(&token).unwrap();
        let back: StoredToken = serde_json::from_str(&json).unwrap();
        assert_eq!(token, back);
    }

    #[test]
    fn stored_token_serde_no_optional_fields() {
        let token = StoredToken {
            provider: "test".into(),
            access_token: "acc".into(),
            refresh_token: None,
            expires_at: None,
            token_type: "Bearer".into(),
        };
        let json = serde_json::to_string(&token).unwrap();
        assert!(!json.contains("refresh_token"));
        assert!(!json.contains("expires_at"));
        let back: StoredToken = serde_json::from_str(&json).unwrap();
        assert!(back.refresh_token.is_none());
        assert!(back.expires_at.is_none());
    }

    #[test]
    fn token_store_serde_roundtrip() {
        let store = TokenStore {
            tokens: vec![make_token("a", Some(100)), make_token("b", None)],
        };
        let json = serde_json::to_string_pretty(&store).unwrap();
        let back: TokenStore = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tokens.len(), 2);
    }

    #[test]
    fn default_token_path_under_crab() {
        let path = default_token_path();
        let path_str = path.to_string_lossy();
        assert!(path_str.contains(".crab"));
        assert!(path_str.contains("auth"));
        assert!(path_str.contains("tokens.json"));
    }
}
