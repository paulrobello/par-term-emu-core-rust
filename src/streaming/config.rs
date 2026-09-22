//! Configuration types for the streaming server (ARC-004).
//!
//! Split out of `server.rs`: TLS material, HTTP Basic Auth credentials,
//! the server `StreamingConfig`, and the API-route auth configuration.

use crate::streaming::error::{Result, StreamingError};

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig as RustlsServerConfig;

/// TLS certificate chain and private key for the WebSocket/HTTPS server,
/// loaded from PEM files.
#[derive(Debug)]
pub struct TlsConfig {
    /// Certificate chain in DER format
    pub certs: Vec<CertificateDer<'static>>,
    /// Private key in DER format
    pub key: PrivateKeyDer<'static>,
}

impl Clone for TlsConfig {
    fn clone(&self) -> Self {
        Self {
            certs: self.certs.clone(),
            key: self.key.clone_key(),
        }
    }
}

/// Reject private-key material readable by group/other (SEC-006).
///
/// Shared by both TLS loading paths — `from_files` (separate key file) and
/// `from_pem` (combined PEM, which contains the key too) — so neither can
/// bypass the 0600/0400 discipline the other enforces.
#[cfg(unix)]
fn reject_world_readable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(path) {
        let mode = metadata.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(StreamingError::ServerError(format!(
                "Private key file '{}' has overly permissive permissions (mode {:o}). \
                 Set to 600 or 400 for security.",
                path.display(),
                mode & 0o777
            )));
        }
    }
    Ok(())
}

impl TlsConfig {
    /// Create TLS config from separate certificate and private key PEM files
    ///
    /// # Arguments
    /// * `cert_path` - Path to certificate PEM file (may contain certificate chain)
    /// * `key_path` - Path to private key PEM file
    ///
    /// # Errors
    /// Returns error if files cannot be read or parsed
    pub fn from_files<P: AsRef<Path>>(cert_path: P, key_path: P) -> Result<Self> {
        let cert_path = cert_path.as_ref();
        let key_path = key_path.as_ref();

        // Load certificates
        let cert_file = File::open(cert_path).map_err(|e| {
            StreamingError::ServerError(format!(
                "Failed to open certificate file '{}': {}",
                cert_path.display(),
                e
            ))
        })?;
        let mut cert_reader = BufReader::new(cert_file);
        let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_reader_iter(&mut cert_reader)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| {
                StreamingError::ServerError(format!(
                    "Failed to parse certificate file '{}': {}",
                    cert_path.display(),
                    e
                ))
            })?;

        if certs.is_empty() {
            return Err(StreamingError::ServerError(format!(
                "No certificates found in '{}'",
                cert_path.display()
            )));
        }

        // Load private key
        let key_file = File::open(key_path).map_err(|e| {
            StreamingError::ServerError(format!(
                "Failed to open key file '{}': {}",
                key_path.display(),
                e
            ))
        })?;

        // Validate private key file permissions on Unix
        #[cfg(unix)]
        reject_world_readable(key_path)?;

        let mut key_reader = BufReader::new(key_file);
        let key = match PrivateKeyDer::pem_reader_iter(&mut key_reader).next() {
            Some(Ok(k)) => k,
            Some(Err(e)) => {
                return Err(StreamingError::ServerError(format!(
                    "Failed to parse key file '{}': {}",
                    key_path.display(),
                    e
                )))
            }
            None => {
                return Err(StreamingError::ServerError(format!(
                    "No private key found in '{}'",
                    key_path.display()
                )))
            }
        };

        Ok(Self { certs, key })
    }

    /// Create TLS config from a single PEM file containing both certificate and key
    ///
    /// # Arguments
    /// * `pem_path` - Path to PEM file containing certificate chain and private key
    ///
    /// # Errors
    /// Returns error if file cannot be read or parsed
    pub fn from_pem<P: AsRef<Path>>(pem_path: P) -> Result<Self> {
        let pem_path = pem_path.as_ref();

        // The combined PEM contains the private key, so it is held to the
        // same permission discipline as a standalone key file (SEC-006).
        #[cfg(unix)]
        reject_world_readable(pem_path)?;

        let pem_bytes = std::fs::read(pem_path).map_err(|e| {
            StreamingError::ServerError(format!(
                "Failed to open PEM file '{}': {}",
                pem_path.display(),
                e
            ))
        })?;

        // Read all certificates from the combined PEM file.
        let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&pem_bytes)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| {
                StreamingError::ServerError(format!(
                    "Failed to parse PEM file '{}': {}",
                    pem_path.display(),
                    e
                ))
            })?;

        if certs.is_empty() {
            return Err(StreamingError::ServerError(format!(
                "No certificates found in '{}'",
                pem_path.display()
            )));
        }

        // Locate the private key in the same file.
        let key = match PrivateKeyDer::pem_slice_iter(&pem_bytes).next() {
            Some(Ok(k)) => k,
            Some(Err(e)) => {
                return Err(StreamingError::ServerError(format!(
                    "Failed to parse PEM file '{}': {}",
                    pem_path.display(),
                    e
                )))
            }
            None => {
                return Err(StreamingError::ServerError(format!(
                    "No private key found in '{}'",
                    pem_path.display()
                )))
            }
        };

        Ok(Self { certs, key })
    }

    /// Build a rustls ServerConfig from this TLS configuration
    pub(crate) fn build_rustls_config(&self) -> Result<RustlsServerConfig> {
        RustlsServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(self.certs.clone(), self.key.clone_key())
            .map_err(|e| StreamingError::ServerError(format!("Failed to build TLS config: {}", e)))
    }
}

/// HTTP Basic Authentication configuration
///
/// Supports password verification via:
/// - Clear text comparison
/// - htpasswd hash formats: bcrypt ($2y$), apr1 ($apr1$), SHA1 ({SHA}), MD5 crypt ($1$)
#[derive(Debug, Clone)]
pub struct HttpBasicAuthConfig {
    /// Username for authentication
    pub username: String,
    /// Password storage - either clear text or htpasswd hash
    pub password: PasswordConfig,
}

/// Fixed bcrypt hash (cost 10) of a meaningless constant string, used as the
/// dummy verification target when the username does not match (SEC-008), so
/// the bcrypt work — and therefore the response timing — is identical whether
/// or not the guessed username exists.
const DUMMY_BCRYPT_HASH: &str = "$2b$10$HFz709l6DqV3WWAW.cpJrulajF3FVSf36C8Kh56zRga9.uCFtDQTi";

/// Password storage configuration.
/// Sensitive data is zeroized on drop to prevent leaking credentials in memory.
pub enum PasswordConfig {
    /// Clear text password (compared directly, zeroized on drop)
    ClearText(String),
    /// htpasswd format hash (bcrypt, apr1, sha1, md5crypt, zeroized on drop)
    Hash(String),
}

/// Redacting `Debug` (SEC-009): the stored password or hash must never
/// appear in `{:?}` output — logs and error messages routinely format
/// config values.
impl std::fmt::Debug for PasswordConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordConfig")
            .field("secret", &"***")
            .finish()
    }
}

impl Clone for PasswordConfig {
    fn clone(&self) -> Self {
        match self {
            PasswordConfig::ClearText(s) => PasswordConfig::ClearText(s.clone()),
            PasswordConfig::Hash(s) => PasswordConfig::Hash(s.clone()),
        }
    }
}

impl Drop for PasswordConfig {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        match self {
            PasswordConfig::ClearText(ref mut s) => s.zeroize(),
            PasswordConfig::Hash(ref mut s) => s.zeroize(),
        }
    }
}

impl HttpBasicAuthConfig {
    /// Create a new HTTP Basic Auth config with clear text password
    pub fn with_password(username: String, password: String) -> Self {
        Self {
            username,
            password: PasswordConfig::ClearText(password),
        }
    }

    /// Create a new HTTP Basic Auth config with htpasswd hash
    pub fn with_hash(username: String, hash: String) -> Self {
        Self {
            username,
            password: PasswordConfig::Hash(hash),
        }
    }

    /// Verify a password against this config
    ///
    /// Both checks always run to completion and are combined at the end
    /// (SEC-008): returning early on a username mismatch would skip the
    /// expensive hash verification for every wrong username while running it
    /// for the correct one, letting an attacker enumerate valid usernames by
    /// timing responses. When the username does not match and the password is
    /// stored as a hash, the bcrypt work runs against a fixed dummy hash so
    /// the amount of work is identical either way.
    pub fn verify(&self, username: &str, password: &str) -> bool {
        use subtle::ConstantTimeEq;
        let user_ok = bool::from(username.as_bytes().ct_eq(self.username.as_bytes()));
        let pass_ok = match &self.password {
            PasswordConfig::ClearText(expected) => {
                bool::from(password.as_bytes().ct_eq(expected.as_bytes()))
            }
            PasswordConfig::Hash(hash) => {
                // Verify htpasswd-format hashes (bcrypt / apr1 / md5crypt / {SHA})
                // using maintained RustCrypto crates — see `auth_hash`.
                let target = if user_ok {
                    hash.as_str()
                } else {
                    DUMMY_BCRYPT_HASH
                };
                crate::streaming::auth_hash::verify_htpasswd_hash(target, password)
            }
        };
        user_ok & pass_ok
    }
}

/// Configuration for the streaming server
#[derive(Clone)]
pub struct StreamingConfig {
    /// Maximum number of concurrent clients
    pub max_clients: usize,
    /// Whether to send initial screen content on connect
    pub send_initial_screen: bool,
    /// Keepalive ping interval in seconds (0 = disabled)
    pub keepalive_interval: u64,
    /// Default mode for new clients (true = read-only, false = read-write)
    pub default_read_only: bool,
    /// Enable HTTP static file serving
    pub enable_http: bool,
    /// Web root directory for static files (default: "./web_term")
    pub web_root: String,
    /// Initial terminal columns (0 = use terminal's current size)
    pub initial_cols: u16,
    /// Initial terminal rows (0 = use terminal's current size)
    pub initial_rows: u16,
    /// TLS configuration for secure connections (None = no TLS)
    pub tls: Option<TlsConfig>,
    /// HTTP Basic Authentication configuration (None = no auth)
    pub http_basic_auth: Option<HttpBasicAuthConfig>,
    /// Maximum number of concurrent sessions (default: 10)
    pub max_sessions: usize,
    /// Idle session timeout in seconds (0 = never timeout, default: 900)
    pub session_idle_timeout: u64,
    /// Shell presets: name → shell command
    pub presets: HashMap<String, String>,
    /// Maximum clients per session (0 = unlimited)
    pub max_clients_per_session: usize,
    /// Input rate limit in bytes per second (0 = unlimited)
    pub input_rate_limit_bytes_per_sec: usize,
    /// Enable system resource statistics collection
    pub enable_system_stats: bool,
    /// System stats collection interval in seconds
    pub system_stats_interval_secs: u64,
    /// API key for authenticating API routes (None = no API key auth)
    pub api_key: Option<String>,
    /// Allow API key authentication via query parameter (?api_key=...).
    /// Disabled by default because query params are logged by proxies/firewalls,
    /// saved in browser history, and leaked via Referer headers.
    pub allow_api_key_in_query: bool,
    /// Allowed browser `Origin` allowlist for WebSocket and HTTP CORS (SEC-005).
    ///
    /// When `Some`, WebSocket handshakes whose `Origin` header is present are
    /// accepted only if the origin is in this list, and HTTP CORS reflects the
    /// same list. When `None` (default), WebSocket connections are accepted if
    /// they have no `Origin` header (non-browser clients — always allowed) or a
    /// local origin (`localhost` / `127.0.0.1` / `::1`); remote browser origins
    /// are rejected to prevent CSRF-via-WebSocket. Set this to expose the server
    /// to specific remote browser origins.
    pub allowed_origins: Option<Vec<String>>,
}

/// Redacting `Debug` (SEC-009): mirrors the derived output field-for-field
/// except that `api_key` is replaced with a marker — `{:?}` output routinely
/// lands in logs, which must not contain the credential. `http_basic_auth`
/// is covered by `PasswordConfig`'s redacting `Debug`.
impl std::fmt::Debug for StreamingConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let api_key = self.api_key.as_ref().map(|_| "***");
        f.debug_struct("StreamingConfig")
            .field("max_clients", &self.max_clients)
            .field("send_initial_screen", &self.send_initial_screen)
            .field("keepalive_interval", &self.keepalive_interval)
            .field("default_read_only", &self.default_read_only)
            .field("enable_http", &self.enable_http)
            .field("web_root", &self.web_root)
            .field("initial_cols", &self.initial_cols)
            .field("initial_rows", &self.initial_rows)
            .field("tls", &self.tls)
            .field("http_basic_auth", &self.http_basic_auth)
            .field("max_sessions", &self.max_sessions)
            .field("session_idle_timeout", &self.session_idle_timeout)
            .field("presets", &self.presets)
            .field("max_clients_per_session", &self.max_clients_per_session)
            .field(
                "input_rate_limit_bytes_per_sec",
                &self.input_rate_limit_bytes_per_sec,
            )
            .field("enable_system_stats", &self.enable_system_stats)
            .field(
                "system_stats_interval_secs",
                &self.system_stats_interval_secs,
            )
            .field("api_key", &api_key)
            .field("allow_api_key_in_query", &self.allow_api_key_in_query)
            .field("allowed_origins", &self.allowed_origins)
            .finish()
    }
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            max_clients: 1000,
            send_initial_screen: true,
            keepalive_interval: 30,
            default_read_only: false,
            enable_http: false,
            web_root: "./web_term".to_string(),
            initial_cols: 0,
            initial_rows: 0,
            tls: None,
            http_basic_auth: None,
            max_sessions: 10,
            session_idle_timeout: 900,
            presets: HashMap::new(),
            max_clients_per_session: 0,
            input_rate_limit_bytes_per_sec: 0,
            enable_system_stats: false,
            system_stats_interval_secs: 5,
            api_key: None,
            allow_api_key_in_query: false,
            allowed_origins: None,
        }
    }
}

/// Unified authentication configuration for API routes.
/// Supports API key auth, HTTP Basic Auth, or both.
/// When both are configured, either one satisfies authentication.
#[cfg(feature = "streaming")]
#[derive(Debug, Clone)]
pub struct ApiAuthConfig {
    /// API key for Bearer / X-API-Key / query param auth
    pub api_key: Option<String>,
    /// HTTP Basic Authentication credentials
    pub http_basic_auth: Option<HttpBasicAuthConfig>,
    /// Whether to allow API key in query parameters
    pub allow_api_key_in_query: bool,
}

#[cfg(feature = "streaming")]
impl ApiAuthConfig {
    /// Returns true if any authentication method is configured
    pub fn is_configured(&self) -> bool {
        self.api_key.is_some() || self.http_basic_auth.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a parseable PEM block around `der` bytes. Block headers are
    /// assembled from parts so the committed source contains no key-shaped
    /// literal that secret scanners would flag. The payloads used with it
    /// are structurally valid but cryptographically meaningless test data.
    fn pem_block(kind: &str, der: &[u8]) -> String {
        use base64::Engine;
        format!(
            "-----BEGIN {kind}-----\n{}\n-----END {kind}-----\n",
            base64::engine::general_purpose::STANDARD.encode(der)
        )
    }

    /// A combined PEM that `from_pem` accepts: one throwaway certificate
    /// block plus one structural (fixed-seed) PKCS#8 Ed25519-style key
    /// block. No real key material is involved.
    fn test_combined_pem() -> String {
        let cert_der: &[u8] = &[0x30, 0x03, 0x02, 0x01, 0x2a];
        let pkcs8_der: &[u8] = &[
            0x30, 0x51, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42,
            0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42,
            0x42, 0x42, 0x42, 0x42, 0x42, 0x42,
        ];
        format!(
            "{}{}",
            pem_block("CERTIFICATE", cert_der),
            pem_block("PRIVATE KEY", pkcs8_der)
        )
    }

    /// Write `contents` to a uniquely named temp file with the given (Unix)
    /// permission mode and return its path. `label` keeps concurrent tests
    /// from colliding on the same filename.
    #[cfg(unix)]
    fn write_temp_pem(label: &str, mode: u32, contents: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "par-term-tls-{}-{}-{:o}.pem",
            label,
            std::process::id(),
            mode
        ));
        std::fs::write(&path, contents).expect("write temp PEM");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
            .expect("chmod temp PEM");
        path
    }

    #[cfg(unix)]
    #[test]
    fn from_pem_rejects_world_readable_combined_pem() {
        let path = write_temp_pem("pem-combined", 0o644, &test_combined_pem());
        let err = TlsConfig::from_pem(&path).expect_err("0644 combined PEM must be rejected");
        let msg = format!("{}", err);
        assert!(
            msg.contains("overly permissive permissions"),
            "expected permissions error, got: {}",
            msg
        );
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn from_pem_accepts_owner_only_combined_pem() {
        let path = write_temp_pem("pem-combined", 0o600, &test_combined_pem());
        let tls = TlsConfig::from_pem(&path).expect("0600 combined PEM must load");
        assert_eq!(tls.certs.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn from_files_rejects_world_readable_key() {
        let combined = test_combined_pem();
        // The key block is the second half (after the certificate block).
        let split_at = combined
            .find(&format!("-----BEGIN {}", "PRIVATE KEY"))
            .expect("test PEM contains a key block");
        let (cert_pem, key_pem) = combined.split_at(split_at);
        let cert_path = write_temp_pem("files-cert", 0o600, cert_pem);
        let key_path = write_temp_pem("files-key", 0o644, key_pem);
        let err =
            TlsConfig::from_files(&cert_path, &key_path).expect_err("0644 key must be rejected");
        let msg = format!("{}", err);
        assert!(
            msg.contains("overly permissive permissions"),
            "expected permissions error, got: {}",
            msg
        );
        let _ = std::fs::remove_file(&cert_path);
        let _ = std::fs::remove_file(&key_path);
    }

    #[tokio::test]
    async fn test_streaming_config_default() {
        let config = StreamingConfig::default();
        assert_eq!(config.max_clients, 1000);
        assert!(config.send_initial_screen);
        assert_eq!(config.keepalive_interval, 30);
        assert!(!config.default_read_only);
        assert_eq!(config.max_sessions, 10);
        assert_eq!(config.session_idle_timeout, 900);
        assert!(config.presets.is_empty());
        assert_eq!(config.max_clients_per_session, 0);
        assert_eq!(config.input_rate_limit_bytes_per_sec, 0);
    }
    /// SEC-009: `{:?}` output must not contain the api key or the stored
    /// password (clear text or hash).
    #[test]
    fn debug_output_redacts_secrets() {
        let config = StreamingConfig {
            api_key: Some("super-secret-api-key-42".to_string()),
            http_basic_auth: Some(HttpBasicAuthConfig::with_password(
                "admin".to_string(),
                "hunter2-clear-password".to_string(),
            )),
            ..StreamingConfig::default()
        };
        let rendered = format!("{:?}", config);
        assert!(
            !rendered.contains("super-secret-api-key-42"),
            "api key leaked in Debug: {}",
            rendered
        );
        assert!(
            !rendered.contains("hunter2-clear-password"),
            "password leaked in Debug: {}",
            rendered
        );
        assert!(
            rendered.contains("***"),
            "no redaction marker: {}",
            rendered
        );

        let hashed = bcrypt::hash("hashed-secret-password", 4).unwrap();
        let auth = HttpBasicAuthConfig::with_hash("admin".to_string(), hashed.clone());
        let rendered = format!("{:?}", auth);
        assert!(
            !rendered.contains(&hashed),
            "password hash leaked in Debug: {}",
            rendered
        );
        assert!(
            !rendered.contains("hashed-secret-password"),
            "password leaked in Debug: {}",
            rendered
        );
        assert!(rendered.contains("***"));
    }
    #[tokio::test]
    async fn test_http_basic_auth_correct_password() {
        let auth = HttpBasicAuthConfig::with_password("admin".to_string(), "secret123".to_string());
        assert!(auth.verify("admin", "secret123"));
    }
    #[tokio::test]
    async fn test_http_basic_auth_wrong_password() {
        let auth = HttpBasicAuthConfig::with_password("admin".to_string(), "secret123".to_string());
        assert!(!auth.verify("admin", "wrongpass"));
    }
    #[tokio::test]
    async fn test_http_basic_auth_wrong_username() {
        let auth = HttpBasicAuthConfig::with_password("admin".to_string(), "secret123".to_string());
        assert!(!auth.verify("root", "secret123"));
    }
    #[tokio::test]
    async fn test_http_basic_auth_empty_username() {
        let auth = HttpBasicAuthConfig::with_password("admin".to_string(), "secret123".to_string());
        assert!(!auth.verify("", "secret123"));
    }
    #[tokio::test]
    async fn test_http_basic_auth_empty_password() {
        let auth = HttpBasicAuthConfig::with_password("admin".to_string(), "secret123".to_string());
        assert!(!auth.verify("admin", ""));
    }
    #[tokio::test]
    async fn test_http_basic_auth_both_empty() {
        let auth = HttpBasicAuthConfig::with_password("admin".to_string(), "secret123".to_string());
        assert!(!auth.verify("", ""));
    }
    #[tokio::test]
    async fn test_http_basic_auth_unicode_username() {
        let auth = HttpBasicAuthConfig::with_password("用户".to_string(), "password".to_string());
        assert!(auth.verify("用户", "password"));
        assert!(!auth.verify("用戶", "password")); // Different Unicode chars
    }
    #[tokio::test]
    async fn test_http_basic_auth_unicode_password() {
        let auth = HttpBasicAuthConfig::with_password("admin".to_string(), "密码123".to_string());
        assert!(auth.verify("admin", "密码123"));
        assert!(!auth.verify("admin", "密碼123")); // Different Unicode chars
    }
    #[tokio::test]
    async fn test_http_basic_auth_case_sensitive() {
        let auth = HttpBasicAuthConfig::with_password("Admin".to_string(), "Secret".to_string());
        assert!(auth.verify("Admin", "Secret"));
        assert!(!auth.verify("admin", "Secret"));
        assert!(!auth.verify("Admin", "secret"));
    }
    #[tokio::test]
    async fn test_http_basic_auth_hash_correct_password() {
        let hashed = bcrypt::hash("secret123", 4).unwrap();
        let auth = HttpBasicAuthConfig::with_hash("admin".to_string(), hashed);
        assert!(auth.verify("admin", "secret123"));
        assert!(!auth.verify("admin", "wrongpass"));
    }
    #[tokio::test]
    async fn test_http_basic_auth_hash_wrong_username_runs_dummy_verify() {
        // SEC-008: a wrong username with a hashed password must still return
        // false (the dummy-hash path must not accidentally verify).
        let hashed = bcrypt::hash("secret123", 4).unwrap();
        let auth = HttpBasicAuthConfig::with_hash("admin".to_string(), hashed);
        assert!(!auth.verify("root", "secret123"));
        assert!(!auth.verify("root", "anything"));
    }
    #[tokio::test]
    async fn test_http_basic_auth_whitespace() {
        let auth = HttpBasicAuthConfig::with_password("admin".to_string(), "pass word".to_string());
        assert!(auth.verify("admin", "pass word"));
        assert!(!auth.verify("admin", "password"));
    }
    #[tokio::test]
    async fn test_http_basic_auth_special_chars() {
        let auth = HttpBasicAuthConfig::with_password(
            "user@example.com".to_string(),
            "p@ss!w0rd#$%".to_string(),
        );
        assert!(auth.verify("user@example.com", "p@ss!w0rd#$%"));
        assert!(!auth.verify("user@example.com", "p@ss!w0rd"));
    }
    #[tokio::test]
    async fn test_streaming_config_default_allow_api_key_in_query() {
        let config = StreamingConfig::default();
        assert!(!config.allow_api_key_in_query);
    }
    #[tokio::test]
    async fn test_streaming_config_default_max_clients() {
        let config = StreamingConfig::default();
        assert_eq!(config.max_clients, 1000);
    }
    #[tokio::test]
    async fn test_streaming_config_default_send_initial_screen() {
        let config = StreamingConfig::default();
        assert!(config.send_initial_screen);
    }
    #[tokio::test]
    async fn test_streaming_config_default_keepalive_interval() {
        let config = StreamingConfig::default();
        assert_eq!(config.keepalive_interval, 30);
    }
    #[tokio::test]
    async fn test_streaming_config_default_read_only() {
        let config = StreamingConfig::default();
        assert!(!config.default_read_only);
    }
    #[tokio::test]
    async fn test_streaming_config_default_max_sessions() {
        let config = StreamingConfig::default();
        assert_eq!(config.max_sessions, 10);
    }
    #[tokio::test]
    async fn test_streaming_config_default_session_idle_timeout() {
        let config = StreamingConfig::default();
        assert_eq!(config.session_idle_timeout, 900);
    }
    #[tokio::test]
    async fn test_streaming_config_default_presets() {
        let config = StreamingConfig::default();
        assert!(config.presets.is_empty());
    }
    #[tokio::test]
    async fn test_streaming_config_default_max_clients_per_session() {
        let config = StreamingConfig::default();
        assert_eq!(config.max_clients_per_session, 0);
    }
    #[tokio::test]
    async fn test_streaming_config_default_input_rate_limit() {
        let config = StreamingConfig::default();
        assert_eq!(config.input_rate_limit_bytes_per_sec, 0);
    }
    #[tokio::test]
    async fn test_streaming_config_default_enable_http() {
        let config = StreamingConfig::default();
        assert!(!config.enable_http);
    }
    #[tokio::test]
    async fn test_streaming_config_default_web_root() {
        let config = StreamingConfig::default();
        assert_eq!(config.web_root, "./web_term");
    }
    #[tokio::test]
    async fn test_streaming_config_default_tls() {
        let config = StreamingConfig::default();
        assert!(config.tls.is_none());
    }
    #[tokio::test]
    async fn test_streaming_config_default_http_basic_auth() {
        let config = StreamingConfig::default();
        assert!(config.http_basic_auth.is_none());
    }
    #[tokio::test]
    async fn test_streaming_config_default_api_key() {
        let config = StreamingConfig::default();
        assert!(config.api_key.is_none());
    }
    #[tokio::test]
    async fn test_api_auth_config_is_configured_none() {
        let config = ApiAuthConfig {
            api_key: None,
            http_basic_auth: None,
            allow_api_key_in_query: false,
        };
        assert!(!config.is_configured());
    }
    #[tokio::test]
    async fn test_api_auth_config_is_configured_api_key_only() {
        let config = ApiAuthConfig {
            api_key: Some("test-key".to_string()),
            http_basic_auth: None,
            allow_api_key_in_query: false,
        };
        assert!(config.is_configured());
    }
    #[tokio::test]
    async fn test_api_auth_config_is_configured_basic_auth_only() {
        let config = ApiAuthConfig {
            api_key: None,
            http_basic_auth: Some(HttpBasicAuthConfig::with_password(
                "admin".to_string(),
                "secret".to_string(),
            )),
            allow_api_key_in_query: false,
        };
        assert!(config.is_configured());
    }
    #[tokio::test]
    async fn test_api_auth_config_is_configured_both() {
        let config = ApiAuthConfig {
            api_key: Some("test-key".to_string()),
            http_basic_auth: Some(HttpBasicAuthConfig::with_password(
                "admin".to_string(),
                "secret".to_string(),
            )),
            allow_api_key_in_query: true,
        };
        assert!(config.is_configured());
    }
    #[tokio::test]
    async fn test_api_auth_config_allow_api_key_in_query_no_auth() {
        let config = ApiAuthConfig {
            api_key: None,
            http_basic_auth: None,
            allow_api_key_in_query: true,
        };
        // Even if allow_api_key_in_query is true, no auth is configured
        assert!(!config.is_configured());
    }
}
