#!/usr/bin/env bash
# Compute project metrics for CI validation
# Outputs JSON with loc, test_count, version fields

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Change to project root
cd "$PROJECT_ROOT"

# Function to count Rust LOC using tokei (with fallback)
count_loc() {
    if command -v tokei &>/dev/null; then
        # Use tokei and extract Rust code lines
        tokei --output json src 2>/dev/null | \
            jq -r '.Rust.code // 0' 2>/dev/null || echo "0"
    else
        # Fallback: count non-empty, non-comment lines in .rs files
        find src -name "*.rs" -type f -exec cat {} + 2>/dev/null | \
            grep -v '^\s*$' | \
            grep -v '^\s*//' | \
            grep -v '^\s*\*' | \
            wc -l | \
            tr -d ' '
    fi
}

# Function to count tests
count_tests() {
    # Count test functions from cargo test --list output
    # Each test ends with ": test" in the listing
    cargo test -- --list 2>/dev/null | \
        grep -c ': test$' || echo "0"
}

# Function to extract version from Cargo.toml
get_version() {
    grep '^version = ' Cargo.toml | \
        head -1 | \
        sed 's/version = "\(.*\)"/\1/' | \
        tr -d ' '
}

# Function to count doc tests
count_doc_tests() {
    cargo test --doc -- --list 2>/dev/null | \
        grep -c ': test$' || echo "0"
}

# Compute all metrics
LOC=$(count_loc)
TEST_COUNT=$(count_tests)
DOC_TEST_COUNT=$(count_doc_tests)
VERSION=$(get_version)
TOTAL_TESTS=$((TEST_COUNT + DOC_TEST_COUNT))

# Output as JSON
cat <<EOF
{
  "loc": $LOC,
  "test_count": $TEST_COUNT,
  "doc_test_count": $DOC_TEST_COUNT,
  "total_tests": $TOTAL_TESTS,
  "version": "$VERSION"
}
EOF
