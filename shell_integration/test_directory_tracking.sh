#!/bin/bash
# Test script to verify directory tracking in TUI

echo "Testing OSC 7 directory tracking..."
echo "You should see the StatusBar update with the current directory"
echo ""

# Send OSC 7 with current directory
printf "\033]7;file://%s%s\007" "$(hostname)" "$PWD"

echo "Current directory: $PWD"
sleep 1

# Change to home directory
cd ~
printf "\033]7;file://%s%s\007" "$(hostname)" "$PWD"
echo "Changed to: $PWD"
sleep 1

# Change to /tmp
cd /tmp
printf "\033]7;file://%s%s\007" "$(hostname)" "$PWD"
echo "Changed to: $PWD"
sleep 1

# Back to original directory
cd -
printf "\033]7;file://%s%s\007" "$(hostname)" "$PWD"
echo "Back to: $PWD"
