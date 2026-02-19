#!/usr/bin/env bash
# Validate computed metrics against README claims
# Fails if metrics differ significantly from documented values

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Change to project root
cd "$PROJECT_ROOT"

# Thresholds for acceptable drift
LOC_DRIFT_PERCENT=10       # Allow 10% drift in LOC
TEST_DRIFT_PERCENT=5       # Allow 5% drift in test count
TEST_DRIFT_ABS=50          # Or 50 absolute test difference

# Get computed metrics
COMPUTED_METRICS=$("$SCRIPT_DIR/compute_metrics.sh")
COMPUTED_LOC=$(echo "$COMPUTED_METRICS" | jq -r '.loc')
COMPUTED_TESTS=$(echo "$COMPUTED_METRICS" | jq -r '.total_tests')
COMPUTED_VERSION=$(echo "$COMPUTED_METRICS" | jq -r '.version')

echo "Computed metrics:"
echo "  LOC: $COMPUTED_LOC"
echo "  Tests: $COMPUTED_TESTS"
echo "  Version: $COMPUTED_VERSION"
echo ""

# Extract documented metrics from README
# Look for patterns like "~XX,000 LOC" or "XX,XXX LOC" or "1,350+ tests"
README_LOC=$(grep -oE '[0-9,]+\s*LOC' README.md | head -1 | tr -d ',LOC ' | tr -d '~')
README_TESTS=$(grep -oE '[0-9,]+\+?\s*tests' README.md | head -1 | tr -d ',+tests ')

# Also check for metric markers (future-proof)
# Format: <!-- METRICS:tests=NNNN,loc=NNNN -->
if grep -q '<!-- METRICS:' README.md; then
    MARKER_TESTS=$(grep -oE '<!-- METRICS:[^>]*tests=([0-9]+)' README.md | grep -oE 'tests=[0-9]+' | cut -d= -f2)
    MARKER_LOC=$(grep -oE '<!-- METRICS:[^>]*loc=([0-9]+)' README.md | grep -oE 'loc=[0-9]+' | cut -d= -f2)
    README_TESTS=${MARKER_TESTS:-$README_TESTS}
    README_LOC=${MARKER_LOC:-$README_LOC}
fi

# Use defaults if not found
README_LOC=${README_LOC:-0}
README_TESTS=${README_TESTS:-0}

echo "Documented metrics (from README.md):"
echo "  LOC: $README_LOC"
echo "  Tests: $README_TESTS"
echo ""

# Validation functions
validate_with_percent() {
    local computed=$1
    local documented=$2
    local threshold=$3
    local name=$4

    if [ "$documented" -eq 0 ]; then
        echo "WARNING: No documented $name found in README"
        return 0
    fi

    local diff=$((computed - documented))
    local abs_diff=${diff#-}  # Absolute value
    local percent_diff=$((abs_diff * 100 / documented))

    if [ "$percent_diff" -gt "$threshold" ]; then
        echo "FAIL: $name drift too high: $percent_diff% (threshold: $threshold%)"
        echo "  Computed: $computed"
        echo "  Documented: $documented"
        return 1
    fi

    echo "OK: $name drift within threshold: $percent_diff% (threshold: $threshold%)"
    return 0
}

validate_tests_with_floor() {
    local computed=$1
    local documented=$2
    local percent_threshold=$3
    local abs_threshold=$4

    if [ "$documented" -eq 0 ]; then
        echo "WARNING: No documented test count found in README"
        return 0
    fi

    local diff=$((computed - documented))
    local abs_diff=${diff#-}  # Absolute value
    local percent_diff=$((abs_diff * 100 / documented))

    # Test count should only go UP, never significantly down
    if [ "$computed" -lt "$documented" ]; then
        # Allow small decreases (e.g., refactoring that removes redundant tests)
        if [ "$abs_diff" -gt "$abs_threshold" ]; then
            echo "FAIL: Test count decreased significantly"
            echo "  Computed: $computed"
            echo "  Documented: $documented"
            echo "  Decrease: $abs_diff (threshold: $abs_threshold)"
            return 1
        fi
    fi

    if [ "$percent_diff" -gt "$percent_threshold" ] && [ "$abs_diff" -gt "$abs_threshold" ]; then
        echo "WARNING: Test count drift: $percent_diff% ($abs_diff tests)"
        echo "  Consider updating README.md"
    fi

    echo "OK: Test count acceptable (computed: $computed, documented: $documented+)"
    return 0
}

# Run validations
EXIT_CODE=0

echo "=== Validating Metrics ==="
echo ""

# LOC validation (informational only - LOC can fluctuate)
if ! validate_with_percent "$COMPUTED_LOC" "$README_LOC" "$LOC_DRIFT_PERCENT" "LOC"; then
    echo "INFO: LOC drift is informational only (not a CI failure)"
fi
echo ""

# Test count validation (this one matters)
if ! validate_tests_with_floor "$COMPUTED_TESTS" "$README_TESTS" "$TEST_DRIFT_PERCENT" "$TEST_DRIFT_ABS"; then
    EXIT_CODE=1
fi
echo ""

# Version check (informational)
echo "Version check:"
CARGO_VERSION=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/' | tr -d ' ')
if [ "$COMPUTED_VERSION" != "$CARGO_VERSION" ]; then
    echo "WARNING: Version mismatch"
    echo "  Cargo.toml: $CARGO_VERSION"
    echo "  Computed: $COMPUTED_VERSION"
else
    echo "OK: Version matches Cargo.toml: $CARGO_VERSION"
fi
echo ""

# Documentation version consistency check
echo "=== Docs Version Consistency ==="
CARGO_VERSION_MAJOR_MINOR=$(echo "$CARGO_VERSION" | sed 's/\([0-9]*\.[0-9]*\).*/\1/')
# Pattern matches versions that should have been updated (0.5.x through 0.8.x)
# Excludes 0.1.x-0.4.x which may be legitimate min-version requirements
STALE_PATTERN='0\.[5-8]\.[0-9]'

STALE_REFS=$(grep -rn "$STALE_PATTERN" docs/ --include='*.md' --exclude-dir=archive 2>/dev/null || true)
if [ -n "$STALE_REFS" ]; then
    echo "WARNING: Stale version references found in docs:"
    echo "$STALE_REFS" | head -20
    echo ""
    echo "  Cargo.toml version: $CARGO_VERSION"
    echo "  Fix these references to match the current version."
else
    echo "OK: No stale version references in docs/"
fi
echo ""

if [ $EXIT_CODE -eq 0 ]; then
    echo "=== All validations passed ==="
else
    echo "=== Validation FAILED ==="
fi

exit $EXIT_CODE
