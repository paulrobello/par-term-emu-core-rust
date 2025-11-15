# par-term-emu Shell Integration

Shell integration provides enhanced terminal features by embedding semantic markers in your shell's prompt and command output. This allows the terminal emulator to understand the structure of your shell session.

## Features

- **Prompt Navigation**: Jump between command prompts with keyboard shortcuts
- **Command Status Tracking**: Visual indicators for successful/failed commands
- **Working Directory Tracking**: Terminal knows current directory for tab titles, new tabs, etc.
- **Smart Selection**: Double-click to select a command, triple-click for output
- **Command Duration**: Measure and display how long commands take to execute
- **Custom Variables**: Set user-defined variables accessible to the terminal

## Quick Start

### Automatic Installation

Run the installer to automatically detect and configure your shell:

```bash
cd shell_integration
./install.sh
```

### Manual Installation

#### Bash

Add to `~/.bashrc` or `~/.bash_profile`:

```bash
if [ -f "$HOME/.par_term_emu_shell_integration.bash" ]; then
  source "$HOME/.par_term_emu_shell_integration.bash"
fi
```

Then copy the integration script:

```bash
cp par_term_emu_shell_integration.bash ~/.par_term_emu_shell_integration.bash
source ~/.par_term_emu_shell_integration.bash
```

#### Zsh

Add to `~/.zshrc`:

```zsh
if [ -f "${ZDOTDIR:-$HOME}/.par_term_emu_shell_integration.zsh" ]; then
  source "${ZDOTDIR:-$HOME}/.par_term_emu_shell_integration.zsh"
fi
```

Then copy the integration script:

```bash
cp par_term_emu_shell_integration.zsh ~/.par_term_emu_shell_integration.zsh
source ~/.par_term_emu_shell_integration.zsh
```

#### Fish

Add to `~/.config/fish/config.fish`:

```fish
if test -f "$HOME/.par_term_emu_shell_integration.fish"
  source "$HOME/.par_term_emu_shell_integration.fish"
end
```

Then copy the integration script:

```bash
cp par_term_emu_shell_integration.fish ~/.par_term_emu_shell_integration.fish
source ~/.par_term_emu_shell_integration.fish
```

## Advanced Usage

### Custom User Variables

You can define custom variables that the terminal can access. This is useful for displaying information in status bars, badges, or custom prompt decorations.

#### Bash/Zsh

Define a `par_term_emu_print_user_vars` function:

```bash
# Add to your .bashrc or .zshrc
par_term_emu_print_user_vars() {
  # Set git branch
  local git_branch=$(git branch --show-current 2>/dev/null)
  if [ -n "$git_branch" ]; then
    par_term_emu_set_user_var "git_branch" "$git_branch"
  fi

  # Set kubernetes context
  local k8s_context=$(kubectl config current-context 2>/dev/null)
  if [ -n "$k8s_context" ]; then
    par_term_emu_set_user_var "k8s_context" "$k8s_context"
  fi

  # Set Python virtual environment
  if [ -n "$VIRTUAL_ENV" ]; then
    par_term_emu_set_user_var "venv" "$(basename $VIRTUAL_ENV)"
  fi
}
```

#### Fish

```fish
# Add to your config.fish
function par_term_emu_print_user_vars
  # Set git branch
  set -l git_branch (git branch --show-current 2>/dev/null)
  if test -n "$git_branch"
    par_term_emu_set_user_var git_branch $git_branch
  end

  # Set kubernetes context
  set -l k8s_context (kubectl config current-context 2>/dev/null)
  if test -n "$k8s_context"
    par_term_emu_set_user_var k8s_context $k8s_context
  end

  # Set Python virtual environment
  if set -q VIRTUAL_ENV
    par_term_emu_set_user_var venv (basename $VIRTUAL_ENV)
  end
end
```

### Dynamic PS1 Generation (Bash)

For complex prompts that need to be generated dynamically:

```bash
par_term_emu_generate_ps1() {
  # Your custom PS1 generation logic
  PS1='\u@\h:\w$ '
}
```

### Embedding Markers in Custom Prompts

If you have a custom prompt theme that doesn't work well with automatic injection, you can manually embed the markers:

#### Bash/Zsh

```bash
# In your PS1:
PS1='$(par_term_emu_prompt_mark)\u@\h:\w$(par_term_emu_prompt_suffix)$ '
```

#### Fish

```fish
# In your fish_prompt function:
function fish_prompt
    par_term_emu_prompt_mark
    echo -n (whoami)@(hostname):
    echo -n (prompt_pwd)
    par_term_emu_prompt_end
    echo -n '$ '
end
```

## Technical Details

### OSC 133 Protocol

Shell integration uses the OSC (Operating System Command) 133 protocol, which is a de facto standard also used by iTerm2, VSCode terminal, and others.

The protocol defines these semantic markers:

- `OSC 133 ; A ST` - **Prompt Start**: Marks the beginning of the prompt
- `OSC 133 ; B ST` - **Prompt End / Command Start**: Marks where user input begins
- `OSC 133 ; C ST` - **Command Executed**: Marks when command execution starts
- `OSC 133 ; D ; [exit_code] ST` - **Command Finished**: Marks command completion with exit code

Where `OSC` is `\033]` (or `\x1b]`) and `ST` is `\007` (or `\x1b\\`).

### Additional Sequences

- `OSC 7 ; file://host/path ST` - Sets current working directory
- `OSC 1337 ; SetUserVar=key=value ST` - Sets custom user variable (value is base64 encoded)
- `OSC 1337 ; ShellIntegrationVersion=N;shell=name ST` - Reports integration version

### Session Flow

Here's what happens during a typical shell session with integration:

```
[Prompt Start - OSC 133;A]
user@host:~/project$
[Prompt End - OSC 133;B]
ls -la
[Command Executed - OSC 133;C]
total 48
drwxr-xr-x  5 user user 4096 Nov 13 10:00 .
drwxr-xr-x 10 user user 4096 Nov 13 09:00 ..
-rw-r--r--  1 user user  123 Nov 13 10:00 file.txt
[Command Finished with code 0 - OSC 133;D;0]
[Directory update - OSC 7;file://hostname/home/user/project]
```

## Compatibility

### Shell Compatibility

- **Bash**: 3.2+ (tested on 4.0+, 5.0+)
- **Zsh**: 5.0+ (uses native precmd/preexec)
- **Fish**: 2.3+ (uses event system)

### Terminal Compatibility

Shell integration scripts work with:
- **par-term-emu** (this terminal emulator)
- **iTerm2** (macOS)
- **VSCode** integrated terminal
- **WezTerm**
- Any terminal that supports OSC 133 protocol

### tmux/screen

By default, shell integration is disabled inside tmux or screen sessions to avoid conflicts. To enable it in tmux:

```bash
export ITERM_ENABLE_SHELL_INTEGRATION_WITH_TMUX=1
```

## Troubleshooting

### Integration not working

1. **Check if sourced**: Run `type par_term_emu_prompt_mark` - should output a function definition
2. **Check PS1**: Run `echo "$PS1"` - should contain escape sequences like `\033]133;`
3. **Check terminal**: Verify your terminal emulator supports OSC 133

### Prompt looks wrong

If your prompt appears corrupted:

1. **Incompatible prompt theme**: Some themes don't work with automatic PS1 modification
2. **Solution**: Manually embed markers (see "Embedding Markers in Custom Prompts" above)

### bash-preexec conflicts

If you already use bash-preexec or have custom DEBUG traps:

1. Source shell integration **after** other prompt customizations
2. Add to `precmd_functions` or `preexec_functions` arrays instead of overwriting

### Performance issues

If shell integration causes slowdown:

1. **Optimize user vars**: Avoid expensive commands in `par_term_emu_print_user_vars`
2. **Cache values**: Only update user vars when needed (e.g., on directory change)

## Uninstallation

Run the installer with the `--uninstall` flag:

```bash
./install.sh --uninstall
```

This removes the integration scripts but leaves source lines in your RC files. Remove those manually:

```bash
# Remove from ~/.bashrc, ~/.zshrc, or ~/.config/fish/config.fish
# Lines containing: par_term_emu_shell_integration
```

## Examples

See `../examples/shell_integration.py` for a demonstration of how the terminal emulator processes these markers.

## References

- [OSC 133 Protocol Specification](https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md)
- [iTerm2 Shell Integration](https://iterm2.com/documentation-shell-integration.html)
- [bash-preexec](https://github.com/rcaloras/bash-preexec)
- [Fish Event System](https://fishshell.com/docs/current/cmds/emit.html)

## License

Shell integration scripts are licensed under GPL-2.0-or-later, same as the par-term-emu project.
