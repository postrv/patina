//! Self-update system with release channels.
//!
//! This module provides functionality for automatic updates of the RCT binary:
//! - Version checking against a release manifest
//! - SHA256 checksum verification for downloaded binaries
//! - Multi-platform support (Linux, macOS, Windows)
//! - Multiple release channels (stable, latest, nightly)
//!
//! # Example
//!
//! ```ignore
//! use patina::update::{ReleaseChannel, UpdateChecker, UpdateInstaller};
//! use std::path::PathBuf;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let checker = UpdateChecker::new("1.0.0", ReleaseChannel::Stable)?;
//!
//!     if let Some(manifest) = checker.check_for_updates().await? {
//!         println!("New version available: {}", manifest.version);
//!
//!         let platform = UpdateChecker::get_platform_key();
//!         if let Some(release) = manifest.platforms.get(platform) {
//!             let installer = UpdateInstaller::new(PathBuf::from("/usr/local/bin/rct"));
//!             installer.download_and_install(release).await?;
//!         }
//!     }
//!     Ok(())
//! }
//! ```

use anyhow::Result;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Release channel for update checking.
///
/// Different channels provide different stability/freshness tradeoffs:
/// - `Stable`: Production-ready releases only
/// - `Latest`: Includes beta/RC releases
/// - `Nightly`: Daily builds with newest features (may be unstable)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseChannel {
    /// Production-ready releases only.
    Stable,
    /// Includes beta and release candidate versions.
    Latest,
    /// Daily builds with newest features (may be unstable).
    Nightly,
}

impl ReleaseChannel {
    /// Returns the string representation of the release channel.
    ///
    /// This is used to construct manifest URLs.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            ReleaseChannel::Stable => "stable",
            ReleaseChannel::Latest => "latest",
            ReleaseChannel::Nightly => "nightly",
        }
    }
}

/// Release manifest containing version and platform-specific download information.
///
/// This struct is deserialized from JSON served by the release server.
#[derive(Debug, Deserialize)]
pub struct ReleaseManifest {
    /// Version string in semver format (e.g., "2.0.0").
    pub version: String,
    /// Release channel this manifest belongs to.
    pub channel: String,
    /// Platform-specific release information keyed by platform identifier.
    pub platforms: std::collections::HashMap<String, PlatformRelease>,
}

/// Platform-specific release information.
///
/// Contains the download URL and verification data for a specific platform.
#[derive(Debug, Deserialize)]
pub struct PlatformRelease {
    /// Download URL for the release binary.
    pub url: String,
    /// SHA256 checksum of the binary for verification.
    pub sha256: String,
    /// Size of the binary in bytes.
    pub size: u64,
}

const DEFAULT_RELEASE_BASE_URL: &str = "https://releases.rct.dev";

/// Checks for available updates from the release server.
///
/// The checker compares the current version against the latest available
/// version for the configured release channel.
pub struct UpdateChecker {
    current_version: semver::Version,
    manifest_url: String,
}

impl UpdateChecker {
    /// Creates a new update checker with the default release server.
    ///
    /// # Arguments
    ///
    /// * `current_version` - The current version of the application (semver format)
    /// * `channel` - The release channel to check for updates
    ///
    /// # Errors
    ///
    /// Returns an error if the version string cannot be parsed as semver.
    ///
    /// # Example
    ///
    /// ```
    /// use patina::update::{ReleaseChannel, UpdateChecker};
    ///
    /// let checker = UpdateChecker::new("1.0.0", ReleaseChannel::Stable)
    ///     .expect("Valid version");
    /// ```
    pub fn new(current_version: &str, channel: ReleaseChannel) -> Result<Self> {
        Self::new_with_base_url(current_version, channel, DEFAULT_RELEASE_BASE_URL)
    }

    /// Creates a new update checker with a custom base URL.
    ///
    /// This is primarily useful for testing against a mock server.
    ///
    /// # Arguments
    ///
    /// * `current_version` - The current version of the application (semver format)
    /// * `channel` - The release channel to check for updates
    /// * `base_url` - Custom base URL for the release server
    ///
    /// # Errors
    ///
    /// Returns an error if the version string cannot be parsed as semver.
    pub fn new_with_base_url(
        current_version: &str,
        channel: ReleaseChannel,
        base_url: &str,
    ) -> Result<Self> {
        Ok(Self {
            current_version: semver::Version::parse(current_version)?,
            manifest_url: format!("{}/{}/manifest.json", base_url, channel.as_str()),
        })
    }

    /// Checks for available updates.
    ///
    /// Fetches the release manifest from the configured server and compares
    /// the latest version against the current version.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(manifest))` - A newer version is available
    /// * `Ok(None)` - Current version is up to date, or server returned an error
    ///
    /// # Errors
    ///
    /// Returns an error if the network request fails or the manifest cannot be parsed.
    pub async fn check_for_updates(&self) -> Result<Option<ReleaseManifest>> {
        let client = reqwest::Client::new();
        let response = client.get(&self.manifest_url).send().await?;

        if !response.status().is_success() {
            return Ok(None);
        }

        let manifest: ReleaseManifest = response.json().await?;
        let latest_version = semver::Version::parse(&manifest.version)?;

        if latest_version > self.current_version {
            Ok(Some(manifest))
        } else {
            Ok(None)
        }
    }

    /// Returns the platform key for the current system.
    ///
    /// The platform key is used to look up the correct binary in the release manifest.
    ///
    /// # Returns
    ///
    /// A static string like `"darwin-aarch64"` or `"linux-x86_64"`.
    /// Returns `"unknown"` for unsupported platforms.
    #[must_use]
    pub fn get_platform_key() -> &'static str {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            "linux-x86_64"
        }
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        {
            "linux-aarch64"
        }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            "darwin-x86_64"
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            "darwin-aarch64"
        }
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            "windows-x86_64"
        }
        #[cfg(not(any(
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64"),
            all(target_os = "macos", target_arch = "x86_64"),
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "windows", target_arch = "x86_64"),
        )))]
        {
            "unknown"
        }
    }
}

/// Downloads and installs updates with checksum verification.
///
/// The installer downloads the binary to a temporary file, verifies the
/// SHA256 checksum, then atomically replaces the current binary.
pub struct UpdateInstaller {
    install_path: PathBuf,
}

impl UpdateInstaller {
    /// Creates a new update installer.
    ///
    /// # Arguments
    ///
    /// * `install_path` - Path where the updated binary should be installed
    #[must_use]
    pub fn new(install_path: PathBuf) -> Self {
        Self { install_path }
    }

    /// Downloads and installs the update.
    ///
    /// This method:
    /// 1. Downloads the binary from the release URL
    /// 2. Verifies the SHA256 checksum
    /// 3. Writes to a temporary file
    /// 4. Sets executable permissions (Unix only)
    /// 5. Atomically replaces the current binary
    ///
    /// # Arguments
    ///
    /// * `release` - Platform release information with download URL and checksum
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * The download fails
    /// * The checksum doesn't match
    /// * File operations fail
    pub async fn download_and_install(&self, release: &PlatformRelease) -> Result<()> {
        let client = reqwest::Client::new();
        let response = client.get(&release.url).send().await?;
        let bytes = response.bytes().await?;

        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hash = hex::encode(hasher.finalize());

        if hash != release.sha256 {
            anyhow::bail!(
                "Checksum mismatch: expected {}, got {}",
                release.sha256,
                hash
            );
        }

        let temp_path = self.install_path.with_extension("new");
        tokio::fs::write(&temp_path, &bytes).await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o755)).await?;
        }

        tokio::fs::rename(&temp_path, &self.install_path).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_release_channel_as_str() {
        assert_eq!(ReleaseChannel::Stable.as_str(), "stable");
        assert_eq!(ReleaseChannel::Latest.as_str(), "latest");
        assert_eq!(ReleaseChannel::Nightly.as_str(), "nightly");
    }

    #[test]
    fn test_update_checker_new() {
        let checker = UpdateChecker::new("1.0.0", ReleaseChannel::Stable);
        assert!(checker.is_ok());
    }

    #[test]
    fn test_update_checker_new_invalid_version() {
        let checker = UpdateChecker::new("not-a-version", ReleaseChannel::Stable);
        assert!(checker.is_err());
    }

    #[test]
    fn test_update_checker_new_with_base_url() {
        let checker = UpdateChecker::new_with_base_url(
            "1.0.0",
            ReleaseChannel::Stable,
            "https://example.com",
        );
        assert!(checker.is_ok());
    }

    #[test]
    fn test_platform_key_valid() {
        let platform = UpdateChecker::get_platform_key();
        let valid_platforms = [
            "linux-x86_64",
            "linux-aarch64",
            "darwin-x86_64",
            "darwin-aarch64",
            "windows-x86_64",
            "unknown",
        ];
        assert!(valid_platforms.contains(&platform));
    }
}
