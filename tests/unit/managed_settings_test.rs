//! Tests for ManagedSettings wiring into startup.

use patina::enterprise::managed_settings::{
    get_enforced_permission_mode, is_mcp_server_allowed, is_plugin_allowed,
    load_managed_settings_from, merge_all_managed_settings, ManagedPermissions, ManagedSettings,
};

#[test]
fn test_no_settings_file_returns_none() {
    let path = std::path::Path::new("/nonexistent/managed.json");
    let result = load_managed_settings_from(path);
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[test]
fn test_load_enforces_model() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("settings.json");
    let s = ManagedSettings {
        default_model: Some("claude-3-opus".to_string()),
        ..Default::default()
    };
    std::fs::write(&path, serde_json::to_string(&s).unwrap()).unwrap();
    let loaded = load_managed_settings_from(&path).unwrap().unwrap();
    assert_eq!(loaded.default_model.as_deref(), Some("claude-3-opus"));
}

#[test]
fn test_plugin_allowlist() {
    let s = ManagedSettings {
        allowed_plugins: Some(vec!["trusted".to_string()]),
        ..Default::default()
    };
    assert!(is_plugin_allowed("trusted", &s));
    assert!(!is_plugin_allowed("unknown", &s));
}

#[test]
fn test_mcp_denylist() {
    let s = ManagedSettings {
        denied_mcp_servers: Some(vec!["evil".to_string()]),
        ..Default::default()
    };
    assert!(!is_mcp_server_allowed("evil", &s));
    assert!(is_mcp_server_allowed("good", &s));
}

#[test]
fn test_permission_mode() {
    let s = ManagedSettings {
        permissions: Some(ManagedPermissions {
            default_mode: Some("deny".to_string()),
            rules: vec![],
        }),
        ..Default::default()
    };
    assert_eq!(get_enforced_permission_mode(&s), Some("deny"));
}

#[test]
fn test_no_enforcement_when_empty() {
    let s = ManagedSettings::default();
    assert!(is_plugin_allowed("any", &s));
    assert!(is_mcp_server_allowed("any", &s));
    assert_eq!(get_enforced_permission_mode(&s), None);
}

#[test]
fn test_merge_overlays() {
    let base = ManagedSettings {
        default_model: Some("base".to_string()),
        ..Default::default()
    };
    let overlays = vec![
        ManagedSettings {
            allowed_plugins: Some(vec!["p-a".to_string()]),
            ..Default::default()
        },
        ManagedSettings {
            default_model: Some("enforced".to_string()),
            ..Default::default()
        },
    ];
    let merged = merge_all_managed_settings(base, overlays);
    assert_eq!(merged.default_model.as_deref(), Some("enforced"));
    assert_eq!(merged.allowed_plugins, Some(vec!["p-a".to_string()]));
}
