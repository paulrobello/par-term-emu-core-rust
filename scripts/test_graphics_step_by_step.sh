#!/bin/bash
# Step-by-step graphics testing for par-term

echo "========================================="
echo "Graphics Test Suite for par-term"
echo "========================================="
echo ""

echo "Test 1: Simple Sixel Graphic"
echo "-----------------------------"
echo "Sending a small red square (Sixel protocol)..."
echo ""

# Simple 10x10 red square
printf '\033Pq"1;1;10;10#0;2;0;0;0#1;2;100;0;0'
for i in {1..10}; do
    printf '#1!10~-'
done
printf '\033\\'

echo ""
echo "⬆️  A red square should appear above"
echo ""
sleep 2

echo ""
echo "Test 2: Larger Sixel Graphic"
echo "-----------------------------"
echo "Sending a 50x50 blue square..."
echo ""

# 50x50 blue square
printf '\033Pq"1;1;50;50#0;2;0;0;0#1;2;0;0;100'
for i in {1..50}; do
    printf '#1!50~-'
done
printf '\033\\'

echo ""
echo "⬆️  A blue square should appear above"
echo ""
sleep 2

echo ""
echo "Test 3: Check if graphics are in backend"
echo "-----------------------------------------"
python3 << 'PYEOF'
import sys
sys.path.insert(0, '/Users/probello/Repos/par-term-emu-core-rust')
try:
    from par_term_emu_core_rust import Terminal
    term = Terminal(80, 24)
    # Send a Sixel directly
    term.feed_str('\033Pq"1;1;20;20#0;2;0;0;0#1;2;100;0;0#1!20~-#1!20~-#1!20~-\033\\')
    print(f"Graphics count: {len(term.all_graphics())}")
    if len(term.all_graphics()) > 0:
        g = term.all_graphics()[0]
        print(f"  Protocol: {g.protocol}")
        print(f"  Position: {g.position}")
        print(f"  Size: {g.width}x{g.height}")
        print("✅ Backend graphics storage working!")
    else:
        print("❌ No graphics stored in backend!")
        print("   This indicates a Sixel parsing issue.")
except Exception as e:
    print(f"❌ Error: {e}")
    import traceback
    traceback.print_exc()
PYEOF

echo ""
echo "Test 4: Kitty Graphics Protocol"
echo "--------------------------------"

if command -v python3 &> /dev/null; then
    echo "Attempting to send a Kitty graphic (requires PIL)..."
    python3 << 'KITTYEOF'
try:
    from PIL import Image
    import io
    import base64

    # Create a simple 100x100 green square
    img = Image.new('RGB', (100, 100), (0, 255, 0))
    buf = io.BytesIO()
    img.save(buf, format='PNG')
    data = base64.standard_b64encode(buf.getvalue()).decode('ascii')

    # Send Kitty graphics command
    print(f'\033_Ga=T,f=100,t=d;{data}\033\\', end='', flush=True)
    print()
    print("⬆️  A green square should appear above")
    print("✅ Kitty graphics command sent")
except ImportError:
    print("⚠️  PIL not installed, skipping Kitty test")
    print("   Install with: uv pip install Pillow")
except Exception as e:
    print(f"❌ Error: {e}")
KITTYEOF
else
    echo "⚠️  Python3 not found, skipping Kitty test"
fi

echo ""
echo "========================================="
echo "Test Complete"
echo "========================================="
echo ""
echo "Expected results:"
echo "  1. Red square (10x10) visible"
echo "  2. Blue square (50x50) visible"
echo "  3. Backend reports graphics stored"
echo "  4. Green square (100x100) visible (if PIL installed)"
echo ""
echo "If graphics are NOT visible:"
echo "  1. Check that you're running in par-term (not a regular terminal)"
echo "  2. Check debug logs: tail -f /tmp/par_term_debug.log | grep GRAPHICS"
echo "  3. Verify graphics support in build: cargo run --features graphics"
echo ""
