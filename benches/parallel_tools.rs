//! Parallel tool execution performance benchmarks
//!
//! These benchmarks measure the performance improvement from parallel tool execution:
//! - Multi-file read operations (sequential vs parallel)
//! - Multiple grep operations (sequential vs parallel)
//!
//! Performance targets:
//! - 5x+ speedup on parallel-eligible operations

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use patina::tools::parallel::{ParallelConfig, ParallelExecutor};
use serde_json::json;
use tokio::runtime::Runtime;

/// Simulates a read_file operation with configurable delay.
async fn mock_read_file(path: &str, _simulate_io_ms: u64) -> String {
    // In real benchmarks, we'd read actual files
    // Here we simulate the I/O latency
    tokio::time::sleep(tokio::time::Duration::from_millis(_simulate_io_ms)).await;
    format!("Content of {}", path)
}

/// Benchmark: Sequential multi-file read
fn bench_sequential_file_reads(c: &mut Criterion) {
    let rt = Runtime::new().expect("Failed to create runtime");

    let mut group = c.benchmark_group("file_reads");

    for file_count in [5, 10, 25, 50].iter() {
        group.bench_with_input(
            BenchmarkId::new("sequential", file_count),
            file_count,
            |b, &count| {
                b.to_async(&rt).iter(|| async move {
                    let mut results = Vec::with_capacity(count);
                    for i in 0..count {
                        let result = mock_read_file(&format!("file_{}.txt", i), 1).await;
                        results.push(result);
                    }
                    black_box(results)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Parallel multi-file read using ParallelExecutor
fn bench_parallel_file_reads(c: &mut Criterion) {
    let rt = Runtime::new().expect("Failed to create runtime");

    let mut group = c.benchmark_group("file_reads");

    for file_count in [5, 10, 25, 50].iter() {
        group.bench_with_input(
            BenchmarkId::new("parallel", file_count),
            file_count,
            |b, &count| {
                let executor = ParallelExecutor::new(ParallelConfig::default());

                b.to_async(&rt).iter(|| {
                    let executor = &executor;
                    async move {
                        let tools: Vec<(&str, serde_json::Value)> = (0..count)
                            .map(|i| ("read_file", json!({"path": format!("file_{}.txt", i)})))
                            .collect();

                        let results = executor
                            .execute_batch(tools.into_iter(), |_name, input| {
                                let path = input["path"].as_str().unwrap_or("").to_string();
                                async move { mock_read_file(&path, 1).await }
                            })
                            .await;

                        black_box(results)
                    }
                });
            },
        );
    }

    group.finish();
}

/// Simulates a grep operation with configurable delay.
async fn mock_grep(pattern: &str, _simulate_io_ms: u64) -> Vec<String> {
    tokio::time::sleep(tokio::time::Duration::from_millis(_simulate_io_ms)).await;
    vec![format!("Match for pattern: {}", pattern)]
}

/// Benchmark: Sequential grep operations
fn bench_sequential_grep(c: &mut Criterion) {
    let rt = Runtime::new().expect("Failed to create runtime");

    let mut group = c.benchmark_group("grep_operations");

    for op_count in [3, 5, 10, 20].iter() {
        group.bench_with_input(
            BenchmarkId::new("sequential", op_count),
            op_count,
            |b, &count| {
                b.to_async(&rt).iter(|| async move {
                    let mut results = Vec::with_capacity(count);
                    for i in 0..count {
                        let result = mock_grep(&format!("PATTERN_{}", i), 2).await;
                        results.push(result);
                    }
                    black_box(results)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Parallel grep operations using ParallelExecutor
fn bench_parallel_grep(c: &mut Criterion) {
    let rt = Runtime::new().expect("Failed to create runtime");

    let mut group = c.benchmark_group("grep_operations");

    for op_count in [3, 5, 10, 20].iter() {
        group.bench_with_input(
            BenchmarkId::new("parallel", op_count),
            op_count,
            |b, &count| {
                let executor = ParallelExecutor::new(ParallelConfig::default());

                b.to_async(&rt).iter(|| {
                    let executor = &executor;
                    async move {
                        let tools: Vec<(&str, serde_json::Value)> = (0..count)
                            .map(|i| ("grep", json!({"pattern": format!("PATTERN_{}", i)})))
                            .collect();

                        let results = executor
                            .execute_batch(tools.into_iter(), |_name, input| {
                                let pattern = input["pattern"].as_str().unwrap_or("").to_string();
                                async move { mock_grep(&pattern, 2).await }
                            })
                            .await;

                        black_box(results)
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Mixed operations (read + grep) with some sequential dependencies
fn bench_mixed_operations(c: &mut Criterion) {
    let rt = Runtime::new().expect("Failed to create runtime");

    c.bench_function("mixed_parallel_sequential", |b| {
        let executor = ParallelExecutor::new(ParallelConfig::default());

        b.to_async(&rt).iter(|| {
            let executor = &executor;
            async move {
                // Mix of ReadOnly (parallelizable) and Unknown (sequential) tools
                let tools: Vec<(&str, serde_json::Value)> = vec![
                    ("read_file", json!({"path": "file_1.txt"})),
                    ("read_file", json!({"path": "file_2.txt"})),
                    ("read_file", json!({"path": "file_3.txt"})),
                    ("bash", json!({"command": "echo test"})), // Forces sequential
                    ("read_file", json!({"path": "file_4.txt"})),
                    ("read_file", json!({"path": "file_5.txt"})),
                ];

                let results = executor
                    .execute_batch(tools.into_iter(), |name, input| {
                        let name = name.to_string();
                        async move {
                            if name == "bash" {
                                // Simulate bash command
                                tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                                "bash output".to_string()
                            } else {
                                let path = input["path"].as_str().unwrap_or("").to_string();
                                mock_read_file(&path, 1).await
                            }
                        }
                    })
                    .await;

                black_box(results)
            }
        });
    });
}

/// Benchmark: Tool classification overhead
fn bench_tool_classification(c: &mut Criterion) {
    use patina::tools::parallel::{classify_bash_command, classify_tool};

    let mut group = c.benchmark_group("classification");

    group.bench_function("classify_tool", |b| {
        b.iter(|| {
            black_box(classify_tool(black_box("read_file")));
            black_box(classify_tool(black_box("write_file")));
            black_box(classify_tool(black_box("bash")));
            black_box(classify_tool(black_box("mcp__server__tool")));
        });
    });

    group.bench_function("classify_bash_command", |b| {
        b.iter(|| {
            black_box(classify_bash_command(black_box("ls -la")));
            black_box(classify_bash_command(black_box("cat file.txt")));
            black_box(classify_bash_command(black_box("rm -rf /")));
            black_box(classify_bash_command(black_box("git status")));
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_sequential_file_reads,
    bench_parallel_file_reads,
    bench_sequential_grep,
    bench_parallel_grep,
    bench_mixed_operations,
    bench_tool_classification,
);

criterion_main!(benches);
