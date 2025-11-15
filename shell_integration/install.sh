#!/bin/bash
# par-term-emu-core-rust Shell Integration Installer
#
# This script installs shell integration for par-term-emu-core-rust terminal emulator.
# It auto-detects your shell and installs the appropriate integration script.
#
# Usage:
#   ./install.sh          # Install for current shell
#   ./install.sh --all    # Install for all supported shells
#   ./install.sh bash     # Install for bash only
#   ./install.sh zsh      # Install for zsh only
#   ./install.sh fish     # Install for fish only

set -e

function die() {
  echo "Error: ${1}" >&2
  exit 1
}

function info() {
  echo "==> ${1}"
}

function warn() {
  echo "Warning: ${1}" >&2
}

# Check for required tools
type printf > /dev/null 2>&1 || die "Shell integration requires the printf binary to be in your path."

# Get script directory (where integration scripts are located)
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# Show help if requested
if [ "$1" = "--help" ] || [ "$1" = "-h" ]; then
  cat <<EOF
par-term-emu-core-rust Shell Integration Installer

This script installs shell integration for par-term-emu-core-rust terminal emulator.
Shell integration provides features like:
  - Prompt navigation (jump between commands)
  - Command status tracking (exit codes)
  - Working directory tracking
  - Command duration measurement
  - Smart selection (select command vs output)

Usage:
  $0                Install for current shell (\$SHELL)
  $0 --all          Install for all supported shells found on system
  $0 bash           Install for bash only
  $0 zsh            Install for zsh only
  $0 fish           Install for fish only
  $0 --uninstall    Uninstall shell integration

Supported shells: bash, zsh, fish

EOF
  exit 0
fi

function install_bash() {
  info "Installing shell integration for Bash..."

  DOTFILE="${HOME}/.bashrc"
  # Use .bash_profile if .bashrc doesn't exist
  if [ ! -f "$DOTFILE" ] && [ -f "${HOME}/.bash_profile" ]; then
    DOTFILE="${HOME}/.bash_profile"
  elif [ ! -f "$DOTFILE" ] && [ -f "${HOME}/.profile" ]; then
    DOTFILE="${HOME}/.profile"
  fi

  INSTALL_FILE="${HOME}/.par_term_emu_core_rust_shell_integration.bash"
  SOURCE_FILE="${SCRIPT_DIR}/par_term_emu_core_rust_shell_integration.bash"

  # Copy integration script
  cp "$SOURCE_FILE" "$INSTALL_FILE" || die "Failed to copy bash integration script"
  chmod +x "$INSTALL_FILE"

  # Check if already installed
  if grep -q "par_term_emu_core_rust_shell_integration" "$DOTFILE" 2>/dev/null; then
    warn "Bash shell integration already installed in $DOTFILE"
  else
    info "Adding source command to $DOTFILE..."
    cat >> "$DOTFILE" <<'EOF'

# par-term-emu-core-rust shell integration
if [ -f "$HOME/.par_term_emu_core_rust_shell_integration.bash" ]; then
  source "$HOME/.par_term_emu_core_rust_shell_integration.bash"
fi
EOF
  fi

  info "Bash shell integration installed successfully!"
  info "To activate now, run: source $INSTALL_FILE"
}

function install_zsh() {
  info "Installing shell integration for Zsh..."

  # Handle ZDOTDIR
  DOTDIR="$HOME"
  if [ -n "$ZDOTDIR" ]; then
    info "Using ZDOTDIR: $ZDOTDIR"
    DOTDIR="$ZDOTDIR"
  fi

  DOTFILE="${DOTDIR}/.zshrc"
  INSTALL_FILE="${DOTDIR}/.par_term_emu_core_rust_shell_integration.zsh"
  SOURCE_FILE="${SCRIPT_DIR}/par_term_emu_core_rust_shell_integration.zsh"

  # Copy integration script
  cp "$SOURCE_FILE" "$INSTALL_FILE" || die "Failed to copy zsh integration script"
  chmod +x "$INSTALL_FILE"

  # Check if already installed
  if grep -q "par_term_emu_core_rust_shell_integration" "$DOTFILE" 2>/dev/null; then
    warn "Zsh shell integration already installed in $DOTFILE"
  else
    info "Adding source command to $DOTFILE..."
    cat >> "$DOTFILE" <<'EOF'

# par-term-emu-core-rust shell integration
if [ -f "${ZDOTDIR:-$HOME}/.par_term_emu_core_rust_shell_integration.zsh" ]; then
  source "${ZDOTDIR:-$HOME}/.par_term_emu_core_rust_shell_integration.zsh"
fi
EOF
  fi

  info "Zsh shell integration installed successfully!"
  info "To activate now, run: source $INSTALL_FILE"
}

function install_fish() {
  info "Installing shell integration for Fish..."

  # Check fish version
  if command -v fish > /dev/null 2>&1; then
    FISH_VERSION=$(fish --version 2>&1 | grep -oE '[0-9]+\.[0-9]+' | head -1)
    info "Detected Fish version: $FISH_VERSION"

    # Parse version and check if >= 2.3
    FISH_MAJOR=$(echo "$FISH_VERSION" | cut -d. -f1)
    FISH_MINOR=$(echo "$FISH_VERSION" | cut -d. -f2)

    if [ "$FISH_MAJOR" -lt 2 ] || ([ "$FISH_MAJOR" -eq 2 ] && [ "$FISH_MINOR" -lt 3 ]); then
      warn "Fish 2.3 or later is recommended. Your version: $FISH_VERSION"
    fi
  else
    warn "Fish shell not found in PATH"
  fi

  FISH_CONFIG_DIR="${HOME}/.config/fish"
  mkdir -p "$FISH_CONFIG_DIR"

  DOTFILE="${FISH_CONFIG_DIR}/config.fish"
  INSTALL_FILE="${HOME}/.par_term_emu_core_rust_shell_integration.fish"
  SOURCE_FILE="${SCRIPT_DIR}/par_term_emu_core_rust_shell_integration.fish"

  # Copy integration script
  cp "$SOURCE_FILE" "$INSTALL_FILE" || die "Failed to copy fish integration script"
  chmod +x "$INSTALL_FILE"

  # Check if already installed
  if grep -q "par_term_emu_core_rust_shell_integration" "$DOTFILE" 2>/dev/null; then
    warn "Fish shell integration already installed in $DOTFILE"
  else
    info "Adding source command to $DOTFILE..."
    cat >> "$DOTFILE" <<'EOF'

# par-term-emu-core-rust shell integration
if test -f "$HOME/.par_term_emu_core_rust_shell_integration.fish"
  source "$HOME/.par_term_emu_core_rust_shell_integration.fish"
end
EOF
  fi

  info "Fish shell integration installed successfully!"
  info "To activate now, run: source $INSTALL_FILE"
}

function uninstall_shell() {
  info "Uninstalling shell integration..."

  # Remove integration files
  rm -f "${HOME}/.par_term_emu_core_rust_shell_integration.bash"
  rm -f "${HOME}/.par_term_emu_core_rust_shell_integration.zsh"
  rm -f "${HOME}/.par_term_emu_core_rust_shell_integration.fish"
  rm -f "${ZDOTDIR:-$HOME}/.par_term_emu_core_rust_shell_integration.zsh"

  info "Shell integration files removed."
  warn "Note: Source lines in your shell RC files were not removed."
  warn "Please manually remove lines containing 'par_term_emu_core_rust_shell_integration' from:"
  warn "  - ~/.bashrc or ~/.bash_profile"
  warn "  - ~/.zshrc"
  warn "  - ~/.config/fish/config.fish"
}

# Parse arguments
if [ "$1" = "--uninstall" ]; then
  uninstall_shell
  exit 0
fi

if [ "$1" = "--all" ]; then
  # Install for all available shells
  info "Installing for all supported shells..."

  if command -v bash > /dev/null 2>&1; then
    install_bash
    echo
  fi

  if command -v zsh > /dev/null 2>&1; then
    install_zsh
    echo
  fi

  if command -v fish > /dev/null 2>&1; then
    install_fish
    echo
  fi

  info "All available shells configured!"
  exit 0
fi

# Install for specific shell or current shell
SHELL_NAME="${1:-$(basename "$SHELL")}"

case "$SHELL_NAME" in
  bash)
    install_bash
    ;;
  zsh)
    install_zsh
    ;;
  fish)
    install_fish
    ;;
  *)
    die "Unsupported shell: $SHELL_NAME. Supported: bash, zsh, fish"
    ;;
esac

echo
info "Installation complete!"
info "Restart your shell or source the integration script to activate."
