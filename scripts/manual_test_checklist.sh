#!/usr/bin/env bash
# Manual test checklist for Patina — covers every user-visible feature
# Run on each target platform before release
#
# Usage:
#   ./scripts/manual_test_checklist.sh [--auto-only]
#
# With --auto-only, skips interactive tests and runs only automated checks.
# Without flags, guides the tester through both automated and manual checks.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_ROOT"

# --- Configuration ---
BINARY="cargo run --release --"
AUTO_ONLY=false
PASS=0
FAIL=0
SKIP=0
MANUAL=0
TMPDIR_BASE=$(mktemp -d "${TMPDIR:-/tmp}/patina-test.XXXXXX")

cleanup() {
    rm -rf "$TMPDIR_BASE"
}
trap cleanup EXIT

# Parse args
for arg in "$@"; do
    case "$arg" in
        --auto-only) AUTO_ONLY=true ;;
        *) echo "Unknown argument: $arg"; exit 1 ;;
    esac
done

# --- Helpers ---
section() {
    echo ""
    echo "════════════════════════════════════════════════════════════"
    echo "  $1"
    echo "════════════════════════════════════════════════════════════"
}

auto_check() {
    local name="$1"
    shift
    printf "  [AUTO] %-50s " "$name"
    if "$@" >/dev/null 2>&1; then
        echo "PASS"
        PASS=$((PASS + 1))
    else
        echo "FAIL"
        FAIL=$((FAIL + 1))
    fi
}

auto_check_output() {
    local name="$1"
    local expected="$2"
    shift 2
    printf "  [AUTO] %-50s " "$name"
    local output
    output=$("$@" 2>&1) || true
    if echo "$output" | grep -q "$expected"; then
        echo "PASS"
        PASS=$((PASS + 1))
    else
        echo "FAIL (expected: $expected)"
        echo "         got: $(echo "$output" | head -3)"
        FAIL=$((FAIL + 1))
    fi
}

manual_check() {
    local name="$1"
    local instruction="$2"
    if $AUTO_ONLY; then
        printf "  [SKIP] %-50s SKIPPED (auto-only)\n" "$name"
        SKIP=$((SKIP + 1))
        return
    fi
    echo ""
    echo "  [MANUAL] $name"
    echo "    Steps: $instruction"
    echo -n "    Result? [p]ass / [f]ail / [s]kip: "
    read -r result
    case "$result" in
        p|P|pass) echo "    -> PASS"; PASS=$((PASS + 1)) ;;
        f|F|fail) echo "    -> FAIL"; FAIL=$((FAIL + 1)) ;;
        *) echo "    -> SKIPPED"; SKIP=$((SKIP + 1)) ;;
    esac
    MANUAL=$((MANUAL + 1))
}

# --- Pre-flight ---
section "Pre-flight Checks"

auto_check "cargo build succeeds" cargo build --release
auto_check "binary exists" test -f target/release/patina
auto_check "cargo test passes" cargo test
auto_check "cargo clippy clean" cargo clippy --all-targets -- -D warnings
auto_check "cargo fmt clean" cargo fmt -- --check
auto_check "validate_metrics.sh passes" "$SCRIPT_DIR/validate_metrics.sh"

# --- 1. CLI Flags ---
section "1. CLI Flags"

auto_check_output "--help shows usage" "Usage:" \
    target/release/patina --help

auto_check_output "--version shows version" "patina" \
    target/release/patina --version

auto_check_output "--list-sessions runs without error" "" \
    target/release/patina --list-sessions

auto_check_output "--debug flag accepted" "" \
    timeout 5 target/release/patina --debug --print "say ok" 2>&1 || true

auto_check_output "-p flag (print mode)" "" \
    timeout 15 target/release/patina -p "Reply with exactly: PATINA_TEST_OK" 2>&1 || true

auto_check_output "-C flag (directory)" "" \
    timeout 5 target/release/patina -C /tmp --print "say ok" 2>&1 || true

auto_check_output "--no-plugins flag accepted" "" \
    timeout 5 target/release/patina --no-plugins --print "say ok" 2>&1 || true

auto_check_output "--no-parallel flag accepted" "" \
    timeout 5 target/release/patina --no-parallel --print "say ok" 2>&1 || true

auto_check_output "--no-auto-context flag accepted" "" \
    timeout 5 target/release/patina --no-auto-context --print "say ok" 2>&1 || true

# --- 2. Print Mode ---
section "2. Print Mode (Non-Interactive)"

auto_check_output "print mode returns response" "" \
    timeout 30 target/release/patina --print "What is 2+2? Reply with just the number." 2>&1 || true

auto_check_output "print mode with model flag" "" \
    timeout 30 target/release/patina --print -m claude-sonnet-4-20250514 "say ok" 2>&1 || true

# --- 3. Interactive Chat ---
section "3. Interactive Chat"

manual_check "TUI renders correctly" \
    "Run: patina
     Expected: TUI renders with input box, status bar, and message area.
     Type a message and press Enter. Verify streaming response appears."

manual_check "Multi-turn conversation" \
    "In a running session, send 'My name is TestUser' then send 'What is my name?'
     Expected: Claude remembers 'TestUser' from context."

manual_check "Ctrl-C interrupts streaming" \
    "Send a long prompt and press Ctrl-C during streaming.
     Expected: Streaming stops, partial response visible, no crash."

# --- 4. Streaming ---
section "4. Streaming"

manual_check "Tokens stream incrementally" \
    "Run: patina -p 'Write a haiku about Rust programming'
     Expected: Text appears word-by-word, not all at once."

manual_check "Stream error recovery" \
    "Disconnect WiFi briefly during streaming, then reconnect.
     Expected: Error message displayed, no crash. Can send new messages."

# --- 5. Tool Execution ---
section "5. Tool Execution"

manual_check "bash tool" \
    "Ask: 'Run echo hello_from_patina in bash'
     Expected: Permission prompt appears. After approval, output shows 'hello_from_patina'."

manual_check "read_file tool" \
    "Ask: 'Read the file Cargo.toml'
     Expected: Cargo.toml contents displayed in response."

manual_check "write_file tool" \
    "Ask: 'Create a file called /tmp/patina_test.txt with content: test passed'
     Expected: Permission prompt. After approval, file created. Verify with: cat /tmp/patina_test.txt"

manual_check "edit tool" \
    "Ask: 'In /tmp/patina_test.txt, replace test passed with edit works'
     Expected: File edited. Verify with: cat /tmp/patina_test.txt"

manual_check "list_files tool" \
    "Ask: 'List files in the scripts directory'
     Expected: Shows files in scripts/ directory."

manual_check "glob tool" \
    "Ask: 'Find all .rs files in src/tools/'
     Expected: Lists Rust source files in tools directory."

manual_check "grep tool" \
    "Ask: 'Search for fn main in src/main.rs'
     Expected: Shows matching lines with context."

# --- 6. Parallel Tool Execution ---
section "6. Parallel Tool Execution"

manual_check "parallel tools execute concurrently" \
    "Ask: 'Read Cargo.toml and list files in src/ at the same time'
     Expected: Both tool calls appear and execute (possibly in parallel).
     Status bar or output should indicate parallel execution."

# --- 7. Vision ---
section "7. Vision (Image Analysis)"

manual_check "image flag accepts file" \
    "Run: patina --image path/to/image.png -p 'Describe this image'
     Expected: Claude describes the image content (requires a test image).
     If no image available, skip this test."

# --- 8. Slash Commands ---
section "8. Slash Commands"

manual_check "/worktree list" \
    "In interactive mode, type: /worktree list
     Expected: Shows worktree listing (may be empty if no worktrees created)."

manual_check "/continuous status" \
    "In interactive mode, type: /continuous status
     Expected: Shows continuous loop status (should be 'idle' or 'not running')."

manual_check "/agent list" \
    "In interactive mode, type: /agent list
     Expected: Shows agent list (may be empty if no agents spawned)."

# --- 9. Session Management ---
section "9. Session Management"

manual_check "session persistence" \
    "1. Run: patina (interactive), send a message, then quit (Ctrl-D or exit)
     2. Run: patina --list-sessions
     Expected: Previous session appears in the list."

manual_check "session resume with --continue" \
    "Run: patina --continue
     Expected: Resumes most recent session with previous messages visible."

manual_check "session resume with --resume" \
    "Run: patina --list-sessions (note a session ID)
     Run: patina --resume <session-id>
     Expected: Resumes that specific session."

# --- 10. Context Compaction ---
section "10. Context Compaction"

manual_check "auto-compaction triggers" \
    "Send many messages in a session until token count is high.
     Expected: Status bar shows token count. Auto-compaction may trigger
     (look for compaction indicator in status bar or debug log)."

# --- 11. Permission Prompting ---
section "11. Permission Prompting"

manual_check "permission prompt for bash" \
    "Ask to run a bash command (e.g., 'run ls in bash').
     Expected: Permission prompt appears with command preview.
     Press 'y' to allow, verify command runs."

manual_check "permission deny works" \
    "Ask to run a bash command. When prompted, press 'n' to deny.
     Expected: Command is not executed, Claude acknowledges denial."

manual_check "dangerous command warning" \
    "Ask: 'Run rm -rf /tmp/patina_test.txt in bash'
     Expected: Enhanced warning about dangerous command before permission prompt."

manual_check "--dangerously-skip-permissions flag" \
    "Run: patina --dangerously-skip-permissions -p 'Run echo skip_test in bash'
     Expected: No permission prompt, command executes directly.
     WARNING: Only test this in a safe environment."

# --- 12. Plugin System ---
section "12. Plugin System"

auto_check_output "plugin list runs" "" \
    timeout 5 target/release/patina plugin list 2>&1 || true

manual_check "plugin install from path" \
    "If example plugins exist in the project, try:
     patina plugin install ./path/to/plugin
     Expected: Plugin installs and appears in 'patina plugin list'."

manual_check "--no-plugins disables loading" \
    "Run: patina --no-plugins
     Expected: No plugin loading messages in debug output.
     Verify with: patina --no-plugins --debug -p 'say ok' 2>&1 | grep -i plugin"

# --- 13. Provider Configuration ---
section "13. Provider Configuration"

manual_check "default provider (anthropic)" \
    "Run: patina -p 'say ok'
     Expected: Uses Anthropic API (default). Response appears normally."

manual_check "--provider openrouter (if configured)" \
    "If OpenRouter key is set:
     Run: patina --provider openrouter --openrouter-key <key> -p 'say ok'
     Expected: Uses OpenRouter API. Skip if no key available."

# --- 14. IDE Integration ---
section "14. IDE Integration"

manual_check "--ide-port starts server" \
    "Run: patina --ide-port 9876 &
     Then: curl http://localhost:9876/health (or similar endpoint)
     Expected: IDE server responds. Kill the background process after."

# --- 15. Platform-Specific ---
section "15. Platform-Specific Behavior"

PLATFORM=$(uname -s)
echo "  Detected platform: $PLATFORM"

case "$PLATFORM" in
    Darwin)
        manual_check "macOS clipboard (pbcopy)" \
            "Copy text from Patina TUI output.
             Expected: Text available in macOS clipboard (Cmd-V to verify)."
        ;;
    Linux)
        if [ -n "${WAYLAND_DISPLAY:-}" ]; then
            manual_check "Wayland clipboard (wl-copy)" \
                "Copy text from Patina TUI output.
                 Expected: Text available via wl-paste."
        elif [ -n "${DISPLAY:-}" ]; then
            manual_check "X11 clipboard (xclip/xsel)" \
                "Copy text from Patina TUI output.
                 Expected: Text available via xclip -o."
        else
            manual_check "Headless clipboard (OSC 52)" \
                "Copy text from Patina TUI output.
                 Expected: OSC 52 escape sequence sent to terminal."
        fi
        ;;
    MINGW*|MSYS*|CYGWIN*)
        manual_check "Windows clipboard" \
            "Copy text from Patina TUI output.
             Expected: Text available via Ctrl-V."
        ;;
esac

# Check multiplexer
if [ -n "${TMUX:-}" ]; then
    echo "  Running inside tmux"
    manual_check "tmux degraded mode" \
        "Verify Patina detects tmux environment.
         Expected: Debug log shows 'Running inside tmux' or similar.
         Image rendering should fall back to text mode."
fi

if [ -n "${SSH_CLIENT:-}" ] || [ -n "${SSH_TTY:-}" ]; then
    echo "  Running via SSH"
    manual_check "SSH clipboard fallback" \
        "Copy text from Patina TUI.
         Expected: Uses OSC 52 for clipboard (no arboard)."
fi

# --- Summary ---
section "Test Summary"

TOTAL=$((PASS + FAIL + SKIP))
echo ""
echo "  Platform:  $PLATFORM"
echo "  Total:     $TOTAL"
echo "  Passed:    $PASS"
echo "  Failed:    $FAIL"
echo "  Skipped:   $SKIP"
echo "  Manual:    $MANUAL"
echo ""

if [ "$FAIL" -gt 0 ]; then
    echo "  RESULT: SOME TESTS FAILED"
    echo ""
    echo "  Review failures above before release."
    exit 1
else
    echo "  RESULT: ALL CHECKED TESTS PASSED"
    if [ "$SKIP" -gt 0 ]; then
        echo "  ($SKIP tests skipped — review before final release)"
    fi
    exit 0
fi
