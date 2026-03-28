//! Secure on-disk storage for MCP OAuth tokens.
//!
//! Tokens are stored as JSON files in `<store_dir>/mcp-tokens/<server_name>.json`.
//! Server names are sanitized to prevent path traversal attacks. On Unix, files
//! are created with `0o600` permissions (owner read/write only).
//!
//! Writes are atomic: data is written to a temporary file first, then renamed
//! into place to prevent partial reads.
//!
//! # Examples
//!
//! ```no_run
//! use patina::mcp::token_storage::{McpTokenStore, McpOAuthToken};
//! use std::time::{SystemTime, Duration};
//!
//! # fn example() -> anyhow::Result<()> {
//! let store = McpTokenStore::new("/tmp/patina-tokens".into());
//! let token = McpOAuthToken {
//!     access_token: "example-access-value".to_string(),
//!     refresh_token: Some("example-refresh-value".to_string()),
//!     expires_at: SystemTime::now() + Duration::from_secs(3600),
//! };
//! store.store_token("my-server", &token)?;
//! let loaded = store.load_token("my-server")?;
//! assert!(loaded.is_some());
//! # Ok(())
//! # }
//! ```

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

/// On-disk store for MCP OAuth tokens.
///
/// Each server's token is stored in a separate JSON file under the
/// `mcp-tokens/` subdirectory of the configured store directory.
pub struct McpTokenStore {
    store_dir: PathBuf,
}

/// An OAuth token with its expiration time.
///
/// # Examples
///
/// ```
/// use patina::mcp::token_storage::McpOAuthToken;
/// use std::time::{SystemTime, Duration};
///
/// let token = McpOAuthToken {
///     access_token: "example-value".to_string(),
///     refresh_token: None,
///     expires_at: SystemTime::now() + Duration::from_secs(3600),
/// };
/// assert!(!McpOAuthToken::is_token_expired(&token));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpOAuthToken {
    /// The OAuth access token.
    pub access_token: String,
    /// Optional refresh token for obtaining new access tokens.
    pub refresh_token: Option<String>,
    /// When this token expires.
    pub expires_at: SystemTime,
}

impl McpOAuthToken {
    /// Returns `true` if the token has expired.
    ///
    /// A token is considered expired if its `expires_at` time is at or before
    /// the current system time.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::mcp::token_storage::McpOAuthToken;
    /// use std::time::{SystemTime, Duration};
    ///
    /// let expired = McpOAuthToken {
    ///     access_token: "example-expired".to_string(),
    ///     refresh_token: None,
    ///     expires_at: SystemTime::UNIX_EPOCH,
    /// };
    /// assert!(McpOAuthToken::is_token_expired(&expired));
    /// ```
    #[must_use]
    pub fn is_token_expired(token: &McpOAuthToken) -> bool {
        SystemTime::now() >= token.expires_at
    }
}

impl McpTokenStore {
    /// Creates a new token store rooted at the given directory.
    ///
    /// The directory (and `mcp-tokens/` subdirectory) will be created on
    /// first write if it does not already exist.
    ///
    /// # Examples
    ///
    /// ```
    /// use patina::mcp::token_storage::McpTokenStore;
    ///
    /// let store = McpTokenStore::new("/tmp/patina-test-tokens".into());
    /// ```
    #[must_use]
    pub fn new(store_dir: PathBuf) -> Self {
        Self { store_dir }
    }

    /// Stores a token for the named server.
    ///
    /// The write is atomic (write to temp file, then rename). On Unix,
    /// file permissions are set to `0o600`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The server name is empty or contains only invalid characters
    /// - The token directory cannot be created
    /// - The file cannot be written or renamed
    pub fn store_token(&self, server_name: &str, token: &McpOAuthToken) -> Result<()> {
        let path = self.token_path(server_name)?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create token directory: {}", parent.display())
            })?;
        }

        let json = serde_json::to_string_pretty(token).context("Failed to serialize token")?;

        // Atomic write: write to temp file, then rename
        let tmp_path = path.with_extension("tmp");
        std::fs::write(&tmp_path, json.as_bytes())
            .with_context(|| format!("Failed to write temp token file: {}", tmp_path.display()))?;

        // Set restrictive permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&tmp_path, perms)
                .with_context(|| format!("Failed to set permissions on {}", tmp_path.display()))?;
        }

        std::fs::rename(&tmp_path, &path)
            .with_context(|| format!("Failed to rename token file to {}", path.display()))?;

        Ok(())
    }

    /// Loads a previously stored token for the named server.
    ///
    /// Returns `Ok(None)` if no token file exists for this server.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The server name is empty or contains only invalid characters
    /// - The file exists but contains invalid JSON
    pub fn load_token(&self, server_name: &str) -> Result<Option<McpOAuthToken>> {
        let path = self.token_path(server_name)?;

        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read token file: {}", path.display()))?;

        let token: McpOAuthToken = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse token file: {}", path.display()))?;

        Ok(Some(token))
    }

    /// Removes the stored token for the named server.
    ///
    /// No-op if no token file exists.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The server name is empty or contains only invalid characters
    /// - The file exists but cannot be deleted
    pub fn clear_token(&self, server_name: &str) -> Result<()> {
        let path = self.token_path(server_name)?;

        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("Failed to remove token file: {}", path.display()))?;
        }

        Ok(())
    }

    /// Computes the file path for a server's token, sanitizing the server name.
    ///
    /// Only alphanumeric characters and hyphens are kept; everything else is
    /// stripped. Returns an error if the sanitized name is empty.
    fn token_path(&self, server_name: &str) -> Result<PathBuf> {
        let sanitized = sanitize_server_name(server_name);
        if sanitized.is_empty() {
            return Err(anyhow!(
                "Server name '{}' contains no valid characters (alphanumeric or hyphen)",
                server_name
            ));
        }
        Ok(self
            .store_dir
            .join("mcp-tokens")
            .join(format!("{sanitized}.json")))
    }
}

/// Sanitizes a server name to prevent path traversal.
///
/// Keeps only ASCII alphanumeric characters and hyphens. All other characters
/// (including `/`, `\`, `..`, etc.) are stripped.
///
/// # Examples
///
/// ```
/// use patina::mcp::token_storage::sanitize_server_name;
///
/// assert_eq!(sanitize_server_name("my-server"), "my-server");
/// assert_eq!(sanitize_server_name("../../etc/passwd"), "etcpasswd");
/// assert_eq!(sanitize_server_name("hello world!"), "helloworld");
/// ```
#[must_use]
pub fn sanitize_server_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn store_and_load_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = McpTokenStore::new(dir.path().to_path_buf());

        let token = McpOAuthToken {
            access_token: "test-access-123".to_string(),
            refresh_token: Some("test-refresh-456".to_string()),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        };

        store.store_token("my-server", &token).unwrap();

        let loaded = store.load_token("my-server").unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.access_token, "test-access-123");
        assert_eq!(loaded.refresh_token.as_deref(), Some("test-refresh-456"));
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = McpTokenStore::new(dir.path().to_path_buf());

        let loaded = store.load_token("no-such-server").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn clear_token_removes_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = McpTokenStore::new(dir.path().to_path_buf());

        let token = McpOAuthToken {
            access_token: "test-access-val".to_string(),
            refresh_token: None,
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        };

        store.store_token("server-a", &token).unwrap();
        assert!(store.load_token("server-a").unwrap().is_some());

        store.clear_token("server-a").unwrap();
        assert!(store.load_token("server-a").unwrap().is_none());
    }

    #[test]
    fn clear_nonexistent_is_noop() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = McpTokenStore::new(dir.path().to_path_buf());

        // Should not error
        store.clear_token("nonexistent").unwrap();
    }

    #[test]
    fn is_token_expired_with_expired_token() {
        let token = McpOAuthToken {
            access_token: "test-old-value".to_string(),
            refresh_token: None,
            expires_at: SystemTime::UNIX_EPOCH,
        };
        assert!(McpOAuthToken::is_token_expired(&token));
    }

    #[test]
    fn is_token_expired_with_valid_token() {
        let token = McpOAuthToken {
            access_token: "test-fresh-value".to_string(),
            refresh_token: None,
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        };
        assert!(!McpOAuthToken::is_token_expired(&token));
    }

    #[test]
    fn server_name_sanitization_prevents_path_traversal() {
        assert_eq!(sanitize_server_name("../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_server_name("my-server"), "my-server");
        assert_eq!(sanitize_server_name("hello world!"), "helloworld");
        assert_eq!(sanitize_server_name("a/b\\c"), "abc");
        assert_eq!(sanitize_server_name("normal123"), "normal123");
    }

    #[test]
    fn empty_sanitized_name_returns_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = McpTokenStore::new(dir.path().to_path_buf());

        let token = McpOAuthToken {
            access_token: "test-x".to_string(),
            refresh_token: None,
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        };

        // A name with only invalid characters
        let result = store.store_token("../../..", &token);
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn token_file_has_restrictive_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::TempDir::new().unwrap();
        let store = McpTokenStore::new(dir.path().to_path_buf());

        let token = McpOAuthToken {
            access_token: "test-perms-value".to_string(),
            refresh_token: None,
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        };

        store.store_token("perm-test", &token).unwrap();

        let path = dir.path().join("mcp-tokens").join("perm-test.json");
        let metadata = std::fs::metadata(&path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "Token file should have 0600 permissions");
    }
}
