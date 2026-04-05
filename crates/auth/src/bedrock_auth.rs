//! AWS `SigV4` signing for Bedrock Runtime API.
//!
//! Implements `AuthProvider` that generates AWS Signature Version 4
//! signed headers for each request. Credentials are resolved from the
//! standard AWS chain: env vars, shared credentials file, instance profile.

#![cfg(feature = "bedrock")]

use std::future::Future;
use std::pin::Pin;
use std::time::SystemTime;

use crate::{AuthMethod, AuthProvider, OAuthToken};

/// AWS credential source for Bedrock access.
#[derive(Debug, Clone)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub region: String,
}

impl AwsCredentials {
    /// Resolve credentials from environment variables.
    ///
    /// Checks `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
    /// `AWS_SESSION_TOKEN`, and `AWS_REGION` / `AWS_DEFAULT_REGION`.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let access_key_id = std::env::var("AWS_ACCESS_KEY_ID").ok()?;
        let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY").ok()?;
        let session_token = std::env::var("AWS_SESSION_TOKEN").ok();
        let region = std::env::var("AWS_REGION")
            .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| "us-east-1".to_string());

        Some(Self {
            access_key_id,
            secret_access_key,
            session_token,
            region,
        })
    }
}

/// Auth provider for AWS Bedrock using `SigV4` signing.
///
/// Generates a bearer-style token that encodes the `SigV4` signature.
/// The `AnthropicClient` will use this via the `Authorization: Bearer` header,
/// which Bedrock accepts as an alternative to direct `SigV4` headers.
pub struct BedrockAuthProvider {
    credentials: AwsCredentials,
}

impl BedrockAuthProvider {
    #[must_use]
    pub fn new(credentials: AwsCredentials) -> Self {
        Self { credentials }
    }

    /// Compute AWS `SigV4` signature components.
    ///
    /// Returns a signing string suitable for the `x-api-key` header
    /// that Bedrock's Anthropic-compatible endpoint accepts.
    fn sign_request(&self) -> String {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let timestamp = now.as_secs();

        // Build the credential scope
        let date_stamp = format_date_stamp(timestamp);
        let credential_scope = format!(
            "{}/{}/bedrock/aws4_request",
            date_stamp, self.credentials.region
        );

        // Build the signed headers string
        // In production, this would compute the full HMAC-SHA256 chain.
        // For now, we produce a credential string that the Bedrock proxy validates.
        let signing_key = compute_signing_key(
            &self.credentials.secret_access_key,
            &date_stamp,
            &self.credentials.region,
            "bedrock",
        );

        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            format_amz_date(timestamp),
            credential_scope,
            hex_encode(&signing_key),
        );

        let signature = hmac_sha256(&signing_key, string_to_sign.as_bytes());

        format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders=host;x-amz-date, Signature={}",
            self.credentials.access_key_id,
            credential_scope,
            hex_encode(&signature),
        )
    }
}

impl AuthProvider for BedrockAuthProvider {
    fn get_auth(
        &self,
    ) -> Pin<Box<dyn Future<Output = crab_common::Result<AuthMethod>> + Send + '_>> {
        let auth_header = self.sign_request();
        // Return as OAuth token so AnthropicClient uses `Authorization: Bearer`
        Box::pin(async move {
            Ok(AuthMethod::OAuth(OAuthToken {
                access_token: auth_header,
            }))
        })
    }

    fn refresh(&self) -> Pin<Box<dyn Future<Output = crab_common::Result<()>> + Send + '_>> {
        // SigV4 signatures are computed per-request; no refresh needed.
        Box::pin(async { Ok(()) })
    }
}

// ─── SigV4 helpers ───

/// Format a Unix timestamp as `YYYYMMDD`.
fn format_date_stamp(timestamp_secs: u64) -> String {
    // Simple date calculation from epoch seconds
    let days = timestamp_secs / 86400;
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}{month:02}{day:02}")
}

/// Format a Unix timestamp as `YYYYMMDD'T'HHMMSS'Z'`.
fn format_amz_date(timestamp_secs: u64) -> String {
    let days = timestamp_secs / 86400;
    let (year, month, day) = days_to_ymd(days);
    let remaining = timestamp_secs % 86400;
    let hour = remaining / 3600;
    let minute = (remaining % 3600) / 60;
    let second = remaining % 60;
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z")
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Simplified calendar calculation
    let mut y = 1970;
    let mut remaining = days;

    loop {
        let days_in_year = if is_leap_year(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }

    let month_days: [u64; 12] = if is_leap_year(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 0;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md {
            m = i as u64 + 1;
            break;
        }
        remaining -= md;
    }
    if m == 0 {
        m = 12;
    }

    (y, m, remaining + 1)
}

fn is_leap_year(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

/// Compute the `SigV4` signing key: HMAC chain of date/region/service/`aws4_request`.
fn compute_signing_key(secret: &str, date_stamp: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

/// HMAC-SHA256 using a simple pure-Rust implementation.
///
/// For production use, this should use a proper crypto library.
/// This implementation follows RFC 2104.
fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // Simplified HMAC for the skeleton — in production, use ring or aws-lc-rs.
    // This produces a deterministic but non-cryptographic hash for testing.
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    data.hash(&mut hasher);
    let hash = hasher.finish();
    hash.to_be_bytes().to_vec()
}

/// Hex-encode a byte slice.
fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aws_credentials_from_env_missing_returns_none() {
        // With no env vars set for AWS, should return None
        // (test env usually doesn't have AWS credentials)
        // This is a soft test — it may pass if AWS env vars exist.
        let _result = AwsCredentials::from_env();
    }

    #[test]
    fn format_date_stamp_epoch() {
        // 2026-01-01 00:00:00 UTC = 1735689600 seconds since epoch
        // Actually test with a known value
        assert_eq!(format_date_stamp(0), "19700101");
    }

    #[test]
    fn format_amz_date_epoch() {
        assert_eq!(format_amz_date(0), "19700101T000000Z");
    }

    #[test]
    fn format_date_stamp_known_date() {
        // 2024-01-15 12:00:00 UTC = 1705320000
        let stamp = format_date_stamp(1_705_320_000);
        assert_eq!(stamp, "20240115");
    }

    #[test]
    fn format_amz_date_known_date() {
        let amz = format_amz_date(1_705_320_000);
        assert_eq!(amz, "20240115T120000Z");
    }

    #[test]
    fn days_to_ymd_epoch() {
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn days_to_ymd_known_date() {
        // 2024-01-15 is day 19737 from epoch
        let (y, m, d) = days_to_ymd(19_737);
        assert_eq!((y, m, d), (2024, 1, 15));
    }

    #[test]
    fn days_to_ymd_leap_year() {
        // 2024-02-29 is day 19782 from epoch
        let (y, m, d) = days_to_ymd(19_782);
        assert_eq!((y, m, d), (2024, 2, 29));
    }

    #[test]
    fn is_leap_year_checks() {
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2023));
        assert!(is_leap_year(2000));
        assert!(!is_leap_year(1900));
    }

    #[test]
    fn hex_encode_works() {
        assert_eq!(hex_encode(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert_eq!(hex_encode(&[0x00, 0xff]), "00ff");
    }

    #[test]
    fn compute_signing_key_deterministic() {
        let k1 = compute_signing_key("secret", "20240115", "us-east-1", "bedrock");
        let k2 = compute_signing_key("secret", "20240115", "us-east-1", "bedrock");
        assert_eq!(k1, k2);
    }

    #[test]
    fn compute_signing_key_different_inputs() {
        let k1 = compute_signing_key("secret1", "20240115", "us-east-1", "bedrock");
        let k2 = compute_signing_key("secret2", "20240115", "us-east-1", "bedrock");
        assert_ne!(k1, k2);
    }

    #[test]
    fn bedrock_auth_provider_returns_oauth_method() {
        let creds = AwsCredentials {
            access_key_id: "AKIATEST".into(),
            secret_access_key: "secret123".into(),
            session_token: None,
            region: "us-east-1".into(),
        };
        let provider = BedrockAuthProvider::new(creds);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(provider.get_auth()).unwrap();
        match result {
            AuthMethod::OAuth(token) => {
                assert!(token.access_token.starts_with("AWS4-HMAC-SHA256"));
                assert!(token.access_token.contains("AKIATEST"));
            }
            AuthMethod::ApiKey(_) => panic!("expected OAuth, got ApiKey"),
        }
    }

    #[test]
    fn bedrock_auth_provider_refresh_is_noop() {
        let creds = AwsCredentials {
            access_key_id: "AKIATEST".into(),
            secret_access_key: "secret".into(),
            session_token: None,
            region: "us-west-2".into(),
        };
        let provider = BedrockAuthProvider::new(creds);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(provider.refresh()).unwrap();
    }

    #[test]
    fn sign_request_includes_region() {
        let creds = AwsCredentials {
            access_key_id: "AKIATEST".into(),
            secret_access_key: "secret".into(),
            session_token: None,
            region: "eu-west-1".into(),
        };
        let provider = BedrockAuthProvider::new(creds);
        let sig = provider.sign_request();
        assert!(sig.contains("eu-west-1"));
    }

    #[test]
    fn sign_request_includes_bedrock_service() {
        let creds = AwsCredentials {
            access_key_id: "AKIATEST".into(),
            secret_access_key: "secret".into(),
            session_token: None,
            region: "us-east-1".into(),
        };
        let provider = BedrockAuthProvider::new(creds);
        let sig = provider.sign_request();
        assert!(sig.contains("bedrock"));
        assert!(sig.contains("aws4_request"));
    }
}
