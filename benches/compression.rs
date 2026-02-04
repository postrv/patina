//! Compression performance benchmarks
//!
//! These benchmarks measure the performance of context compression operations:
//! - Default context generation (manifest + architecture)
//! - Cache lookup performance
//! - Markdown rendering
//!
//! Performance targets:
//! - `default_context_generation`: <100ms p99
//! - `cache_lookup`: <1ms
//! - `manifest_rendering`: <10ms
//! - `architecture_rendering`: <10ms

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use patina::context::compression::{
    render_architecture_markdown, render_manifest_markdown, CcgArchitecture, CcgManifest,
    CompressionLevel, CompressionOrchestrator, DependencyEdge, ExportInfo, LanguageInfo,
    ModuleInfo, QualitySummary, SecuritySummary, SymbolSummary,
};
use patina::narsil::NarsilCapabilities;

// =============================================================================
// Test Data Generators
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

fn create_sample_manifest() -> CcgManifest {
    CcgManifest {
        repo_name: "patina".to_string(),
        commit: "abc123def456".to_string(),
        languages: vec![
            LanguageInfo {
                name: "Rust".to_string(),
                lines: 50000,
                files: 200,
            },
            LanguageInfo {
                name: "TypeScript".to_string(),
                lines: 10000,
                files: 50,
            },
            LanguageInfo {
                name: "TOML".to_string(),
                lines: 500,
                files: 5,
            },
        ],
        symbols: SymbolSummary {
            total: 2500,
            functions: 1500,
            types: 500,
            modules: 100,
        },
        security: SecuritySummary {
            critical: 0,
            high: 0,
            medium: 2,
            low: 5,
        },
        quality: QualitySummary {
            test_coverage: Some(85.5),
            avg_complexity: 8.2,
            doc_coverage: Some(70.0),
        },
    }
}

fn create_sample_architecture() -> CcgArchitecture {
    let mut modules = Vec::with_capacity(20);
    let mut public_api = Vec::with_capacity(50);
    let mut dependency_graph = Vec::with_capacity(40);

    // Create realistic module structure
    let module_names = [
        "api",
        "tui",
        "tools",
        "mcp",
        "hooks",
        "skills",
        "commands",
        "agents",
        "plugins",
        "context",
        "update",
        "ide",
        "util",
        "types",
        "session",
        "worktree",
        "narsil",
        "continuous",
        "terminal",
        "app",
    ];

    for (i, name) in module_names.iter().enumerate() {
        modules.push(ModuleInfo {
            name: (*name).to_string(),
            path: format!("src/{}", name),
            public_symbols: 30 + (i * 5),
            private_symbols: 20 + (i * 5),
        });

        // Add some public API exports
        public_api.push(ExportInfo {
            name: format!("{}Client", name),
            kind: "struct".to_string(),
            module: format!("src/{}/mod.rs", name),
            signature: None,
        });

        // Add dependency edges (each module depends on some others)
        if i > 0 {
            dependency_graph.push(DependencyEdge {
                from: (*name).to_string(),
                to: module_names[i - 1].to_string(),
                weight: 1,
            });
        }
        if i > 2 {
            dependency_graph.push(DependencyEdge {
                from: (*name).to_string(),
                to: "types".to_string(),
                weight: 1,
            });
        }
    }

    CcgArchitecture {
        modules,
        public_api,
        dependency_graph,
    }
}

// =============================================================================
// Benchmarks
// =============================================================================

/// Benchmark: Manifest markdown rendering
/// Target: <10ms
fn bench_manifest_rendering(c: &mut Criterion) {
    let manifest = create_sample_manifest();

    c.bench_function("manifest_rendering", |b| {
        b.iter(|| {
            let md = render_manifest_markdown(black_box(&manifest));
            black_box(md);
        });
    });
}

/// Benchmark: Architecture markdown rendering
/// Target: <10ms
fn bench_architecture_rendering(c: &mut Criterion) {
    let architecture = create_sample_architecture();

    c.bench_function("architecture_rendering", |b| {
        b.iter(|| {
            let md = render_architecture_markdown(black_box(&architecture));
            black_box(md);
        });
    });
}

/// Benchmark: Cache lookup (hit path)
/// Target: <1ms
fn bench_cache_lookup_hit(c: &mut Criterion) {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    // Warm up cache
    let _result = orchestrator.get_context(CompressionLevel::Manifest, "hash123", || {
        "Cached content".to_string()
    });

    c.bench_function("cache_lookup_hit", |b| {
        b.iter(|| {
            let cached = orchestrator.get_cached(black_box(CompressionLevel::Manifest), "hash123");
            black_box(cached);
        });
    });
}

/// Benchmark: Cache lookup (miss path)
/// Target: <1ms
fn bench_cache_lookup_miss(c: &mut Criterion) {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    c.bench_function("cache_lookup_miss", |b| {
        b.iter(|| {
            let cached =
                orchestrator.get_cached(black_box(CompressionLevel::Manifest), "nonexistent");
            black_box(cached);
        });
    });
}

/// Benchmark: Default context generation (cache miss)
/// Target: <100ms
fn bench_default_context_generation(c: &mut Criterion) {
    let manifest = create_sample_manifest();
    let architecture = create_sample_architecture();

    c.bench_function("default_context_generation", |b| {
        b.iter_batched(
            || {
                // Create fresh orchestrator for each iteration (cache miss)
                let caps = create_ccg_capabilities();
                CompressionOrchestrator::new(caps, "test-repo")
            },
            |orchestrator| {
                let result = orchestrator.get_default_context(
                    "hash123",
                    || render_manifest_markdown(&manifest),
                    || render_architecture_markdown(&architecture),
                );
                black_box(result);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// Benchmark: Default context generation (cache hit)
/// Target: <1ms
fn bench_default_context_cached(c: &mut Criterion) {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");

    let manifest = create_sample_manifest();
    let architecture = create_sample_architecture();

    // Warm up cache
    let _result = orchestrator.get_default_context(
        "hash123",
        || render_manifest_markdown(&manifest),
        || render_architecture_markdown(&architecture),
    );

    c.bench_function("default_context_cached", |b| {
        b.iter(|| {
            let result = orchestrator.get_default_context(
                "hash123",
                || panic!("Should hit cache"),
                || panic!("Should hit cache"),
            );
            black_box(result);
        });
    });
}

/// Benchmark: Token budget context building
/// Target: <100ms
fn bench_token_budget_context(c: &mut Criterion) {
    let manifest = create_sample_manifest();
    let architecture = create_sample_architecture();

    c.bench_function("token_budget_context", |b| {
        b.iter_batched(
            || {
                let caps = create_ccg_capabilities();
                CompressionOrchestrator::new(caps, "test-repo")
            },
            |orchestrator| {
                let result = orchestrator.build_context_with_budget(
                    black_box(5000),
                    "hash123",
                    || render_manifest_markdown(&manifest),
                    || render_architecture_markdown(&architecture),
                    Some(|| "# Symbol Details".to_string()),
                );
                black_box(result);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// Benchmark: Graceful degradation path
/// Target: <100μs
fn bench_graceful_degradation(c: &mut Criterion) {
    let caps = create_ccg_capabilities();
    let orchestrator = CompressionOrchestrator::new(caps, "test-repo");
    orchestrator.enter_degraded_mode();

    c.bench_function("graceful_degradation", |b| {
        b.iter(|| {
            let result = orchestrator.get_context_graceful(
                black_box(CompressionLevel::Manifest),
                "hash123",
                || panic!("Should not be called"),
            );
            black_box(result);
        });
    });
}

criterion_group!(
    benches,
    bench_manifest_rendering,
    bench_architecture_rendering,
    bench_cache_lookup_hit,
    bench_cache_lookup_miss,
    bench_default_context_generation,
    bench_default_context_cached,
    bench_token_budget_context,
    bench_graceful_degradation,
);

criterion_main!(benches);
