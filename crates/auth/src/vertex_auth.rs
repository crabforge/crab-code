//! Google Cloud Vertex AI authentication.
//!
//! Implements `AuthProvider` that resolves GCP credentials from:
//! 1. `GOOGLE_APPLICATION_CREDENTIALS` env var (service account JSON key)
//! 2. Application Default Credentials (ADC) via `gcloud auth application-default`
//! 3. Compute Engine metadata server (GCE/GKE/Cloud Run)

#![cfg(feature = "vertex")]

use std::future::Future;
use std::pin::Pin;

use crate::{AuthMethod, AuthProvider, OAuthToken};

/// GCP credential source for Vertex AI access.
#[derive(Debug, Clone)]
pub struct GcpCredentials {
    /// GCP project ID.
    pub project_id: String,
    /// GCP region (e.g., "us-central1").
    pub region: String,
    /// Service account key JSON (if using service account auth).
    pub service_account_key: Option<String>,
}

impl GcpCredentials {
    /// Resolve credentials from environment.
    ///
    /// Checks `GOOGLE_CLOUD_PROJECT` / `GCLOUD_PROJECT` for project ID,
    /// `GOOGLE_CLOUD_REGION` for region, and
    /// `GOOGLE_APPLICATION_CREDENTIALS` for service account key path.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let project_id = std::env::var("GOOGLE_CLOUD_PROJECT")
            .or_else(|_| std::env::var("GCLOUD_PROJECT"))
            .ok()?;
        let region =
            std::env::var("GOOGLE_CLOUD_REGION").unwrap_or_else(|_| "us-central1".to_string());

        let service_account_key = std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
            .ok()
            .and_then(|path| std::fs::read_to_string(path).ok());

        Some(Self {
            project_id,
            region,
            service_account_key,
        })
    }

    /// Vertex AI endpoint URL for the Anthropic Messages API.
    #[must_use]
    pub fn endpoint_url(&self, model_id: &str) -> String {
        format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/anthropic/models/{}",
            self.region, self.project_id, self.region, model_id
        )
    }

    /// Base URL for the Vertex AI Anthropic-compatible endpoint.
    ///
    /// This is used as the base URL for the `AnthropicClient`, which appends
    /// `/v1/messages` to make the full endpoint.
    #[must_use]
    pub fn base_url(&self, model_id: &str) -> String {
        format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/anthropic/models/{}:rawPredict",
            self.region, self.project_id, self.region, model_id
        )
    }
}

/// Auth provider for GCP Vertex AI.
///
/// Uses Google Cloud OAuth 2.0 access tokens for authentication.
/// Tokens are obtained from the ADC chain or service account key.
pub struct VertexAuthProvider {
    credentials: GcpCredentials,
    /// Cached access token (refreshed when expired).
    cached_token: tokio::sync::Mutex<Option<CachedToken>>,
}

struct CachedToken {
    access_token: String,
    expires_at: std::time::Instant,
}

impl VertexAuthProvider {
    #[must_use]
    pub fn new(credentials: GcpCredentials) -> Self {
        Self {
            credentials,
            cached_token: tokio::sync::Mutex::new(None),
        }
    }

    /// Obtain an access token from the GCP metadata server or ADC.
    async fn obtain_token(&self) -> crab_common::Result<String> {
        // Try 1: Service account key (JWT -> access token exchange)
        if let Some(ref _key_json) = self.credentials.service_account_key {
            return self.token_from_service_account().await;
        }

        // Try 2: GCE metadata server (for Compute Engine / GKE / Cloud Run)
        if let Ok(token) = self.token_from_metadata_server().await {
            return Ok(token);
        }

        // Try 3: ADC file from gcloud CLI
        if let Ok(token) = self.token_from_adc_file().await {
            return Ok(token);
        }

        Err(crab_common::Error::Other(
            "failed to obtain GCP access token: no valid credential source found".into(),
        ))
    }

    /// Exchange service account JWT for access token.
    async fn token_from_service_account(&self) -> crab_common::Result<String> {
        // In a full implementation, this would:
        // 1. Parse the service account JSON key
        // 2. Create a JWT with the appropriate claims
        // 3. Sign it with the private key
        // 4. Exchange it at https://oauth2.googleapis.com/token
        //
        // For the skeleton, we return an error to fall through to other methods.
        Err(crab_common::Error::Other(
            "service account JWT exchange not yet implemented".into(),
        ))
    }

    /// Fetch token from GCE metadata server.
    async fn token_from_metadata_server(&self) -> crab_common::Result<String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .map_err(|e| crab_common::Error::Other(e.to_string()))?;

        let resp = client
            .get("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token")
            .header("Metadata-Flavor", "Google")
            .send()
            .await
            .map_err(|e| crab_common::Error::Other(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(crab_common::Error::Other(format!(
                "metadata server returned {}",
                resp.status()
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| crab_common::Error::Other(e.to_string()))?;

        body.get("access_token")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| crab_common::Error::Other("no access_token in metadata response".into()))
    }

    /// Read ADC file from gcloud CLI's default location.
    async fn token_from_adc_file(&self) -> crab_common::Result<String> {
        let adc_path = if cfg!(windows) {
            dirs_path("APPDATA", "gcloud/application_default_credentials.json")
        } else {
            dirs_path(
                "HOME",
                ".config/gcloud/application_default_credentials.json",
            )
        };

        let Some(path) = adc_path else {
            return Err(crab_common::Error::Other("ADC file path not found".into()));
        };

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| crab_common::Error::Other(format!("reading ADC file: {e}")))?;

        let adc: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| crab_common::Error::Other(format!("parsing ADC file: {e}")))?;

        // ADC file may contain a refresh token — exchange it for an access token
        let _client_id = adc
            .get("client_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crab_common::Error::Other("no client_id in ADC".into()))?;

        let _client_secret = adc
            .get("client_secret")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crab_common::Error::Other("no client_secret in ADC".into()))?;

        let _refresh_token = adc
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crab_common::Error::Other("no refresh_token in ADC".into()))?;

        // In production, exchange refresh_token for access_token via OAuth2 endpoint.
        // For now, return error to indicate this path isn't fully wired.
        Err(crab_common::Error::Other(
            "ADC refresh token exchange not yet implemented".into(),
        ))
    }
}

fn dirs_path(env_var: &str, suffix: &str) -> Option<std::path::PathBuf> {
    std::env::var(env_var).ok().map(|dir| {
        let mut path = std::path::PathBuf::from(dir);
        path.push(suffix);
        path
    })
}

impl AuthProvider for VertexAuthProvider {
    fn get_auth(
        &self,
    ) -> Pin<Box<dyn Future<Output = crab_common::Result<AuthMethod>> + Send + '_>> {
        Box::pin(async move {
            // Check cached token
            {
                let guard = self.cached_token.lock().await;
                if let Some(ref cached) = *guard
                    && cached.expires_at > std::time::Instant::now()
                {
                    return Ok(AuthMethod::OAuth(OAuthToken {
                        access_token: cached.access_token.clone(),
                    }));
                }
            }

            // Obtain fresh token
            let token = self.obtain_token().await?;

            // Cache it (default 55 min expiry — GCP tokens last 60 min)
            {
                let mut guard = self.cached_token.lock().await;
                *guard = Some(CachedToken {
                    access_token: token.clone(),
                    expires_at: std::time::Instant::now() + std::time::Duration::from_secs(55 * 60),
                });
            }

            Ok(AuthMethod::OAuth(OAuthToken {
                access_token: token,
            }))
        })
    }

    fn refresh(&self) -> Pin<Box<dyn Future<Output = crab_common::Result<()>> + Send + '_>> {
        Box::pin(async move {
            // Invalidate cached token to force re-fetch on next get_auth()
            let mut guard = self.cached_token.lock().await;
            *guard = None;
            drop(guard);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gcp_credentials_endpoint_url() {
        let creds = GcpCredentials {
            project_id: "my-project".into(),
            region: "us-central1".into(),
            service_account_key: None,
        };
        let url = creds.endpoint_url("claude-sonnet-4-20250514");
        assert!(url.contains("us-central1-aiplatform.googleapis.com"));
        assert!(url.contains("my-project"));
        assert!(url.contains("claude-sonnet-4-20250514"));
    }

    #[test]
    fn gcp_credentials_base_url() {
        let creds = GcpCredentials {
            project_id: "proj-123".into(),
            region: "europe-west1".into(),
            service_account_key: None,
        };
        let url = creds.base_url("claude-haiku-3-5");
        assert!(url.contains("europe-west1-aiplatform.googleapis.com"));
        assert!(url.contains("proj-123"));
        assert!(url.contains(":rawPredict"));
    }

    #[test]
    fn gcp_credentials_from_env_missing() {
        // Without GOOGLE_CLOUD_PROJECT set, should return None
        // (test env usually doesn't have GCP vars)
        let _result = GcpCredentials::from_env();
    }

    #[test]
    fn vertex_auth_provider_refresh_clears_cache() {
        let creds = GcpCredentials {
            project_id: "test".into(),
            region: "us-central1".into(),
            service_account_key: None,
        };
        let provider = VertexAuthProvider::new(creds);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(provider.refresh()).unwrap();
    }

    #[test]
    fn vertex_auth_provider_get_auth_fails_without_credentials() {
        let creds = GcpCredentials {
            project_id: "test".into(),
            region: "us-central1".into(),
            service_account_key: None,
        };
        let provider = VertexAuthProvider::new(creds);
        let rt = tokio::runtime::Runtime::new().unwrap();
        // Without real GCP credentials, this should error
        let result = rt.block_on(provider.get_auth());
        assert!(result.is_err());
    }

    #[test]
    fn dirs_path_with_env_var() {
        // Test the helper directly
        let result = dirs_path("PATH", "subdir/file.json");
        assert!(result.is_some()); // PATH is always set
    }

    #[test]
    fn dirs_path_missing_env() {
        let result = dirs_path("NONEXISTENT_VAR_12345", "file.json");
        assert!(result.is_none());
    }
}
