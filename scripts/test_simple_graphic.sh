#!/bin/bash
# Simple test to verify graphics are working

echo "Testing basic Sixel graphics..."
echo ""

# Simple red square (10x10 pixels)
echo -ne '\033Pq"1;1;10;10#0;2;0;0;0#1;2;100;0;0#1!10~-#1!10~-#1!10~-#1!10~-#1!10~-#1!10~-#1!10~-#1!10~-#1!10~-#1!10~-\033\\'

echo ""
echo "Red square should appear above this line."
echo ""

# Wait a bit to see the graphic
sleep 2

echo "If you see a red square above, graphics are working!"
echo "If not, check:"
echo "  1. Is par-term running?"
echo "  2. Check debug logs: tail -f /tmp/par_term_debug.log"
