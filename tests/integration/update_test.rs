//! Integration tests for the auto-update system.
//!
//! Tests update checking functionality including:
//! - Checking for updates against a mock server
//! - Signature verification for release binaries
//! - Download and checksum verification
//! - Platform detection

use rct::update::{ReleaseChannel, UpdateChecker, UpdateInstaller};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// =============================================================================
// Helper functions
// =============================================================================

/// Create manifest JSON string.
fn manifest_json(version: &str, channel: &str) -> String {
    serde_json::json!({
        "version": version,
        "channel": channel,
        "platforms": {
            "darwin-aarch64": {
                "url": "https://releases.rct.dev/stable/rct-darwin-aarch64",
                "sha256": "abc123",
                "size": 1024
            },
            "darwin-x86_64": {
                "url": "https://releases.rct.dev/stable/rct-darwin-x86_64",
                "sha256": "def456",
                "size": 1024
            },
            "linux-x86_64": {
                "url": "https://releases.rct.dev/stable/rct-linux-x86_64",
                "sha256": "ghi789",
                "size": 1024
            }
        }
    })
    .to_string()
}

// =============================================================================
// 8.3.1 Update check tests
// =============================================================================

/// Test that update check detects a newer version.
#[tokio::test]
async fn test_auto_update_check() {
    let mock_server = MockServer::start().await;

    // Mock the manifest endpoint with a newer version
    Mock::given(method("GET"))
        .and(path("/stable/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest_json("2.0.0", "stable")))
        .mount(&mock_server)
        .await;

    let checker =
        UpdateChecker::new_with_base_url("1.0.0", ReleaseChannel::Stable, &mock_server.uri())
            .expect("Failed to create checker");

    let result = checker.check_for_updates().await.expect("Check failed");
    assert!(result.is_some(), "Should detect newer version");

    let manifest = result.unwrap();
    assert_eq!(manifest.version, "2.0.0");
    assert_eq!(manifest.channel, "stable");
}

/// Test that update check returns None when already up-to-date.
#[tokio::test]
async fn test_auto_update_check_already_current() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/stable/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest_json("1.0.0", "stable")))
        .mount(&mock_server)
        .await;

    let checker =
        UpdateChecker::new_with_base_url("1.0.0", ReleaseChannel::Stable, &mock_server.uri())
            .expect("Failed to create checker");

    let result = checker.check_for_updates().await.expect("Check failed");
    assert!(result.is_none(), "Should not detect update when current");
}

/// Test that update check returns None when current version is newer.
#[tokio::test]
async fn test_auto_update_check_current_is_newer() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/stable/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest_json("1.0.0", "stable")))
        .mount(&mock_server)
        .await;

    let checker =
        UpdateChecker::new_with_base_url("2.0.0", ReleaseChannel::Stable, &mock_server.uri())
            .expect("Failed to create checker");

    let result = checker.check_for_updates().await.expect("Check failed");
    assert!(
        result.is_none(),
        "Should not detect update when current is newer"
    );
}

/// Test that update check handles server errors gracefully.
#[tokio::test]
async fn test_auto_update_check_server_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/stable/manifest.json"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let checker =
        UpdateChecker::new_with_base_url("1.0.0", ReleaseChannel::Stable, &mock_server.uri())
            .expect("Failed to create checker");

    let result = checker.check_for_updates().await.expect("Check failed");
    assert!(result.is_none(), "Should return None on server error");
}

/// Test that update check handles 404 gracefully.
#[tokio::test]
async fn test_auto_update_check_not_found() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/stable/manifest.json"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server)
        .await;

    let checker =
        UpdateChecker::new_with_base_url("1.0.0", ReleaseChannel::Stable, &mock_server.uri())
            .expect("Failed to create checker");

    let result = checker.check_for_updates().await.expect("Check failed");
    assert!(result.is_none(), "Should return None on 404");
}

/// Test that different release channels use different endpoints.
#[tokio::test]
async fn test_auto_update_check_channels() {
    let mock_server = MockServer::start().await;

    // Mock stable channel
    Mock::given(method("GET"))
        .and(path("/stable/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest_json("1.0.0", "stable")))
        .mount(&mock_server)
        .await;

    // Mock latest channel
    Mock::given(method("GET"))
        .and(path("/latest/manifest.json"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(manifest_json("1.1.0-beta", "latest")),
        )
        .mount(&mock_server)
        .await;

    // Mock nightly channel
    Mock::given(method("GET"))
        .and(path("/nightly/manifest.json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(manifest_json("1.2.0-nightly.20260130", "nightly")),
        )
        .mount(&mock_server)
        .await;

    // Test stable channel
    let checker_stable =
        UpdateChecker::new_with_base_url("0.9.0", ReleaseChannel::Stable, &mock_server.uri())
            .expect("Failed to create checker");
    let result = checker_stable
        .check_for_updates()
        .await
        .expect("Check failed");
    assert!(result.is_some());
    assert_eq!(result.unwrap().channel, "stable");

    // Test latest channel
    let checker_latest =
        UpdateChecker::new_with_base_url("0.9.0", ReleaseChannel::Latest, &mock_server.uri())
            .expect("Failed to create checker");
    let result = checker_latest
        .check_for_updates()
        .await
        .expect("Check failed");
    assert!(result.is_some());
    assert_eq!(result.unwrap().channel, "latest");

    // Test nightly channel
    let checker_nightly =
        UpdateChecker::new_with_base_url("0.9.0", ReleaseChannel::Nightly, &mock_server.uri())
            .expect("Failed to create checker");
    let result = checker_nightly
        .check_for_updates()
        .await
        .expect("Check failed");
    assert!(result.is_some());
    assert_eq!(result.unwrap().channel, "nightly");
}

/// Test platform key detection.
#[test]
fn test_platform_key_detection() {
    let platform = UpdateChecker::get_platform_key();

    // Should return a valid platform key for this system
    let valid_platforms = [
        "linux-x86_64",
        "linux-aarch64",
        "darwin-x86_64",
        "darwin-aarch64",
        "windows-x86_64",
        "unknown",
    ];
    assert!(
        valid_platforms.contains(&platform),
        "Platform key '{}' should be one of the valid platforms",
        platform
    );
}

// =============================================================================
// 8.3.1 Signature verification tests
// =============================================================================

/// Test that download verifies SHA256 checksum.
#[tokio::test]
async fn test_auto_update_verify_signature() {
    let mock_server = MockServer::start().await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let install_path = temp_dir.path().join("rct");

    // Create test binary content
    let binary_content = b"#!/bin/sh\necho 'RCT v2.0.0'";

    // Calculate actual SHA256
    let mut hasher = Sha256::new();
    hasher.update(binary_content);
    let expected_hash = hex::encode(hasher.finalize());

    // Mock the download endpoint
    Mock::given(method("GET"))
        .and(path("/stable/rct-darwin-aarch64"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(binary_content.to_vec()))
        .mount(&mock_server)
        .await;

    let release = rct::update::PlatformRelease {
        url: format!("{}/stable/rct-darwin-aarch64", mock_server.uri()),
        sha256: expected_hash,
        size: binary_content.len() as u64,
    };

    let installer = UpdateInstaller::new(install_path.clone());
    let result = installer.download_and_install(&release).await;

    assert!(
        result.is_ok(),
        "Download with valid checksum should succeed"
    );
    assert!(install_path.exists(), "Binary should be installed");

    // Verify content
    let installed_content = std::fs::read(&install_path).expect("Failed to read installed file");
    assert_eq!(installed_content, binary_content, "Content should match");
}

/// Test that download fails on checksum mismatch.
#[tokio::test]
async fn test_auto_update_checksum_mismatch() {
    let mock_server = MockServer::start().await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let install_path = temp_dir.path().join("rct");

    // Create test binary content
    let binary_content = b"#!/bin/sh\necho 'RCT v2.0.0'";

    // Mock the download endpoint
    Mock::given(method("GET"))
        .and(path("/stable/rct-darwin-aarch64"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(binary_content.to_vec()))
        .mount(&mock_server)
        .await;

    // Use wrong checksum
    let release = rct::update::PlatformRelease {
        url: format!("{}/stable/rct-darwin-aarch64", mock_server.uri()),
        sha256: "wrong_checksum_that_should_not_match".to_string(),
        size: binary_content.len() as u64,
    };

    let installer = UpdateInstaller::new(install_path.clone());
    let result = installer.download_and_install(&release).await;

    assert!(
        result.is_err(),
        "Download with invalid checksum should fail"
    );
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("Checksum mismatch"),
        "Error should mention checksum mismatch: {}",
        err
    );
    assert!(
        !install_path.exists(),
        "Binary should not be installed on checksum failure"
    );
}

/// Test that download handles server errors.
#[tokio::test]
async fn test_auto_update_download_server_error() {
    let mock_server = MockServer::start().await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let install_path = temp_dir.path().join("rct");

    Mock::given(method("GET"))
        .and(path("/stable/rct-darwin-aarch64"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let release = rct::update::PlatformRelease {
        url: format!("{}/stable/rct-darwin-aarch64", mock_server.uri()),
        sha256: "abc123".to_string(),
        size: 1024,
    };

    let installer = UpdateInstaller::new(install_path.clone());
    let result = installer.download_and_install(&release).await;

    assert!(result.is_err(), "Download should fail on server error");
}

// =============================================================================
// Release channel tests
// =============================================================================

/// Test release channel string conversion.
#[test]
fn test_release_channel_as_str() {
    assert_eq!(ReleaseChannel::Stable.as_str(), "stable");
    assert_eq!(ReleaseChannel::Latest.as_str(), "latest");
    assert_eq!(ReleaseChannel::Nightly.as_str(), "nightly");
}

/// Test release channel equality.
#[test]
fn test_release_channel_equality() {
    assert_eq!(ReleaseChannel::Stable, ReleaseChannel::Stable);
    assert_ne!(ReleaseChannel::Stable, ReleaseChannel::Latest);
    assert_ne!(ReleaseChannel::Latest, ReleaseChannel::Nightly);
}
