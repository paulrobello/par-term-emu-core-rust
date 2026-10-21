
use builtin;
use str;

set edit:completion:arg-completer[par-mux] = {|@words|
    fn spaces {|n|
        builtin:repeat $n ' ' | str:join ''
    }
    fn cand {|text desc|
        edit:complex-candidate $text &display=$text' '(spaces (- 14 (wcswidth $text)))$desc
    }
    var command = 'par-mux'
    for word $words[1..-1] {
        if (str:has-prefix $word '-') {
            break
        }
        set command = $command';'$word
    }
    var completions = [
        &'par-mux'= {
            cand --socket 'Bind an explicit socket path instead of the named default'
            cand --state-dir 'Override the platform state directory (D3.4) that persisted session trees are written under, instead of the OS-standard state/data dir'
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -c 'Client mode: send one control command to the daemon on this socket, print its reply body to stdout (one line per reply line), and exit. A `%error` reply prints to stderr and exits non-zero. Never starts a daemon. A flag, like --stop, so the positional NAME stays unambiguous'
            cand --cmd 'Client mode: send one control command to the daemon on this socket, print its reply body to stdout (one line per reply line), and exit. A `%error` reply prints to stderr and exits non-zero. Never starts a daemon. A flag, like --stop, so the positional NAME stays unambiguous'
            cand --skills-dir 'With --install-skill: write under this skills root instead of the Claude Code default (tests, other agent harnesses)'
            cand --print-completions 'Print the shell-completion script for SHELL (bash, zsh, fish, powershell, elvish) generated from this CLI, and exit. Never contacts a daemon'
            cand --stop 'Stop the daemon serving this socket cleanly (final state save, `%exit` to clients) and wait for it to exit. Flags rather than subcommands: the positional NAME would otherwise be ambiguous with a session named `stop`'
            cand --restart 'Stop the running daemon (as --stop), then start a fresh one on the same socket — it restores the tree the stop just saved. Use after rebuilding par-mux: clients attach to whatever daemon owns the socket, so an old daemon keeps serving old code until restarted'
            cand --foreground 'Serve the daemon in the foreground of this process instead of the default daemonize-to-background: the debugging and supervision escape, and the shape programmatic spawners use (they own and kill the child they spawned). Not a daemon_mode member on purpose: it composes with --restart (stop, then serve here)'
            cand --no-attach 'Daemon-only: daemonize to background as usual but skip the auto-attach of the invoking terminal (and the isatty fallback that skips attaching for scripted starts)'
            cand --list-servers 'Client mode: list every live par-mux server this user can reach — registry entries plus the named-default sockets on disk — one line per server (`<name-or-path>  sessions=N  stamp`), pruning registry entries whose probe failed. Never starts a daemon. Honors `--state-dir`, so tests can point the registry at a fixture; with `--socket`/NAME, the named/default resolution is skipped for the target socket (discovery is user-wide, not per-socket)'
            cand --pane-endpoints 'Give each pane its own hook-only socket endpoint: `PAR_MUX_SOCKET` in a pane then names a socket that accepts ONLY hook reports for that pane, so its child processes cannot drive other panes or stop the server. Default off — the in-pane full-socket fallback is a core feature; see docs/MUX.md for the trade and the default-flip gate'
            cand --expose-control-socket 'With --pane-endpoints: also export `PAR_MUX_CONTROL_SOCKET` (the full control socket) in every pane, so agent-driven pane control keeps working. A session env of `PAR_MUX_CONTROL=1` grants the same per session without this flag'
            cand --gen-config 'Write the config file with the CURRENT EFFECTIVE settings (file if present > env > built-in defaults; startup CLI flags are not introspectable after parse, so a flagged value that also appears here wins only where this code path sees the flag — the socket and the daemon bools). Never overwrites without --force. With no config file yet, this is how a user starts one'
            cand --force 'With --gen-config: overwrite an existing config file'
            cand --install-skill 'Install the bundled par-mux agent skill into the Claude Code skills directory and exit. Never starts or contacts a daemon, so it runs anywhere, including inside a pane; par-term invokes it after installing par-mux so the skill matches the shipped binary'
            cand --install-agent-hooks 'Install the agent-state session hooks for claude, codex, grok, muse, droid, copilot, and cursor (script assets plus the config entries that run them) and exit. Never starts or contacts a daemon; par-term invokes it instead of embedding its own copies of the integrations'
            cand --uninstall-agent-hooks 'Remove the agent-state session hooks par-mux installed: the config entries first, then the script assets. Never contacts a daemon'
            cand --agent-hooks-status 'Print one line per agent — `<agent> installed` or `<agent> absent` — and exit: the read-only state behind the par-term Settings rows'
            cand --integration-status 'Print one line per integration — the agent hooks, the pi/omp extensions, and the par-mux skill — naming each one''s version and whether the installed asset matches the bundled copy (`v2`, `v1 → v2 outdated`, `asset missing`, or `absent`), and exit'
            cand --install-agent-extensions 'Install the pi and omp agent-state extensions (a file drop into each agent extension directory) and exit. Never contacts a daemon'
            cand --uninstall-agent-extensions 'Remove the pi and omp agent-state extensions par-mux installed'
            cand --heal-agent-hooks 'Rewrite a registered hook script asset that went missing (the hooks directory has been deleted externally while the agent config entries survived) without touching any agent config — the startup self-heal par-term runs. Never contacts a daemon'
            cand --print-schema 'Print the versioned JSON command schema (every control command, its flags, and its feature tokens) bundled with this binary, and exit. Never contacts a daemon; `list-commands -J` serves the same document from a running one'
            cand -h 'Print help'
            cand --help 'Print help'
            cand -V 'Print version'
            cand --version 'Print version'
            cand start 'Start the daemon (the default action): detached, auto-attaching this terminal unless --no-attach. Same as bare `par-mux [NAME]`'
            cand stop 'Stop the daemon serving the resolved socket cleanly. Same as `--stop`'
            cand restart 'Stop, then start a fresh daemon that restores the saved tree. Same as `--restart`'
            cand status 'One-shot orientation: the daemon''s build stamp, its workspaces and sessions, the displayed session, and the pane count — the orient step in one invocation (R2)'
            cand ls 'List the resolved daemon''s sessions (`list-sessions`, tmux''s `ls`)'
            cand attach 'Attach a client to a running daemon; never starts one. The feature-gated interactive client'
            cand config 'Write the config file with the current effective settings. Same as `--gen-config` (add `--force` to overwrite)'
            cand observe 'Stream one pane''s raw output to stdout, read-only (`observe-pane`): the screen seed, then live PTY bytes until the pane closes. Any number of observers may watch the same pane'
            cand control 'Drive one pane (`control-pane`): stream its output to stdout like `observe`, forward stdin bytes to it until EOF (no raw mode — for pipes and scripts; interactive use is `attach -t %N`), and report this terminal''s size (polled) or `--size` as a viewer of the pane''s window'
            cand completion 'Print the shell-completion script for SHELL, herdr''s spelling of `--print-completions`. Never contacts a daemon'
            cand machine 'Manage saved machines (`[machines.<label>]` in the config): SSH thin-client profiles for `--machine`. Never contacts a daemon'
            cand plugin 'Manage plugins: manifests under the state dir''s `plugins/` whose hooks run on daemon events. Install previews every command; nothing runs until `--trust`. Never contacts a daemon'
            cand update 'Check for a newer release and install it (self-update): resolve the channel, download the platform archive, verify the binary, replace this one, restart the daemon (its sessions ride the stop-and-restore handoff)'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'par-mux;start'= {
            cand --socket 'Bind an explicit socket path instead of the named default'
            cand --state-dir 'Override the platform state directory'
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand --foreground 'Serve in this process instead of detaching'
            cand --no-attach 'Skip the auto-attach of the invoking terminal'
            cand --pane-endpoints 'Give each pane its own hook-only socket endpoint'
            cand --expose-control-socket 'With --pane-endpoints: also export the full control socket'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;stop'= {
            cand --socket 'Connect to an explicit socket path instead of the named default'
            cand --state-dir 'Override the platform state directory (the registry base for discovery-flavored reads)'
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;restart'= {
            cand --socket 'Connect to an explicit socket path instead of the named default'
            cand --state-dir 'Override the platform state directory (the registry base for discovery-flavored reads)'
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;status'= {
            cand --socket 'Connect to an explicit socket path instead of the named default'
            cand --state-dir 'Override the platform state directory (the registry base for discovery-flavored reads)'
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;ls'= {
            cand --socket 'Connect to an explicit socket path instead of the named default'
            cand --state-dir 'Override the platform state directory (the registry base for discovery-flavored reads)'
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;attach'= {
            cand -t 'Initial target (session/window/pane, tmux-style — resolves like a send-keys -t target); Phase A''s view attaches to it'
            cand --prefix 'Detach prefix key, tmux spelling (e.g. `C-b`); Phase A routes it'
            cand --mode 'Render pipeline: `render` (the default — the Phase B pane renderer with the input router: mode-aware key re-encode, mouse routing, wheel scrollback) or `passthrough` (pane bytes to the host terminal verbatim, Phase A). Absent = the `[client] mode` from the config file, else render'
            cand --socket 'Connect to an explicit socket path instead of the named default'
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;config'= {
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand --force 'Overwrite an existing config file'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;observe'= {
            cand -t 'The pane (`%N` or a pane title)'
            cand --size '`control` only: report this fixed size (`WxH`) instead of following the terminal''s'
            cand --socket 'Connect to an explicit socket path instead of the named default'
            cand --state-dir 'Override the platform state directory (the registry base for discovery-flavored reads)'
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand --no-seed 'Skip the screen seed: stdout carries only output that follows'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;control'= {
            cand -t 'The pane (`%N` or a pane title)'
            cand --size '`control` only: report this fixed size (`WxH`) instead of following the terminal''s'
            cand --socket 'Connect to an explicit socket path instead of the named default'
            cand --state-dir 'Override the platform state directory (the registry base for discovery-flavored reads)'
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand --no-seed 'Skip the screen seed: stdout carries only output that follows'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;completion'= {
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;machine'= {
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
            cand add 'Save `[machines.<LABEL>]` (or replace it): the ssh target, an optional default remote session, enabled by default. Nothing is contacted — the first attach relays whenever ssh is ready'
            cand list 'One line per saved machine: `label  target  [session]  enabled|disabled`'
            cand remove 'Remove a machine''s profile (remote sessions keep running)'
            cand rename 'Rename a machine''s label'
            cand enable 'Mark a machine enabled'
            cand disable 'Mark a machine disabled (listed, never auto-attached)'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'par-mux;machine;add'= {
            cand --label 'The machine''s label (defaults to the target''s host part)'
            cand --session 'The remote session `--machine <LABEL> attach` lands on'
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;machine;list'= {
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;machine;remove'= {
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;machine;rename'= {
            cand --to 'to'
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;machine;enable'= {
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;machine;disable'= {
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;machine;help'= {
            cand add 'Save `[machines.<LABEL>]` (or replace it): the ssh target, an optional default remote session, enabled by default. Nothing is contacted — the first attach relays whenever ssh is ready'
            cand list 'One line per saved machine: `label  target  [session]  enabled|disabled`'
            cand remove 'Remove a machine''s profile (remote sessions keep running)'
            cand rename 'Rename a machine''s label'
            cand enable 'Mark a machine enabled'
            cand disable 'Mark a machine disabled (listed, never auto-attached)'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'par-mux;machine;help;add'= {
        }
        &'par-mux;machine;help;list'= {
        }
        &'par-mux;machine;help;remove'= {
        }
        &'par-mux;machine;help;rename'= {
        }
        &'par-mux;machine;help;enable'= {
        }
        &'par-mux;machine;help;disable'= {
        }
        &'par-mux;machine;help;help'= {
        }
        &'par-mux;plugin'= {
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
            cand install 'Resolve a plugin source (a directory or git URL) and PREVIEW it: the source, its identity, and every command an event would run. Installs only with `--trust`'
            cand list 'One line per installed plugin: name, version, hook count, source'
            cand remove 'Remove a plugin (its hooks stop after the daemon reloads)'
            cand update 'Re-fetch a plugin from its recorded source (the preview and `--trust` apply again)'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'par-mux;plugin;install'= {
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand --trust 'Trust the previewed commands and install'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;plugin;list'= {
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;plugin;remove'= {
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;plugin;update'= {
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand --trust 'trust'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;plugin;help'= {
            cand install 'Resolve a plugin source (a directory or git URL) and PREVIEW it: the source, its identity, and every command an event would run. Installs only with `--trust`'
            cand list 'One line per installed plugin: name, version, hook count, source'
            cand remove 'Remove a plugin (its hooks stop after the daemon reloads)'
            cand update 'Re-fetch a plugin from its recorded source (the preview and `--trust` apply again)'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'par-mux;plugin;help;install'= {
        }
        &'par-mux;plugin;help;list'= {
        }
        &'par-mux;plugin;help;remove'= {
        }
        &'par-mux;plugin;help;update'= {
        }
        &'par-mux;plugin;help;help'= {
        }
        &'par-mux;update'= {
            cand --channel 'The release channel: `stable` (latest release, the default) or `preview` (newest prerelease)'
            cand --machine 'Relay this invocation to a saved machine over SSH (the machine''s profile in `[machines.<label>]` names the ssh target; credentials stay with ssh itself). One-shot commands (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run non-interactively; `attach` forces passthrough under a pty — the thin client'
            cand --check 'Report what would install, touch nothing'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'par-mux;help'= {
            cand start 'Start the daemon (the default action): detached, auto-attaching this terminal unless --no-attach. Same as bare `par-mux [NAME]`'
            cand stop 'Stop the daemon serving the resolved socket cleanly. Same as `--stop`'
            cand restart 'Stop, then start a fresh daemon that restores the saved tree. Same as `--restart`'
            cand status 'One-shot orientation: the daemon''s build stamp, its workspaces and sessions, the displayed session, and the pane count — the orient step in one invocation (R2)'
            cand ls 'List the resolved daemon''s sessions (`list-sessions`, tmux''s `ls`)'
            cand attach 'Attach a client to a running daemon; never starts one. The feature-gated interactive client'
            cand config 'Write the config file with the current effective settings. Same as `--gen-config` (add `--force` to overwrite)'
            cand observe 'Stream one pane''s raw output to stdout, read-only (`observe-pane`): the screen seed, then live PTY bytes until the pane closes. Any number of observers may watch the same pane'
            cand control 'Drive one pane (`control-pane`): stream its output to stdout like `observe`, forward stdin bytes to it until EOF (no raw mode — for pipes and scripts; interactive use is `attach -t %N`), and report this terminal''s size (polled) or `--size` as a viewer of the pane''s window'
            cand completion 'Print the shell-completion script for SHELL, herdr''s spelling of `--print-completions`. Never contacts a daemon'
            cand machine 'Manage saved machines (`[machines.<label>]` in the config): SSH thin-client profiles for `--machine`. Never contacts a daemon'
            cand plugin 'Manage plugins: manifests under the state dir''s `plugins/` whose hooks run on daemon events. Install previews every command; nothing runs until `--trust`. Never contacts a daemon'
            cand update 'Check for a newer release and install it (self-update): resolve the channel, download the platform archive, verify the binary, replace this one, restart the daemon (its sessions ride the stop-and-restore handoff)'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'par-mux;help;start'= {
        }
        &'par-mux;help;stop'= {
        }
        &'par-mux;help;restart'= {
        }
        &'par-mux;help;status'= {
        }
        &'par-mux;help;ls'= {
        }
        &'par-mux;help;attach'= {
        }
        &'par-mux;help;config'= {
        }
        &'par-mux;help;observe'= {
        }
        &'par-mux;help;control'= {
        }
        &'par-mux;help;completion'= {
        }
        &'par-mux;help;machine'= {
            cand add 'Save `[machines.<LABEL>]` (or replace it): the ssh target, an optional default remote session, enabled by default. Nothing is contacted — the first attach relays whenever ssh is ready'
            cand list 'One line per saved machine: `label  target  [session]  enabled|disabled`'
            cand remove 'Remove a machine''s profile (remote sessions keep running)'
            cand rename 'Rename a machine''s label'
            cand enable 'Mark a machine enabled'
            cand disable 'Mark a machine disabled (listed, never auto-attached)'
        }
        &'par-mux;help;machine;add'= {
        }
        &'par-mux;help;machine;list'= {
        }
        &'par-mux;help;machine;remove'= {
        }
        &'par-mux;help;machine;rename'= {
        }
        &'par-mux;help;machine;enable'= {
        }
        &'par-mux;help;machine;disable'= {
        }
        &'par-mux;help;plugin'= {
            cand install 'Resolve a plugin source (a directory or git URL) and PREVIEW it: the source, its identity, and every command an event would run. Installs only with `--trust`'
            cand list 'One line per installed plugin: name, version, hook count, source'
            cand remove 'Remove a plugin (its hooks stop after the daemon reloads)'
            cand update 'Re-fetch a plugin from its recorded source (the preview and `--trust` apply again)'
        }
        &'par-mux;help;plugin;install'= {
        }
        &'par-mux;help;plugin;list'= {
        }
        &'par-mux;help;plugin;remove'= {
        }
        &'par-mux;help;plugin;update'= {
        }
        &'par-mux;help;update'= {
        }
        &'par-mux;help;help'= {
        }
    ]
    $completions[$command]
}
