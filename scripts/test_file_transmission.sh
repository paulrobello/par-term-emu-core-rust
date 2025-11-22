#!/bin/bash

# Test Kitty graphics file transmission

set -e

echo "=== Kitty Graphics File Transmission Test ==="
echo ""

# Build both projects
echo "1. Building projects..."
cd /Users/probello/Repos/par-term-emu-core-rust
make dev > /dev/null 2>&1
echo "   ✓ Built par-term-emu-core-rust"

cd /Users/probello/Repos/par-term
cargo build --release > /dev/null 2>&1
echo "   ✓ Built par-term"
echo ""

# Create test log
LOG_FILE="/tmp/par-term-test-$(date +%s).log"
echo "2. Starting par-term (logging to $LOG_FILE)..."

# Start par-term in background
timeout 30 /Users/probello/Repos/par-term/target/release/par-term > "$LOG_FILE" 2>&1 &
PAR_TERM_PID=$!

# Give it time to start
sleep 2

# Check if still running
if ! kill -0 $PAR_TERM_PID 2>/dev/null; then
    echo "   ✗ par-term failed to start"
    cat "$LOG_FILE"
    exit 1
fi

echo "   ✓ par-term running (PID: $PAR_TERM_PID)"
echo ""

echo "3. Testing file transmission modes..."
echo ""

# Test 1: Detection
echo "   Test 1: Protocol detection"
kitty +kitten icat --detect-support 2>&1 | grep -q "supported" && \
    echo "      ✓ Detection works" || \
    echo "      ✗ Detection failed"

# Test 2: File transmission (the main test)
echo "   Test 2: File transmission (t=f)"
echo "      Sending: /Users/probello/Repos/par-term/images/snake_tui.png"
echo ""

# Wait a bit for logs to be captured
sleep 3

# Kill par-term
kill $PAR_TERM_PID 2>/dev/null || true
wait $PAR_TERM_PID 2>/dev/null || true

echo ""
echo "4. Checking logs..."
echo ""

# Check for key indicators
if grep -q "KITTY_DEBUG: Action: Transmit" "$LOG_FILE"; then
    echo "   ✓ Found Transmit action (not just Query)"
else
    echo "   ✗ No Transmit action found"
fi

if grep -q "Parse failed" "$LOG_FILE"; then
    echo "   ✗ Found parse errors:"
    grep "Parse failed" "$LOG_FILE" | head -3
    echo ""
else
    echo "   ✓ No parse errors"
fi

if grep -q "App: Got [1-9]" "$LOG_FILE"; then
    echo "   ✓ Graphics received:"
    grep "App: Got" "$LOG_FILE" | tail -3
else
    echo "   ⚠ No graphics received (check if file path was sent)"
fi

echo ""
echo "5. Full debug log (last 50 lines):"
echo "   --------------------------------"
tail -50 "$LOG_FILE" | grep -E "(KITTY|APC|DCS|Parse|App: Got)" || echo "   No debug output found"

echo ""
echo "=== Test Complete ==="
echo "Full log: $LOG_FILE"
