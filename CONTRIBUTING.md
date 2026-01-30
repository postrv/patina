# Contributing to RCT

Thank you for your interest in contributing to RCT (Rust Claude Terminal)! This document provides guidelines and workflows for contributing.

## Development Setup

### Prerequisites

- Rust 1.75 or later
- Git
- `gh` CLI (for GitHub operations)

### Getting Started

1. Fork and clone the repository:

```bash
gh repo fork your-org/rct --clone
cd rct
```

2. Install development dependencies:

```bash
cargo build
```

3. Run the test suite to verify setup:

```bash
cargo test
```

4. Run linters:

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
```

## Code Standards

### Forbidden Patterns

The following patterns are **not allowed** in merged code:

```rust
#[allow(dead_code)]           // Wire it in or delete it
#[allow(unused_*)]            // Use it or remove it
#[allow(clippy::*)]           // Fix the underlying issue
todo!()                       // Implement now
unimplemented!()              // Implement or remove
// TODO: ...                  // Implement now or don't merge
// FIXME: ...                 // Fix now or don't merge
panic!("not implemented")     // Implement or remove
```

### Required Patterns

```rust
#[must_use]                   // On functions returning values that should be used
/// # Panics                  // Document panic conditions
/// # Errors                  // Document error conditions
/// # Examples                // Provide usage examples for public APIs
#[cfg(test)]                  // Keep tests in modules
```

### Documentation

- All public types and functions must have doc comments
- Use `# Panics` and `# Errors` sections where applicable
- Include `# Examples` for complex public APIs
- Keep comments focused on "why" not "what"

### Error Handling

- Use `anyhow::Result` for application-level errors
- Use `thiserror` for library-level error types
- Prefer `?` operator over explicit matching
- Provide context with `.context()` for better error messages

## TDD Workflow

RCT follows Test-Driven Development. The cycle is:

```
REINDEX → RED → GREEN → REFACTOR → REVIEW → COMMIT → REINDEX
```

### 1. REINDEX

If using narsil-mcp, refresh the code index:

```bash
# Via narsil-mcp
narsil reindex
```

### 2. RED - Write Failing Tests First

Before writing any implementation code:

1. Write test(s) that define the expected behavior
2. Run tests to confirm they fail for the right reason
3. Document the behavioral contract in test comments

```rust
#[test]
fn test_new_feature_expected_behavior() {
    // This test should fail until the feature is implemented
    let result = new_feature(input);
    assert_eq!(result, expected_output);
}
```

### 3. GREEN - Minimal Implementation

Write the minimum code needed to make tests pass:

- Don't add functionality beyond what tests require
- Keep implementations simple and focused
- Avoid premature optimization

### 4. REFACTOR - Clean Up

With tests passing, improve the code:

- Extract common patterns (only if used 3+ times)
- Improve naming and organization
- Keep tests green throughout

### 5. REVIEW - Quality Gates

Before committing, all gates must pass:

```bash
# Run clippy with warnings as errors
cargo clippy --all-targets -- -D warnings

# Run all tests
cargo test

# Check formatting
cargo fmt -- --check

# Security scan (if narsil-mcp available)
narsil scan_security
```

### 6. COMMIT

Only commit when ALL gates pass:

```bash
git add -A
git commit -m "feat: descriptive message"
```

### Test Requirements

- Every public function: at least 1 test
- Every public type: exercised in tests
- Every error path: tested
- Use `#[should_panic]` for expected panics
- Use `#[cfg(test)]` modules for unit tests
- Integration tests go in `tests/` directory

## Pull Request Process

### Before Opening a PR

1. Ensure all quality gates pass
2. Update documentation if API changed
3. Add tests for new functionality
4. Run the full test suite

### PR Title Format

Use conventional commits format:

- `feat:` New feature
- `fix:` Bug fix
- `docs:` Documentation
- `refactor:` Code refactoring
- `test:` Test changes
- `chore:` Maintenance

Examples:
- `feat: add session persistence`
- `fix: handle empty input in bash tool`
- `docs: update API reference for hooks`

### PR Description

Include:

```markdown
## Summary

Brief description of changes.

## Test Plan

- [ ] Unit tests pass
- [ ] Integration tests pass
- [ ] Manual testing performed

## Checklist

- [ ] Code follows project standards
- [ ] Tests added/updated
- [ ] Documentation updated
- [ ] All quality gates pass
```

### Review Process

1. PRs require at least one approval
2. All CI checks must pass
3. Address review feedback promptly
4. Squash commits before merge if requested

## Project Structure

```
rct/
├── src/
│   ├── main.rs          # Entry point
│   ├── lib.rs           # Library crate root
│   ├── api/             # Anthropic API client
│   ├── app/             # Application state and event loop
│   ├── agents/          # Subagent orchestration
│   ├── commands/        # Slash commands
│   ├── context/         # Project context loading
│   ├── enterprise/      # Enterprise features (audit, cost)
│   ├── hooks/           # Lifecycle hooks
│   ├── ide/             # IDE integration
│   ├── mcp/             # MCP protocol client
│   ├── plugins/         # Plugin system
│   ├── session/         # Session persistence
│   ├── skills/          # Skill engine
│   ├── tools/           # Tool execution
│   ├── tui/             # Terminal UI
│   ├── types/           # Core types
│   ├── update/          # Auto-update
│   └── util/            # Utilities
├── tests/
│   ├── common/          # Test utilities
│   ├── unit/            # Unit tests
│   └── *.rs             # Integration tests
├── benches/             # Benchmarks
├── docs/                # Documentation
└── examples/            # Example plugins
```

## Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run tests with output
cargo test -- --nocapture

# Run unit tests only
cargo test --lib

# Run integration tests only
cargo test --test '*'

# Run doc tests
cargo test --doc

# Run benchmarks
cargo bench
```

## Adding New Features

1. **Discuss first**: For significant changes, open an issue to discuss
2. **Write tests first**: Follow TDD workflow
3. **Keep it focused**: One feature per PR
4. **Document**: Update relevant documentation
5. **Consider backwards compatibility**: Avoid breaking changes when possible

## Reporting Bugs

Open an issue with:

- RCT version
- Rust version
- Operating system
- Steps to reproduce
- Expected vs actual behavior
- Relevant logs/output

## Security Issues

For security vulnerabilities, please see [SECURITY.md](SECURITY.md).

## Code of Conduct

Be respectful and constructive. We're all here to build something great together.

## Getting Help

- Open an issue for questions
- Check existing issues and discussions
- Review the documentation

## License

By contributing, you agree that your contributions will be licensed under the same license as the project (MIT OR Apache-2.0).

---

Thank you for contributing to RCT!
