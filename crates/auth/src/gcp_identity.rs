//! GCP Workload Identity Federation.
//!
//! Exchanges an external identity token (OIDC/SAML) for a Google Cloud
//! access token via the Security Token Service (STS) and IAM credentials API.
//!
//! Flow:
//! 1. Read external credential from source (file, URL, or inline)
//! 2. Exchange at STS (`sts.googleapis.com/v1/token`) for a federated token
//! 3. Optionally impersonate a service account via IAM credentials API
//! 4. Cache and auto-refresh the resulting access token

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use crate::{AuthMethod, AuthProvider, OAuthToken};

/// Default token lifetime (1 hour).
const DEFAULT_TOKEN_LIFETIME_SECS: u64 = 3600;

/// Refresh 5 minutes before expiry.
const REFRESH_MARGIN_SECS: u64 = 300;

/// Configuration for Workload Identity Federation.
#[derive(Debug, Clone)]
pub struct WorkloadIdentityConfig {
    /// Audience for the STS token exchange.
    /// Format: `//iam.googleapis.com/projects/{project_number}/locations/global/workloadIdentityPools/{pool_id}/providers/{provider_id}`
    pub audience: String,

    /// Source of the external credential.
    pub credential_source: CredentialSource,

    /// Service account to impersonate (optional).
    /// If set, the federated token is exchanged for SA credentials.
    pub service_account_impersonation_url: Option<String>,

    /// OAuth scopes for the token.
    pub scopes: Vec<String>,

    /// Token lifetime in seconds.
    pub token_lifetime_secs: u64,
}

impl Default for WorkloadIdentityConfig {
    fn default() -> Self {
        Self {
            audience: String::new(),
            credential_source: CredentialSource::File {
                path: String::new(),
                format: CredentialFormat::Text,
            },
            service_account_impersonation_url: None,
            scopes: vec!["https://www.googleapis.com/auth/cloud-platform".to_string()],
            token_lifetime_secs: DEFAULT_TOKEN_LIFETIME_SECS,
        }
    }
}

/// Where to read the external identity token from.
#[derive(Debug, Clone)]
pub enum CredentialSource {
    /// Read token from a file.
    File {
        path: String,
        format: CredentialFormat,
    },
    /// Fetch token from a URL (e.g., IMDS endpoint).
    Url {
        url: String,
        headers: Vec<(String, String)>,
        format: CredentialFormat,
    },
    /// Inline token value.
    Inline(String),
}

/// Format of the credential source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialFormat {
    /// Plain text — entire content is the token.
    Text,
    /// JSON — extract token from a specific field.
    Json {
        /// JSON field path (e.g., `"access_token"`).
        field: String,
    },
}

/// Cached GCP access token.
struct CachedGcpToken {
    access_token: String,
    expires_at: Instant,
}

/// Auth provider using Workload Identity Federation.
pub struct WorkloadIdentityProvider {
    config: WorkloadIdentityConfig,
    cached: tokio::sync::Mutex<Option<CachedGcpToken>>,
}

impl WorkloadIdentityProvider {
    #[must_use]
    pub fn new(config: WorkloadIdentityConfig) -> Self {
        Self {
            config,
            cached: tokio::sync::Mutex::new(None),
        }
    }

    /// Create from a GCP credential configuration file.
    ///
    /// Reads the JSON config file pointed to by `GOOGLE_APPLICATION_CREDENTIALS`
    /// and checks if it's a workload identity federation config (type = `"external_account"`).
    pub fn from_env() -> Option<Self> {
        let cred_path = std::env::var("GOOGLE_APPLICATION_CREDENTIALS").ok()?;
        let content = std::fs::read_to_string(&cred_path).ok()?;
        let config: serde_json::Value = serde_json::from_str(&content).ok()?;

        // Must be an external_account type
        if config.get("type")?.as_str()? != "external_account" {
            return None;
        }

        let audience = config.get("audience")?.as_str()?.to_string();
        let credential_source = parse_credential_source(config.get("credential_source")?)?;

        let service_account_impersonation_url = config
            .get("service_account_impersonation_url")
            .and_then(|v| v.as_str())
            .map(String::from);

        Some(Self::new(WorkloadIdentityConfig {
            audience,
            credential_source,
            service_account_impersonation_url,
            ..Default::default()
        }))
    }

    /// Read the external credential token.
    async fn read_subject_token(&self) -> crab_common::Result<String> {
        match &self.config.credential_source {
            CredentialSource::Inline(token) => Ok(token.clone()),

            CredentialSource::File { path, format } => {
                let content = tokio::fs::read_to_string(path)
                    .await
                    .map_err(|e| crab_common::Error::Auth(format!("reading credential file {path}: {e}")))?;
                extract_token(&content, format)
            }

            CredentialSource::Url {
                url,
                headers,
                format,
            } => {
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(5))
                    .build()
                    .map_err(|e| crab_common::Error::Auth(e.to_string()))?;

                let mut req = client.get(url);
                for (k, v) in headers {
                    req = req.header(k.as_str(), v.as_str());
                }

                let resp: reqwest::Response = req
                    .send()
                    .await
                    .map_err(|e| crab_common::Error::Auth(format!("fetching credential from {url}: {e}")))?;

                if !resp.status().is_success() {
                    return Err(crab_common::Error::Auth(format!(
                        "credential URL returned {}",
                        resp.status()
                    )));
                }

                let body = resp
                    .text()
                    .await
                    .map_err(|e| crab_common::Error::Auth(format!("reading credential response: {e}")))?;

                extract_token(&body, format)
            }
        }
    }

    /// Exchange the subject token at GCP STS for a federated access token.
    async fn exchange_token(&self, subject_token: &str) -> crab_common::Result<String> {
        let client = reqwest::Client::new();

        let scopes = self.config.scopes.join(" ");
        let params = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:token-exchange"),
            ("audience", &self.config.audience),
            ("scope", &scopes),
            ("requested_token_type", "urn:ietf:params:oauth:token-type:access_token"),
            ("subject_token_type", "urn:ietf:params:oauth:token-type:jwt"),
            ("subject_token", subject_token),
        ];

        let form_body = crate::aws_iam::url_encode_params(&params);

        let resp: reqwest::Response = client
            .post("https://sts.googleapis.com/v1/token")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form_body)
            .send()
            .await
            .map_err(|e| crab_common::Error::Auth(format!("STS token exchange failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            return Err(crab_common::Error::Auth(format!(
                "GCP STS returned {status}: {err_body}"
            )));
        }

        let resp_text = resp
            .text()
            .await
            .map_err(|e| crab_common::Error::Auth(format!("reading STS response: {e}")))?;

        let parsed: serde_json::Value = serde_json::from_str(&resp_text)
            .map_err(|e| crab_common::Error::Auth(format!("parsing STS response: {e}")))?;

        parsed
            .get("access_token")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| {
                crab_common::Error::Auth("no access_token in STS response".into())
            })
    }

    /// Impersonate a service account using the federated token.
    async fn impersonate_service_account(
        &self,
        federated_token: &str,
        impersonation_url: &str,
    ) -> crab_common::Result<String> {
        let client = reqwest::Client::new();

        let body = serde_json::json!({
            "scope": self.config.scopes,
            "lifetime": format!("{}s", self.config.token_lifetime_secs),
        });

        let resp: reqwest::Response = client
            .post(impersonation_url)
            .bearer_auth(federated_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                crab_common::Error::Auth(format!("SA impersonation request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            return Err(crab_common::Error::Auth(format!(
                "SA impersonation returned {status}: {err_body}"
            )));
        }

        let resp_text = resp
            .text()
            .await
            .map_err(|e| crab_common::Error::Auth(format!("reading impersonation response: {e}")))?;

        let parsed: serde_json::Value = serde_json::from_str(&resp_text)
            .map_err(|e| crab_common::Error::Auth(format!("parsing impersonation response: {e}")))?;

        parsed
            .get("accessToken")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| {
                crab_common::Error::Auth("no accessToken in impersonation response".into())
            })
    }

    /// Full credential resolution flow.
    async fn obtain_token(&self) -> crab_common::Result<String> {
        // Step 1: Read external credential
        let subject_token = self.read_subject_token().await?;

        // Step 2: Exchange at STS
        let federated_token = self.exchange_token(&subject_token).await?;

        // Step 3: Optionally impersonate service account
        if let Some(ref url) = self.config.service_account_impersonation_url {
            self.impersonate_service_account(&federated_token, url)
                .await
        } else {
            Ok(federated_token)
        }
    }
}

impl AuthProvider for WorkloadIdentityProvider {
    fn get_auth(
        &self,
    ) -> Pin<Box<dyn Future<Output = crab_common::Result<AuthMethod>> + Send + '_>> {
        Box::pin(async move {
            // Check cache
            {
                let guard = self.cached.lock().await;
                if let Some(ref cached) = *guard
                    && cached.expires_at > Instant::now() + Duration::from_secs(REFRESH_MARGIN_SECS)
                    {
                        return Ok(AuthMethod::OAuth(OAuthToken {
                            access_token: cached.access_token.clone(),
                        }));
                    }
            }

            let token = self.obtain_token().await?;

            // Cache with configured lifetime
            {
                let mut guard = self.cached.lock().await;
                *guard = Some(CachedGcpToken {
                    access_token: token.clone(),
                    expires_at: Instant::now()
                        + Duration::from_secs(self.config.token_lifetime_secs),
                });
            }

            Ok(AuthMethod::OAuth(OAuthToken {
                access_token: token,
            }))
        })
    }

    fn refresh(&self) -> Pin<Box<dyn Future<Output = crab_common::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let mut guard = self.cached.lock().await;
            *guard = None;
            drop(guard);
            Ok(())
        })
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

/// Extract a token from content based on format.
fn extract_token(content: &str, format: &CredentialFormat) -> crab_common::Result<String> {
    match format {
        CredentialFormat::Text => Ok(content.trim().to_string()),
        CredentialFormat::Json { field } => {
            let value: serde_json::Value = serde_json::from_str(content)
                .map_err(|e| crab_common::Error::Auth(format!("parsing credential JSON: {e}")))?;
            value
                .get(field)
                .and_then(|v| v.as_str())
                .map(String::from)
                .ok_or_else(|| {
                    crab_common::Error::Auth(format!("field '{field}' not found in credential JSON"))
                })
        }
    }
}

/// Parse credential source from GCP config JSON.
fn parse_credential_source(value: &serde_json::Value) -> Option<CredentialSource> {
    if let Some(file_path) = value.get("file").and_then(|v| v.as_str()) {
        let format = parse_credential_format(value.get("format"));
        return Some(CredentialSource::File {
            path: file_path.to_string(),
            format,
        });
    }

    if let Some(url) = value.get("url").and_then(|v| v.as_str()) {
        let format = parse_credential_format(value.get("format"));
        let headers = value
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        return Some(CredentialSource::Url {
            url: url.to_string(),
            headers,
            format,
        });
    }

    None
}

/// Parse the credential format specification from config JSON.
fn parse_credential_format(format_value: Option<&serde_json::Value>) -> CredentialFormat {
    let Some(fmt) = format_value else {
        return CredentialFormat::Text;
    };

    match fmt.get("type").and_then(|v| v.as_str()) {
        Some("json") => {
            let field = fmt
                .get("subject_token_field_name")
                .and_then(|v| v.as_str())
                .unwrap_or("access_token")
                .to_string();
            CredentialFormat::Json { field }
        }
        _ => CredentialFormat::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workload_identity_config_defaults() {
        let config = WorkloadIdentityConfig::default();
        assert_eq!(config.token_lifetime_secs, 3600);
        assert_eq!(config.scopes.len(), 1);
        assert!(config.scopes[0].contains("cloud-platform"));
        assert!(config.service_account_impersonation_url.is_none());
    }

    #[test]
    fn credential_format_text_eq() {
        assert_eq!(CredentialFormat::Text, CredentialFormat::Text);
    }

    #[test]
    fn credential_format_json_eq() {
        assert_eq!(
            CredentialFormat::Json {
                field: "token".into()
            },
            CredentialFormat::Json {
                field: "token".into()
            }
        );
    }

    #[test]
    fn extract_token_text() {
        let token = extract_token("  my-token-123 \n", &CredentialFormat::Text).unwrap();
        assert_eq!(token, "my-token-123");
    }

    #[test]
    fn extract_token_json() {
        let json = r#"{"access_token": "gcp-token-456", "expires_in": 3600}"#;
        let format = CredentialFormat::Json {
            field: "access_token".into(),
        };
        let token = extract_token(json, &format).unwrap();
        assert_eq!(token, "gcp-token-456");
    }

    #[test]
    fn extract_token_json_missing_field() {
        let json = r#"{"other_field": "value"}"#;
        let format = CredentialFormat::Json {
            field: "access_token".into(),
        };
        let result = extract_token(json, &format);
        assert!(result.is_err());
    }

    #[test]
    fn extract_token_json_invalid() {
        let format = CredentialFormat::Json {
            field: "token".into(),
        };
        let result = extract_token("not json", &format);
        assert!(result.is_err());
    }

    #[test]
    fn parse_credential_source_file() {
        let value = serde_json::json!({
            "file": "/tmp/token.txt"
        });
        let source = parse_credential_source(&value).unwrap();
        match source {
            CredentialSource::File { path, format } => {
                assert_eq!(path, "/tmp/token.txt");
                assert_eq!(format, CredentialFormat::Text);
            }
            _ => panic!("expected File source"),
        }
    }

    #[test]
    fn parse_credential_source_file_json_format() {
        let value = serde_json::json!({
            "file": "/tmp/cred.json",
            "format": {
                "type": "json",
                "subject_token_field_name": "id_token"
            }
        });
        let source = parse_credential_source(&value).unwrap();
        match source {
            CredentialSource::File { path, format } => {
                assert_eq!(path, "/tmp/cred.json");
                assert_eq!(
                    format,
                    CredentialFormat::Json {
                        field: "id_token".into()
                    }
                );
            }
            _ => panic!("expected File source"),
        }
    }

    #[test]
    fn parse_credential_source_url() {
        let value = serde_json::json!({
            "url": "http://metadata/token",
            "headers": {
                "Metadata-Flavor": "Google"
            }
        });
        let source = parse_credential_source(&value).unwrap();
        match source {
            CredentialSource::Url {
                url,
                headers,
                format,
            } => {
                assert_eq!(url, "http://metadata/token");
                assert_eq!(headers.len(), 1);
                assert_eq!(headers[0].0, "Metadata-Flavor");
                assert_eq!(headers[0].1, "Google");
                assert_eq!(format, CredentialFormat::Text);
            }
            _ => panic!("expected Url source"),
        }
    }

    #[test]
    fn parse_credential_source_empty() {
        let value = serde_json::json!({});
        assert!(parse_credential_source(&value).is_none());
    }

    #[test]
    fn parse_credential_format_none() {
        assert_eq!(parse_credential_format(None), CredentialFormat::Text);
    }

    #[test]
    fn parse_credential_format_text() {
        let value = serde_json::json!({"type": "text"});
        assert_eq!(parse_credential_format(Some(&value)), CredentialFormat::Text);
    }

    #[test]
    fn parse_credential_format_json() {
        let value = serde_json::json!({
            "type": "json",
            "subject_token_field_name": "jwt_token"
        });
        let result = parse_credential_format(Some(&value));
        assert_eq!(
            result,
            CredentialFormat::Json {
                field: "jwt_token".into()
            }
        );
    }

    #[test]
    fn workload_identity_provider_refresh_clears_cache() {
        let config = WorkloadIdentityConfig {
            audience: "test-audience".into(),
            credential_source: CredentialSource::Inline("token".into()),
            ..Default::default()
        };
        let provider = WorkloadIdentityProvider::new(config);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(provider.refresh()).unwrap();
    }

    #[tokio::test]
    async fn read_subject_token_inline() {
        let config = WorkloadIdentityConfig {
            audience: "test".into(),
            credential_source: CredentialSource::Inline("inline-token-789".into()),
            ..Default::default()
        };
        let provider = WorkloadIdentityProvider::new(config);
        let token = provider.read_subject_token().await.unwrap();
        assert_eq!(token, "inline-token-789");
    }

    #[tokio::test]
    async fn read_subject_token_file() {
        let dir = std::env::temp_dir().join("crab-auth-gcp-identity-test");
        let _ = std::fs::create_dir_all(&dir);
        let token_path = dir.join("subject_token.txt");
        std::fs::write(&token_path, " file-subject-token \n").unwrap();

        let config = WorkloadIdentityConfig {
            audience: "test".into(),
            credential_source: CredentialSource::File {
                path: token_path.to_string_lossy().into_owned(),
                format: CredentialFormat::Text,
            },
            ..Default::default()
        };
        let provider = WorkloadIdentityProvider::new(config);
        let token = provider.read_subject_token().await.unwrap();
        assert_eq!(token, "file-subject-token");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn read_subject_token_file_json() {
        let dir = std::env::temp_dir().join("crab-auth-gcp-identity-json-test");
        let _ = std::fs::create_dir_all(&dir);
        let token_path = dir.join("cred.json");
        std::fs::write(
            &token_path,
            r#"{"id_token": "jwt-from-file", "expires_in": 3600}"#,
        )
        .unwrap();

        let config = WorkloadIdentityConfig {
            audience: "test".into(),
            credential_source: CredentialSource::File {
                path: token_path.to_string_lossy().into_owned(),
                format: CredentialFormat::Json {
                    field: "id_token".into(),
                },
            },
            ..Default::default()
        };
        let provider = WorkloadIdentityProvider::new(config);
        let token = provider.read_subject_token().await.unwrap();
        assert_eq!(token, "jwt-from-file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn read_subject_token_missing_file() {
        let config = WorkloadIdentityConfig {
            audience: "test".into(),
            credential_source: CredentialSource::File {
                path: "/nonexistent/token.txt".into(),
                format: CredentialFormat::Text,
            },
            ..Default::default()
        };
        let provider = WorkloadIdentityProvider::new(config);
        assert!(provider.read_subject_token().await.is_err());
    }
}
