
using namespace System.Management.Automation
using namespace System.Management.Automation.Language

Register-ArgumentCompleter -Native -CommandName 'par-mux' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $commandElements = $commandAst.CommandElements
    $command = @(
        'par-mux'
        for ($i = 1; $i -lt $commandElements.Count; $i++) {
            $element = $commandElements[$i]
            if ($element -isnot [StringConstantExpressionAst] -or
                $element.StringConstantType -ne [StringConstantType]::BareWord -or
                $element.Value.StartsWith('-') -or
                $element.Value -eq $wordToComplete) {
                break
        }
        $element.Value
    }) -join ';'

    $completions = @(switch ($command) {
        'par-mux' {
            [CompletionResult]::new('--socket', '--socket', [CompletionResultType]::ParameterName, 'Bind an explicit socket path instead of the named default')
            [CompletionResult]::new('--state-dir', '--state-dir', [CompletionResultType]::ParameterName, 'Override the platform state directory (D3.4) that persisted session trees are written under, instead of the OS-standard state/data dir')
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-c', '-c', [CompletionResultType]::ParameterName, 'Client mode: send one control command to the daemon on this socket, print its reply body to stdout (one line per reply line), and exit. A `%error` reply prints to stderr and exits non-zero. Never starts a daemon. A flag, like --stop, so the positional NAME stays unambiguous')
            [CompletionResult]::new('--cmd', '--cmd', [CompletionResultType]::ParameterName, 'Client mode: send one control command to the daemon on this socket, print its reply body to stdout (one line per reply line), and exit. A `%error` reply prints to stderr and exits non-zero. Never starts a daemon. A flag, like --stop, so the positional NAME stays unambiguous')
            [CompletionResult]::new('--skills-dir', '--skills-dir', [CompletionResultType]::ParameterName, 'With --install-skill: write under this skills root instead of the Claude Code default (tests, other agent harnesses)')
            [CompletionResult]::new('--print-completions', '--print-completions', [CompletionResultType]::ParameterName, 'Print the shell-completion script for SHELL (bash, zsh, fish, powershell, elvish) generated from this CLI, and exit. Never contacts a daemon')
            [CompletionResult]::new('--stop', '--stop', [CompletionResultType]::ParameterName, 'Stop the daemon serving this socket cleanly (final state save, `%exit` to clients) and wait for it to exit. Flags rather than subcommands: the positional NAME would otherwise be ambiguous with a session named `stop`')
            [CompletionResult]::new('--restart', '--restart', [CompletionResultType]::ParameterName, 'Stop the running daemon (as --stop), then start a fresh one on the same socket — it restores the tree the stop just saved. Use after rebuilding par-mux: clients attach to whatever daemon owns the socket, so an old daemon keeps serving old code until restarted')
            [CompletionResult]::new('--foreground', '--foreground', [CompletionResultType]::ParameterName, 'Serve the daemon in the foreground of this process instead of the default daemonize-to-background: the debugging and supervision escape, and the shape programmatic spawners use (they own and kill the child they spawned). Not a daemon_mode member on purpose: it composes with --restart (stop, then serve here)')
            [CompletionResult]::new('--no-attach', '--no-attach', [CompletionResultType]::ParameterName, 'Daemon-only: daemonize to background as usual but skip the auto-attach of the invoking terminal (and the isatty fallback that skips attaching for scripted starts)')
            [CompletionResult]::new('--list-servers', '--list-servers', [CompletionResultType]::ParameterName, 'Client mode: list every live par-mux server this user can reach — registry entries plus the named-default sockets on disk — one line per server (`<name-or-path>  sessions=N  stamp`), pruning registry entries whose probe failed. Never starts a daemon. Honors `--state-dir`, so tests can point the registry at a fixture; with `--socket`/NAME, the named/default resolution is skipped for the target socket (discovery is user-wide, not per-socket)')
            [CompletionResult]::new('--pane-endpoints', '--pane-endpoints', [CompletionResultType]::ParameterName, 'Give each pane its own hook-only socket endpoint: `PAR_MUX_SOCKET` in a pane then names a socket that accepts ONLY hook reports for that pane, so its child processes cannot drive other panes or stop the server. Default off — the in-pane full-socket fallback is a core feature; see docs/MUX.md for the trade and the default-flip gate')
            [CompletionResult]::new('--expose-control-socket', '--expose-control-socket', [CompletionResultType]::ParameterName, 'With --pane-endpoints: also export `PAR_MUX_CONTROL_SOCKET` (the full control socket) in every pane, so agent-driven pane control keeps working. A session env of `PAR_MUX_CONTROL=1` grants the same per session without this flag')
            [CompletionResult]::new('--gen-config', '--gen-config', [CompletionResultType]::ParameterName, 'Write the config file with the CURRENT EFFECTIVE settings (file if present > env > built-in defaults; startup CLI flags are not introspectable after parse, so a flagged value that also appears here wins only where this code path sees the flag — the socket and the daemon bools). Never overwrites without --force. With no config file yet, this is how a user starts one')
            [CompletionResult]::new('--force', '--force', [CompletionResultType]::ParameterName, 'With --gen-config: overwrite an existing config file')
            [CompletionResult]::new('--install-skill', '--install-skill', [CompletionResultType]::ParameterName, 'Install the bundled par-mux agent skill into the Claude Code skills directory and exit. Never starts or contacts a daemon, so it runs anywhere, including inside a pane; par-term invokes it after installing par-mux so the skill matches the shipped binary')
            [CompletionResult]::new('--install-agent-hooks', '--install-agent-hooks', [CompletionResultType]::ParameterName, 'Install the agent-state session hooks for claude, codex, grok, muse, droid, copilot, and cursor (script assets plus the config entries that run them) and exit. Never starts or contacts a daemon; par-term invokes it instead of embedding its own copies of the integrations')
            [CompletionResult]::new('--uninstall-agent-hooks', '--uninstall-agent-hooks', [CompletionResultType]::ParameterName, 'Remove the agent-state session hooks par-mux installed: the config entries first, then the script assets. Never contacts a daemon')
            [CompletionResult]::new('--agent-hooks-status', '--agent-hooks-status', [CompletionResultType]::ParameterName, 'Print one line per agent — `<agent> installed` or `<agent> absent` — and exit: the read-only state behind the par-term Settings rows')
            [CompletionResult]::new('--integration-status', '--integration-status', [CompletionResultType]::ParameterName, 'Print one line per integration — the agent hooks, the pi/omp extensions, and the par-mux skill — naming each one''s version and whether the installed asset matches the bundled copy (`v2`, `v1 → v2 outdated`, `asset missing`, or `absent`), and exit')
            [CompletionResult]::new('--install-agent-extensions', '--install-agent-extensions', [CompletionResultType]::ParameterName, 'Install the pi and omp agent-state extensions (a file drop into each agent extension directory) and exit. Never contacts a daemon')
            [CompletionResult]::new('--uninstall-agent-extensions', '--uninstall-agent-extensions', [CompletionResultType]::ParameterName, 'Remove the pi and omp agent-state extensions par-mux installed')
            [CompletionResult]::new('--heal-agent-hooks', '--heal-agent-hooks', [CompletionResultType]::ParameterName, 'Rewrite a registered hook script asset that went missing (the hooks directory has been deleted externally while the agent config entries survived) without touching any agent config — the startup self-heal par-term runs. Never contacts a daemon')
            [CompletionResult]::new('--print-schema', '--print-schema', [CompletionResultType]::ParameterName, 'Print the versioned JSON command schema (every control command, its flags, and its feature tokens) bundled with this binary, and exit. Never contacts a daemon; `list-commands -J` serves the same document from a running one')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('-V', '-V ', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('--version', '--version', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('start', 'start', [CompletionResultType]::ParameterValue, 'Start the daemon (the default action): detached, auto-attaching this terminal unless --no-attach. Same as bare `par-mux [NAME]`')
            [CompletionResult]::new('stop', 'stop', [CompletionResultType]::ParameterValue, 'Stop the daemon serving the resolved socket cleanly. Same as `--stop`')
            [CompletionResult]::new('restart', 'restart', [CompletionResultType]::ParameterValue, 'Stop, then start a fresh daemon that restores the saved tree. Same as `--restart`')
            [CompletionResult]::new('status', 'status', [CompletionResultType]::ParameterValue, 'One-shot orientation: the daemon''s build stamp, its workspaces and sessions, the displayed session, and the pane count — the orient step in one invocation (R2)')
            [CompletionResult]::new('ls', 'ls', [CompletionResultType]::ParameterValue, 'List the resolved daemon''s sessions (`list-sessions`, tmux''s `ls`)')
            [CompletionResult]::new('attach', 'attach', [CompletionResultType]::ParameterValue, 'Attach a client to a running daemon; never starts one. The feature-gated interactive client')
            [CompletionResult]::new('config', 'config', [CompletionResultType]::ParameterValue, 'Write the config file with the current effective settings. Same as `--gen-config` (add `--force` to overwrite)')
            [CompletionResult]::new('observe', 'observe', [CompletionResultType]::ParameterValue, 'Stream one pane''s raw output to stdout, read-only (`observe-pane`): the screen seed, then live PTY bytes until the pane closes. Any number of observers may watch the same pane')
            [CompletionResult]::new('control', 'control', [CompletionResultType]::ParameterValue, 'Drive one pane (`control-pane`): stream its output to stdout like `observe`, forward stdin bytes to it until EOF (no raw mode — for pipes and scripts; interactive use is `attach -t %N`), and report this terminal''s size (polled) or `--size` as a viewer of the pane''s window')
            [CompletionResult]::new('completion', 'completion', [CompletionResultType]::ParameterValue, 'Print the shell-completion script for SHELL, herdr''s spelling of `--print-completions`. Never contacts a daemon')
            [CompletionResult]::new('machine', 'machine', [CompletionResultType]::ParameterValue, 'Manage saved machines (`[machines.<label>]` in the config): SSH thin-client profiles for `--machine`. Never contacts a daemon')
            [CompletionResult]::new('plugin', 'plugin', [CompletionResultType]::ParameterValue, 'Manage plugins: manifests under the state dir''s `plugins/` whose hooks run on daemon events. Install previews every command; nothing runs until `--trust`. Never contacts a daemon')
            [CompletionResult]::new('update', 'update', [CompletionResultType]::ParameterValue, 'Check for a newer release and install it (self-update): resolve the channel, download the platform archive, verify the binary, replace this one, restart the daemon (its sessions ride the stop-and-restore handoff)')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'par-mux;start' {
            [CompletionResult]::new('--socket', '--socket', [CompletionResultType]::ParameterName, 'Bind an explicit socket path instead of the named default')
            [CompletionResult]::new('--state-dir', '--state-dir', [CompletionResultType]::ParameterName, 'Override the platform state directory')
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('--foreground', '--foreground', [CompletionResultType]::ParameterName, 'Serve in this process instead of detaching')
            [CompletionResult]::new('--no-attach', '--no-attach', [CompletionResultType]::ParameterName, 'Skip the auto-attach of the invoking terminal')
            [CompletionResult]::new('--pane-endpoints', '--pane-endpoints', [CompletionResultType]::ParameterName, 'Give each pane its own hook-only socket endpoint')
            [CompletionResult]::new('--expose-control-socket', '--expose-control-socket', [CompletionResultType]::ParameterName, 'With --pane-endpoints: also export the full control socket')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;stop' {
            [CompletionResult]::new('--socket', '--socket', [CompletionResultType]::ParameterName, 'Connect to an explicit socket path instead of the named default')
            [CompletionResult]::new('--state-dir', '--state-dir', [CompletionResultType]::ParameterName, 'Override the platform state directory (the registry base for discovery-flavored reads)')
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;restart' {
            [CompletionResult]::new('--socket', '--socket', [CompletionResultType]::ParameterName, 'Connect to an explicit socket path instead of the named default')
            [CompletionResult]::new('--state-dir', '--state-dir', [CompletionResultType]::ParameterName, 'Override the platform state directory (the registry base for discovery-flavored reads)')
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;status' {
            [CompletionResult]::new('--socket', '--socket', [CompletionResultType]::ParameterName, 'Connect to an explicit socket path instead of the named default')
            [CompletionResult]::new('--state-dir', '--state-dir', [CompletionResultType]::ParameterName, 'Override the platform state directory (the registry base for discovery-flavored reads)')
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;ls' {
            [CompletionResult]::new('--socket', '--socket', [CompletionResultType]::ParameterName, 'Connect to an explicit socket path instead of the named default')
            [CompletionResult]::new('--state-dir', '--state-dir', [CompletionResultType]::ParameterName, 'Override the platform state directory (the registry base for discovery-flavored reads)')
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;attach' {
            [CompletionResult]::new('-t', '-t', [CompletionResultType]::ParameterName, 'Initial target (session/window/pane, tmux-style — resolves like a send-keys -t target); Phase A''s view attaches to it')
            [CompletionResult]::new('--prefix', '--prefix', [CompletionResultType]::ParameterName, 'Detach prefix key, tmux spelling (e.g. `C-b`); Phase A routes it')
            [CompletionResult]::new('--mode', '--mode', [CompletionResultType]::ParameterName, 'Render pipeline: `render` (the default — the Phase B pane renderer with the input router: mode-aware key re-encode, mouse routing, wheel scrollback) or `passthrough` (pane bytes to the host terminal verbatim, Phase A). Absent = the `[client] mode` from the config file, else render')
            [CompletionResult]::new('--socket', '--socket', [CompletionResultType]::ParameterName, 'Connect to an explicit socket path instead of the named default')
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;config' {
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('--force', '--force', [CompletionResultType]::ParameterName, 'Overwrite an existing config file')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;observe' {
            [CompletionResult]::new('-t', '-t', [CompletionResultType]::ParameterName, 'The pane (`%N` or a pane title)')
            [CompletionResult]::new('--size', '--size', [CompletionResultType]::ParameterName, '`control` only: report this fixed size (`WxH`) instead of following the terminal''s')
            [CompletionResult]::new('--socket', '--socket', [CompletionResultType]::ParameterName, 'Connect to an explicit socket path instead of the named default')
            [CompletionResult]::new('--state-dir', '--state-dir', [CompletionResultType]::ParameterName, 'Override the platform state directory (the registry base for discovery-flavored reads)')
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('--no-seed', '--no-seed', [CompletionResultType]::ParameterName, 'Skip the screen seed: stdout carries only output that follows')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;control' {
            [CompletionResult]::new('-t', '-t', [CompletionResultType]::ParameterName, 'The pane (`%N` or a pane title)')
            [CompletionResult]::new('--size', '--size', [CompletionResultType]::ParameterName, '`control` only: report this fixed size (`WxH`) instead of following the terminal''s')
            [CompletionResult]::new('--socket', '--socket', [CompletionResultType]::ParameterName, 'Connect to an explicit socket path instead of the named default')
            [CompletionResult]::new('--state-dir', '--state-dir', [CompletionResultType]::ParameterName, 'Override the platform state directory (the registry base for discovery-flavored reads)')
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('--no-seed', '--no-seed', [CompletionResultType]::ParameterName, 'Skip the screen seed: stdout carries only output that follows')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;completion' {
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;machine' {
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('add', 'add', [CompletionResultType]::ParameterValue, 'Save `[machines.<LABEL>]` (or replace it): the ssh target, an optional default remote session, enabled by default. Nothing is contacted — the first attach relays whenever ssh is ready')
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'One line per saved machine: `label  target  [session]  enabled|disabled`')
            [CompletionResult]::new('remove', 'remove', [CompletionResultType]::ParameterValue, 'Remove a machine''s profile (remote sessions keep running)')
            [CompletionResult]::new('rename', 'rename', [CompletionResultType]::ParameterValue, 'Rename a machine''s label')
            [CompletionResult]::new('enable', 'enable', [CompletionResultType]::ParameterValue, 'Mark a machine enabled')
            [CompletionResult]::new('disable', 'disable', [CompletionResultType]::ParameterValue, 'Mark a machine disabled (listed, never auto-attached)')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'par-mux;machine;add' {
            [CompletionResult]::new('--label', '--label', [CompletionResultType]::ParameterName, 'The machine''s label (defaults to the target''s host part)')
            [CompletionResult]::new('--session', '--session', [CompletionResultType]::ParameterName, 'The remote session `--machine <LABEL> attach` lands on')
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;machine;list' {
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;machine;remove' {
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;machine;rename' {
            [CompletionResult]::new('--to', '--to', [CompletionResultType]::ParameterName, 'to')
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;machine;enable' {
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;machine;disable' {
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;machine;help' {
            [CompletionResult]::new('add', 'add', [CompletionResultType]::ParameterValue, 'Save `[machines.<LABEL>]` (or replace it): the ssh target, an optional default remote session, enabled by default. Nothing is contacted — the first attach relays whenever ssh is ready')
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'One line per saved machine: `label  target  [session]  enabled|disabled`')
            [CompletionResult]::new('remove', 'remove', [CompletionResultType]::ParameterValue, 'Remove a machine''s profile (remote sessions keep running)')
            [CompletionResult]::new('rename', 'rename', [CompletionResultType]::ParameterValue, 'Rename a machine''s label')
            [CompletionResult]::new('enable', 'enable', [CompletionResultType]::ParameterValue, 'Mark a machine enabled')
            [CompletionResult]::new('disable', 'disable', [CompletionResultType]::ParameterValue, 'Mark a machine disabled (listed, never auto-attached)')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'par-mux;machine;help;add' {
            break
        }
        'par-mux;machine;help;list' {
            break
        }
        'par-mux;machine;help;remove' {
            break
        }
        'par-mux;machine;help;rename' {
            break
        }
        'par-mux;machine;help;enable' {
            break
        }
        'par-mux;machine;help;disable' {
            break
        }
        'par-mux;machine;help;help' {
            break
        }
        'par-mux;plugin' {
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('install', 'install', [CompletionResultType]::ParameterValue, 'Resolve a plugin source (a directory or git URL) and PREVIEW it: the source, its identity, and every command an event would run. Installs only with `--trust`')
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'One line per installed plugin: name, version, hook count, source')
            [CompletionResult]::new('remove', 'remove', [CompletionResultType]::ParameterValue, 'Remove a plugin (its hooks stop after the daemon reloads)')
            [CompletionResult]::new('update', 'update', [CompletionResultType]::ParameterValue, 'Re-fetch a plugin from its recorded source (the preview and `--trust` apply again)')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'par-mux;plugin;install' {
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('--trust', '--trust', [CompletionResultType]::ParameterName, 'Trust the previewed commands and install')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;plugin;list' {
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;plugin;remove' {
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;plugin;update' {
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('--trust', '--trust', [CompletionResultType]::ParameterName, 'trust')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'par-mux;plugin;help' {
            [CompletionResult]::new('install', 'install', [CompletionResultType]::ParameterValue, 'Resolve a plugin source (a directory or git URL) and PREVIEW it: the source, its identity, and every command an event would run. Installs only with `--trust`')
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'One line per installed plugin: name, version, hook count, source')
            [CompletionResult]::new('remove', 'remove', [CompletionResultType]::ParameterValue, 'Remove a plugin (its hooks stop after the daemon reloads)')
            [CompletionResult]::new('update', 'update', [CompletionResultType]::ParameterValue, 'Re-fetch a plugin from its recorded source (the preview and `--trust` apply again)')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'par-mux;plugin;help;install' {
            break
        }
        'par-mux;plugin;help;list' {
            break
        }
        'par-mux;plugin;help;remove' {
            break
        }
        'par-mux;plugin;help;update' {
            break
        }
        'par-mux;plugin;help;help' {
            break
        }
        'par-mux;update' {
            [CompletionResult]::new('--channel', '--channel', [CompletionResultType]::ParameterName, 'The release channel: `stable` (latest release, the default) or `preview` (newest prerelease)')
            [CompletionResult]::new('--machine', '--machine', [CompletionResultType]::ParameterName, 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client')
            [CompletionResult]::new('--check', '--check', [CompletionResultType]::ParameterName, 'Report what would install, touch nothing')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'par-mux;help' {
            [CompletionResult]::new('start', 'start', [CompletionResultType]::ParameterValue, 'Start the daemon (the default action): detached, auto-attaching this terminal unless --no-attach. Same as bare `par-mux [NAME]`')
            [CompletionResult]::new('stop', 'stop', [CompletionResultType]::ParameterValue, 'Stop the daemon serving the resolved socket cleanly. Same as `--stop`')
            [CompletionResult]::new('restart', 'restart', [CompletionResultType]::ParameterValue, 'Stop, then start a fresh daemon that restores the saved tree. Same as `--restart`')
            [CompletionResult]::new('status', 'status', [CompletionResultType]::ParameterValue, 'One-shot orientation: the daemon''s build stamp, its workspaces and sessions, the displayed session, and the pane count — the orient step in one invocation (R2)')
            [CompletionResult]::new('ls', 'ls', [CompletionResultType]::ParameterValue, 'List the resolved daemon''s sessions (`list-sessions`, tmux''s `ls`)')
            [CompletionResult]::new('attach', 'attach', [CompletionResultType]::ParameterValue, 'Attach a client to a running daemon; never starts one. The feature-gated interactive client')
            [CompletionResult]::new('config', 'config', [CompletionResultType]::ParameterValue, 'Write the config file with the current effective settings. Same as `--gen-config` (add `--force` to overwrite)')
            [CompletionResult]::new('observe', 'observe', [CompletionResultType]::ParameterValue, 'Stream one pane''s raw output to stdout, read-only (`observe-pane`): the screen seed, then live PTY bytes until the pane closes. Any number of observers may watch the same pane')
            [CompletionResult]::new('control', 'control', [CompletionResultType]::ParameterValue, 'Drive one pane (`control-pane`): stream its output to stdout like `observe`, forward stdin bytes to it until EOF (no raw mode — for pipes and scripts; interactive use is `attach -t %N`), and report this terminal''s size (polled) or `--size` as a viewer of the pane''s window')
            [CompletionResult]::new('completion', 'completion', [CompletionResultType]::ParameterValue, 'Print the shell-completion script for SHELL, herdr''s spelling of `--print-completions`. Never contacts a daemon')
            [CompletionResult]::new('machine', 'machine', [CompletionResultType]::ParameterValue, 'Manage saved machines (`[machines.<label>]` in the config): SSH thin-client profiles for `--machine`. Never contacts a daemon')
            [CompletionResult]::new('plugin', 'plugin', [CompletionResultType]::ParameterValue, 'Manage plugins: manifests under the state dir''s `plugins/` whose hooks run on daemon events. Install previews every command; nothing runs until `--trust`. Never contacts a daemon')
            [CompletionResult]::new('update', 'update', [CompletionResultType]::ParameterValue, 'Check for a newer release and install it (self-update): resolve the channel, download the platform archive, verify the binary, replace this one, restart the daemon (its sessions ride the stop-and-restore handoff)')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'par-mux;help;start' {
            break
        }
        'par-mux;help;stop' {
            break
        }
        'par-mux;help;restart' {
            break
        }
        'par-mux;help;status' {
            break
        }
        'par-mux;help;ls' {
            break
        }
        'par-mux;help;attach' {
            break
        }
        'par-mux;help;config' {
            break
        }
        'par-mux;help;observe' {
            break
        }
        'par-mux;help;control' {
            break
        }
        'par-mux;help;completion' {
            break
        }
        'par-mux;help;machine' {
            [CompletionResult]::new('add', 'add', [CompletionResultType]::ParameterValue, 'Save `[machines.<LABEL>]` (or replace it): the ssh target, an optional default remote session, enabled by default. Nothing is contacted — the first attach relays whenever ssh is ready')
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'One line per saved machine: `label  target  [session]  enabled|disabled`')
            [CompletionResult]::new('remove', 'remove', [CompletionResultType]::ParameterValue, 'Remove a machine''s profile (remote sessions keep running)')
            [CompletionResult]::new('rename', 'rename', [CompletionResultType]::ParameterValue, 'Rename a machine''s label')
            [CompletionResult]::new('enable', 'enable', [CompletionResultType]::ParameterValue, 'Mark a machine enabled')
            [CompletionResult]::new('disable', 'disable', [CompletionResultType]::ParameterValue, 'Mark a machine disabled (listed, never auto-attached)')
            break
        }
        'par-mux;help;machine;add' {
            break
        }
        'par-mux;help;machine;list' {
            break
        }
        'par-mux;help;machine;remove' {
            break
        }
        'par-mux;help;machine;rename' {
            break
        }
        'par-mux;help;machine;enable' {
            break
        }
        'par-mux;help;machine;disable' {
            break
        }
        'par-mux;help;plugin' {
            [CompletionResult]::new('install', 'install', [CompletionResultType]::ParameterValue, 'Resolve a plugin source (a directory or git URL) and PREVIEW it: the source, its identity, and every command an event would run. Installs only with `--trust`')
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'One line per installed plugin: name, version, hook count, source')
            [CompletionResult]::new('remove', 'remove', [CompletionResultType]::ParameterValue, 'Remove a plugin (its hooks stop after the daemon reloads)')
            [CompletionResult]::new('update', 'update', [CompletionResultType]::ParameterValue, 'Re-fetch a plugin from its recorded source (the preview and `--trust` apply again)')
            break
        }
        'par-mux;help;plugin;install' {
            break
        }
        'par-mux;help;plugin;list' {
            break
        }
        'par-mux;help;plugin;remove' {
            break
        }
        'par-mux;help;plugin;update' {
            break
        }
        'par-mux;help;update' {
            break
        }
        'par-mux;help;help' {
            break
        }
    })

    $completions.Where{ $_.CompletionText -like "$wordToComplete*" } |
        Sort-Object -Property ListItemText
}
