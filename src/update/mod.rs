//! Self-update system with release channels

use anyhow::Result;
use serde::Deserialize;
use sha2::{Sha256, Digest};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseChannel {
    Stable,
    Latest,
    Nightly,
}

impl ReleaseChannel {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReleaseChannel::Stable => "stable",
            ReleaseChannel::Latest => "latest",
            ReleaseChannel::Nightly => "nightly",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ReleaseManifest {
    pub version: String,
    pub channel: String,
    pub platforms: std::collections::HashMap<String, PlatformRelease>,
}

#[derive(Debug, Deserialize)]
pub struct PlatformRelease {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

pub struct UpdateChecker {
    current_version: semver::Version,
    channel: ReleaseChannel,
    manifest_url: String,
}

impl UpdateChecker {
    pub fn new(current_version: &str, channel: ReleaseChannel) -> Result<Self> {
        Ok(Self {
            current_version: semver::Version::parse(current_version)?,
            channel,
            manifest_url: format!(
                "https://releases.rct.dev/{}/manifest.json",
                channel.as_str()
            ),
        })
    }

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

    pub fn get_platform_key() -> &'static str {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        { "linux-x86_64" }
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        { "linux-aarch64" }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        { "darwin-x86_64" }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        { "darwin-aarch64" }
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        { "windows-x86_64" }
        #[cfg(not(any(
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64"),
            all(target_os = "macos", target_arch = "x86_64"),
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "windows", target_arch = "x86_64"),
        )))]
        { "unknown" }
    }
}

pub struct UpdateInstaller {
    install_path: PathBuf,
}

impl UpdateInstaller {
    pub fn new(install_path: PathBuf) -> Self {
        Self { install_path }
    }

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
