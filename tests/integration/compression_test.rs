//! Integration tests for the compression module.
//!
//! These tests verify the full compression pipeline:
//! - CCG path (when narsil has `--graph` flag)
//! - Fallback path (core narsil tools only)
//! - Cache behavior across multiple calls
//! - Graceful degradation on failures

use patina::context::compression::{
    parse_ccg_architecture, parse_ccg_manifest, render_architecture_markdown,
    render_manifest_markdown, CcgArchitecture, CcgManifest, CompressionLevel,
    CompressionOrchestrator, ContextSource, DependencyEdge, ExportInfo, LanguageInfo, ModuleInfo,
    QualitySummary, SecuritySummary, SymbolSummary,
};
use patina::narsil::NarsilCapabilities;
use std::time::Duration;

// =============================================================================
// Test Helpers
// =============================================================================

fn create_ccg_capabilities() -> NarsilCapabilities {
    let tools = vec![
        "get_ccg_manifest".to_string(),
        "export_ccg_architecture".to_string(),
        "query_ccg".to_string(),
        "find_symbols".to_string(),
        "get_dependencies".to_string(),
    ];
    NarsilCapabilities::from_tools(&tools)
}

fn create_fallback_capabilities() -> NarsilCapabilities {
    let tools = vec![
        "find_symbols".to_string(),
        "get_dependencies".to_string(),
        "get_export_map".to_string(),
        "get_incremental_status".to_string(),
    ];
    NarsilCapabilities::from_tools(&tools)
}

fn create_sample_manifest() -> CcgManifest {
    CcgManifest {
        repo_name: "test-repo".to_string(),
        commit: "abc1234".to_string(),
        languages: vec![
            LanguageInfo {
                name: "Rust".to_string(),
                lines: 10000,
                files: 50,
            },
            LanguageInfo {
                name: "TypeScript".to_string(),
                lines: 5000,
                files: 25,
            },
        ],
        symbols: SymbolSummary {
            total: 500,
            functions: 300,
            types: 100,
            modules: 50,
        },
        security: SecuritySummary::default(),
        quality: QualitySummary {
            test_coverage: Some(85.5),
            avg_complexity: 8.2,
            doc_coverage: None,
        },
    }
}

fn create_sample_architecture() -> CcgArchitecture {
    CcgArchitecture {
        modules: vec![
            ModuleInfo {
                name: "api".to_string(),
                path: "src/api".to_string(),
                public_symbols: 30,
                private_symbols: 20,
            },
            ModuleInfo {
                name: "tui".to_string(),
                path: "src/tui".to_string(),
                public_symbols: 45,
                private_symbols: 30,
            },
        ],
        public_api: vec![
            ExportInfo {
                name: "Client".to_string(),
                kind: "struct".to_string(),
                module: "api".to_string(),
                signature: None,
            },
            ExportInfo {
                name: "render".to_string(),
                kind: "function".to_string(),
                module: "tui".to_string(),
                signature: Some("fn render(frame: &mut Frame, state: &mut AppState)".to_string()),
            },
        ],
        dependency_graph: vec![
            DependencyEdge {
                from: "tui".to_string(),
                to: "api".to_string(),
                weight: 5,
            },
            DependencyEdge {
                from: "api".to_string(),
                to: "types".to_string(),
                weight: 10,
            },
        ],
    }
}

// =============================================================================
// End-to-End CCG Path Tests
// =============================================================================

#[test]
fn test_e2e_ccg_path_manifest_rendering() {
    // Simulate the CCG path where narsil has --graph flag
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    // Verify CCG path is selected
    assert!(orchestrator.should_use_ccg());

    // Create manifest content (simulating CCG response)
    let manifest = create_sample_manifest();
    let manifest_md = render_manifest_markdown(&manifest);

    // Get context through orchestrator
    let result = orchestrator.get_context(CompressionLevel::Manifest, "hash123", || {
        manifest_md.clone()
    });

    // Verify result
    assert_eq!(result.level(), CompressionLevel::Manifest);
    assert_eq!(result.source(), ContextSource::CcgLayer0);
    assert!(result.content().contains("test-repo"));
    assert!(result.content().contains("Rust"));
}

#[test]
fn test_e2e_ccg_path_architecture_rendering() {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    let architecture = create_sample_architecture();
    let arch_md = render_architecture_markdown(&architecture);

    let result = orchestrator.get_context(CompressionLevel::Architecture, "hash123", || {
        arch_md.clone()
    });

    assert_eq!(result.level(), CompressionLevel::Architecture);
    assert_eq!(result.source(), ContextSource::CcgLayer1);
    assert!(result.content().contains("api"));
    assert!(result.content().contains("tui"));
}

#[test]
fn test_e2e_ccg_path_default_context() {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    let manifest = create_sample_manifest();
    let architecture = create_sample_architecture();

    let result = orchestrator.get_default_context(
        "hash123",
        || render_manifest_markdown(&manifest),
        || render_architecture_markdown(&architecture),
    );

    // Should contain both manifest and architecture
    assert!(result.content().contains("test-repo"));
    assert!(result.content().contains("Modules"));
    assert!(result.content().contains("---")); // Separator

    // Should be under size limit
    assert!(result.byte_len() < CompressionOrchestrator::default_context_max_bytes());
}

// =============================================================================
// End-to-End Fallback Path Tests
// =============================================================================

#[test]
fn test_e2e_fallback_path_manifest() {
    let caps = create_fallback_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    // Verify fallback path is selected
    assert!(orchestrator.should_use_fallback());
    assert!(!orchestrator.should_use_ccg());

    let result = orchestrator.get_context(CompressionLevel::Manifest, "hash123", || {
        "# Manifest\n\nFallback content".to_string()
    });

    assert_eq!(result.level(), CompressionLevel::Manifest);
    assert_eq!(result.source(), ContextSource::Constructed);
    assert!(result.content().contains("Fallback content"));
}

#[test]
fn test_e2e_fallback_path_architecture() {
    let caps = create_fallback_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    let result = orchestrator.get_context(CompressionLevel::Architecture, "hash123", || {
        "# Architecture\n\nModules: api, tui".to_string()
    });

    assert_eq!(result.level(), CompressionLevel::Architecture);
    assert_eq!(result.source(), ContextSource::Constructed);
}

#[test]
fn test_e2e_fallback_path_default_context() {
    let caps = create_fallback_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    let result = orchestrator.get_default_context(
        "hash123",
        || "# Manifest\n\nRepo info".to_string(),
        || "# Architecture\n\nModule structure".to_string(),
    );

    assert!(result.content().contains("Manifest"));
    assert!(result.content().contains("Architecture"));
}

// =============================================================================
// Cache Behavior Tests
// =============================================================================

#[test]
fn test_e2e_cache_hit_after_warmup() {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    // First call - cache miss
    let _result1 = orchestrator.get_context(CompressionLevel::Manifest, "hash123", || {
        "First call content".to_string()
    });

    assert_eq!(orchestrator.metrics().cache_misses(), 1);
    assert_eq!(orchestrator.metrics().cache_hits(), 0);

    // Second call - cache hit (content function shouldn't be called)
    let result2 = orchestrator.get_context(CompressionLevel::Manifest, "hash123", || {
        panic!("Content function should not be called on cache hit");
    });

    assert_eq!(orchestrator.metrics().cache_hits(), 1);
    assert!(result2.content().contains("First call content"));
}

#[test]
fn test_e2e_cache_invalidation_on_repo_change() {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    // Cache with first hash
    let _result1 = orchestrator.get_context(CompressionLevel::Manifest, "hash_v1", || {
        "Version 1 content".to_string()
    });

    // Try to get with different hash - should miss cache
    let mut called = false;
    let result2 = orchestrator.get_context(CompressionLevel::Manifest, "hash_v2", || {
        called = true;
        "Version 2 content".to_string()
    });

    assert!(called, "Content function should be called when hash changes");
    assert!(result2.content().contains("Version 2"));
}

#[test]
fn test_e2e_cache_different_levels() {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    // Cache manifest
    let _manifest = orchestrator.get_context(CompressionLevel::Manifest, "hash123", || {
        "Manifest".to_string()
    });

    // Cache architecture (different level)
    let _arch = orchestrator.get_context(CompressionLevel::Architecture, "hash123", || {
        "Architecture".to_string()
    });

    // Both should be cache misses (different levels)
    assert_eq!(orchestrator.metrics().cache_misses(), 2);

    // Now both should hit
    let _manifest2 = orchestrator.get_context(CompressionLevel::Manifest, "hash123", || {
        panic!("Should hit cache");
    });
    let _arch2 = orchestrator.get_context(CompressionLevel::Architecture, "hash123", || {
        panic!("Should hit cache");
    });

    assert_eq!(orchestrator.metrics().cache_hits(), 2);
}

#[test]
fn test_e2e_cache_respects_ttl() {
    // Create orchestrator with very short TTL
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::with_config(
        caps,
        "test-repo",
        Duration::from_millis(10), // 10ms TTL
        100,
    );

    // Cache something
    let _result1 = orchestrator.get_context(CompressionLevel::Manifest, "hash123", || {
        "Cached content".to_string()
    });

    // Wait for TTL to expire
    std::thread::sleep(Duration::from_millis(20));

    // Should miss cache due to expiry
    let mut called = false;
    let _result2 = orchestrator.get_context(CompressionLevel::Manifest, "hash123", || {
        called = true;
        "New content after expiry".to_string()
    });

    assert!(called, "Content function should be called after TTL expiry");
}

// =============================================================================
// Graceful Degradation Tests
// =============================================================================

#[test]
fn test_e2e_graceful_degradation_returns_minimal_context() {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    // Simulate narsil disconnection
    orchestrator.enter_degraded_mode();

    let result =
        orchestrator.get_context_graceful(CompressionLevel::Manifest, "hash123", || {
            panic!("Should not call content function in degraded mode");
        });

    assert!(result.content().contains("Degraded Mode"));
    assert!(result.content().contains("unavailable"));
}

#[test]
fn test_e2e_graceful_degradation_on_error() {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    // Simulate MCP error
    let result =
        orchestrator.get_context_graceful(CompressionLevel::Architecture, "hash123", || {
            Err("MCP connection timeout".to_string())
        });

    assert!(orchestrator.is_degraded());
    assert!(result.content().contains("Degraded Mode"));
    assert_eq!(orchestrator.metrics().degradations(), 1);
}

#[test]
fn test_e2e_graceful_degradation_recovery() {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    // Enter degraded mode
    orchestrator.enter_degraded_mode();
    assert!(orchestrator.is_degraded());

    // Exit degraded mode (simulating narsil reconnection)
    orchestrator.exit_degraded_mode();
    assert!(!orchestrator.is_degraded());

    // Should work normally now
    let result =
        orchestrator.get_context_graceful(CompressionLevel::Manifest, "hash123", || {
            Ok("# Manifest\n\nRecovered".to_string())
        });

    assert!(result.content().contains("Recovered"));
    assert!(!result.content().contains("Degraded"));
}

// =============================================================================
// Token Budget Tests
// =============================================================================

#[test]
fn test_e2e_token_budget_progressive_loading() {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    let manifest = create_sample_manifest();
    let architecture = create_sample_architecture();

    // Large budget - should include everything
    let result = orchestrator.build_context_with_budget(
        10000,
        "hash123",
        || render_manifest_markdown(&manifest),
        || render_architecture_markdown(&architecture),
        Some(|| "# Symbol Details\n\nDetailed symbols".to_string()),
    );

    assert!(result.content().contains("test-repo")); // Manifest
    assert!(result.content().contains("Modules")); // Architecture
    assert!(result.content().contains("Symbol Details")); // Symbols
}

#[test]
fn test_e2e_token_budget_respects_limit() {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    // Small budget that only fits manifest
    let result = orchestrator.build_context_with_budget(
        50,
        "hash123",
        || "# Manifest\n\nSmall".to_string(),
        || "# Architecture\n\n".repeat(50), // Large
        None::<fn() -> String>,
    );

    // Should only have manifest
    assert!(result.content().contains("Manifest"));
    assert!(!result.content().contains("Architecture"));
    assert!(result.tokens_approx() <= 50);
}

// =============================================================================
// JSON-LD Parsing Tests
// =============================================================================

#[test]
fn test_e2e_parse_manifest_json_ld() {
    let json = serde_json::json!({
        "@context": "https://narsil.dev/ccg/v1",
        "@type": "Manifest",
        "repo_name": "my-project",
        "commit": "deadbeef",
        "languages": [
            {"name": "Rust", "lines": 5000, "files": 20}
        ],
        "symbols": {
            "total": 100,
            "functions": 60,
            "types": 30,
            "modules": 10
        },
        "security": {
            "critical": 0,
            "high": 1,
            "medium": 2,
            "low": 5
        },
        "quality": {
            "test_coverage": 75.0,
            "avg_complexity": 10.5
        }
    });

    let manifest = parse_ccg_manifest(&json).expect("Should parse valid JSON-LD");

    assert_eq!(manifest.repo_name, "my-project");
    assert_eq!(manifest.commit, "deadbeef");
    assert_eq!(manifest.languages.len(), 1);
    assert_eq!(manifest.symbols.total, 100);
    assert!(manifest.security.has_findings());
}

#[test]
fn test_e2e_parse_architecture_json_ld() {
    let json = serde_json::json!({
        "@context": "https://narsil.dev/ccg/v1",
        "@type": "Architecture",
        "modules": [
            {"name": "core", "path": "src/core", "symbols": 50}
        ],
        "public_api": [
            {"name": "run", "kind": "function", "path": "src/main.rs"}
        ],
        "dependency_graph": [
            {"from": "app", "to": "core"}
        ]
    });

    let arch = parse_ccg_architecture(&json).expect("Should parse valid JSON-LD");

    assert_eq!(arch.modules.len(), 1);
    assert_eq!(arch.public_api.len(), 1);
    assert_eq!(arch.dependency_graph.len(), 1);
    assert_eq!(arch.dependency_graph[0].from, "app");
    assert_eq!(arch.dependency_graph[0].to, "core");
}

// =============================================================================
// Metrics Tests
// =============================================================================

#[test]
fn test_e2e_metrics_tracking() {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    // Initial state
    assert_eq!(orchestrator.metrics().cache_hits(), 0);
    assert_eq!(orchestrator.metrics().cache_misses(), 0);
    assert_eq!(orchestrator.metrics().ccg_calls(), 0);
    assert_eq!(orchestrator.metrics().fallback_calls(), 0);

    // Trigger operations
    let _r1 = orchestrator.get_context(CompressionLevel::Manifest, "h1", || "M".to_string());
    let _r2 = orchestrator.get_context(CompressionLevel::Manifest, "h1", || "M".to_string());

    // Verify metrics
    assert_eq!(orchestrator.metrics().cache_misses(), 1);
    assert_eq!(orchestrator.metrics().cache_hits(), 1);
    assert_eq!(orchestrator.metrics().ccg_calls(), 1); // Only on miss

    // Hit rate should be 50%
    assert!((orchestrator.metrics().hit_rate() - 0.5).abs() < 0.01);
}

#[test]
fn test_e2e_metrics_fallback_tracking() {
    let caps = create_fallback_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    let _r1 = orchestrator.get_context(CompressionLevel::Manifest, "h1", || "M".to_string());

    // Should track fallback, not CCG
    assert_eq!(orchestrator.metrics().ccg_calls(), 0);
    assert_eq!(orchestrator.metrics().fallback_calls(), 1);
}
