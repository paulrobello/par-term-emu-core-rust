# par-mux codex session hook (Windows) — reports session identity to the
# par-mux daemon on codex SessionStart events, and with codex's claude-wire
# hook events also keeps a live session registered and stateful between
# those: UserPromptSubmit → `prompt` (working), PermissionRequest →
# `question`, Stop → `stop` (idle — the idle-at-prompt capture this
# integration previously lacked: the roster lists only panes WITH state,
# and a codex pane used to fire no state event after SessionStart, card
# 01a19ec3), and releases the pane's claim on SessionEnd (`release`).
# PowerShell port of the POSIX reporter (par-mux-codex-session-hook.sh):
# one JSON line over the control socket, one reply read, close. Inert
# outside a par-mux pane (env guards) and silent on every failure.
# PAR_MUX_INTEGRATION_ID=codex
# PAR_MUX_INTEGRATION_VERSION=3

param([string]$Action = "")

if ($Action -notin @("session", "prompt", "question", "stop", "release")) { exit 0 }
if ($env:PAR_MUX_ENV -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:PAR_MUX_SOCKET)) { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:PAR_MUX_PANE_ID)) { exit 0 }

# Transport mirrors the daemon's platform split (par-term-emu-core-rust
# mux ipc.rs): named pipes on Windows - the socket path string becomes the
# pipe name after \\.\pipe\, exactly as the interprocess crate derives it -
# and Unix-domain sockets elsewhere (pwsh 7 always has
# UnixDomainSocketEndPoint; .NET Framework, which lacks it, only runs on
# Windows, whose arm this is). One JSON line out, one reply read, close.
function Send-MuxReport([string]$Path, [byte[]]$Bytes) {
    if ($env:OS -eq "Windows_NT") {
        $pipe = New-Object System.IO.Pipes.NamedPipeClientStream(
            ".", $Path, [System.IO.Pipes.PipeDirection]::InOut)
        $pipe.Connect(500)
        $pipe.Write($Bytes, 0, $Bytes.Length)
        $buffer = New-Object byte[] 4096
        $null = $pipe.Read($buffer, 0, $buffer.Length)
        $pipe.Close()
        return
    }
    $socket = [System.Net.Sockets.Socket]::new(
        [System.Net.Sockets.AddressFamily]::Unix,
        [System.Net.Sockets.SocketType]::Stream,
        [System.Net.Sockets.ProtocolType]::Unspecified)
    $socket.SendTimeout = 500
    $socket.ReceiveTimeout = 500
    $socket.Connect([System.Net.Sockets.UnixDomainSocketEndPoint]::new($Path))
    $null = $socket.Send($Bytes)
    $buffer = New-Object byte[] 4096
    $null = $socket.Receive($buffer)
    $socket.Close()
}

$inputText = [Console]::In.ReadToEnd()
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    exit 0
}

# The action is ours — the installer registered it against a codex event —
# while hook_event_name is codex's surface, the one that renamed itself out
# from under v2 (card 01a19ec3). The arms therefore dispatch on the action
# and never demand a particular event name (except the session arm, whose
# payload shape is SessionStart's by contract).
if ($null -eq $payload) { exit 0 }

$sessionId = $payload.session_id
if ([string]::IsNullOrWhiteSpace($sessionId)) { exit 0 }

# codex exports the pane's thread id into hook processes. When a fork or
# subagent fires an event under a different id, that session is not this
# pane's session — do not overwrite the roster with it.
$inherited = $env:CODEX_THREAD_ID
if (-not [string]::IsNullOrWhiteSpace($inherited) -and $inherited -ne $sessionId) { exit 0 }

# codex's payload names the THREAD, and a thread id is not always one
# `codex resume` resolves — measured 2026-10-16 (codex 0.163): a restored
# pane ran `codex resume <payload id>` into "no rollout found for thread
# id" while the transcript's rollout existed under a different id. The
# rollout filename carries the resumable id (rollout-<timestamp>-<uuid>.jsonl),
# so when the transcript matches that naming and holds a different id, the
# file wins. The corrected id rides EVERY report so the daemon's
# release-proof check compares against the id the session report
# established.
$transcriptPath = $null
if ($payload.transcript_path -is [string] -and -not [string]::IsNullOrWhiteSpace($payload.transcript_path)) {
    $transcriptPath = "$($payload.transcript_path)"
}
if ($null -ne $transcriptPath) {
    $name = [System.IO.Path]::GetFileName("$transcriptPath")
    if ($name.StartsWith("rollout-") -and $name.EndsWith(".jsonl")) {
        $groups = $name.Substring(0, $name.Length - 6).Split("-") | Select-Object -Last 5
        $candidate = $groups -join "-"
        if ($candidate.Length -eq 36 -and $candidate -cne $sessionId) { $sessionId = $candidate }
    }
}

$seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() * 1000000
$requestId = "par-mux:codex:$Action`:$seq"
switch ($Action) {
    "session" {
        if ($payload.hook_event_name -isnot [string] -or $payload.hook_event_name -cne "SessionStart") { exit 0 }
        $source = $payload.source
        if ([string]::IsNullOrWhiteSpace($source)) { $source = "startup" }
        $params = [ordered]@{
            pane_id = "$($env:PAR_MUX_PANE_ID)"
            source = "par-mux:codex"
            agent = "codex"
            seq = $seq
            session_start_source = "$source"
            agent_session_id = "$sessionId"
            session_resume_argv = @("codex", "resume", "$sessionId")
        }
    }
    "release" {
        # pane.release_agent clears the pane claim. The corrected session id
        # rides along as proof of ownership; the daemon no-ops a release
        # whose proof does not match the claimed session (a nested codex
        # exit never owns the parent claim).
        $params = [ordered]@{
            pane_id = "$($env:PAR_MUX_PANE_ID)"
            source = "par-mux:codex"
            agent = "codex"
            seq = $seq
            agent_session_id = "$sessionId"
        }
    }
    default {
        # prompt → working (UserPromptSubmit), question → question
        # (PermissionRequest), stop → idle (Stop: the turn finished, codex
        # is back at the prompt). The session id and transcript path ride
        # every state report so a claim that lost its identity is
        # re-established without waiting for a SessionStart.
        $state = @{ prompt = "working"; question = "question"; stop = "idle" }[$Action]
        if ($null -eq $state) { exit 0 }
        $params = [ordered]@{
            pane_id = "$($env:PAR_MUX_PANE_ID)"
            source = "par-mux:codex"
            agent = "codex"
            seq = $seq
            state = "$state"
            agent_session_id = "$sessionId"
        }
    }
}
if ($null -ne $transcriptPath -and $Action -ne "release") {
    $params["agent_session_path"] = "$transcriptPath"
}

$request = [ordered]@{
    id = $requestId
    method = if ($Action -eq "session") { "pane.report_agent_session" }
             elseif ($Action -eq "release") { "pane.release_agent" }
             else { "pane.report_agent" }
    params = $params
}

$requestJson = $request | ConvertTo-Json -Compress -Depth 6

try {
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($requestJson + "`n")
    Send-MuxReport $env:PAR_MUX_SOCKET $bytes
} catch {
}
exit 0
