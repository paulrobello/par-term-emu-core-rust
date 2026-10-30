# par-mux claude session hook (Windows) — reports session identity to the
# par-mux daemon on claude SessionStart events (no action argument, or
# `session`), releases the pane claim on SessionEnd (`release`), and keeps
# a live session registered between those (`prompt` on UserPromptSubmit,
# `notify` on Notification, `stop` on Stop).
# PowerShell port of the POSIX reporter (par-mux-claude-session-hook.sh):
# one JSON line over the control socket, one reply read, close. Inert
# outside a par-mux pane (env guards) and silent on every failure.
#
# A claude nested inside another claude's pane (the `claude -p` a sub-agent
# shells out to) inherits this pane's env, so every arm first walks the
# process ancestry: two or more claude processes above this hook mean a
# nested claude, which sends nothing. Every report carries the claude
# session id — proof of ownership for the release, and the resume identity
# a state report re-establishes after a claim-losing event.
# PAR_MUX_INTEGRATION_ID=claude
# PAR_MUX_INTEGRATION_VERSION=4

if ($env:PAR_MUX_ENV -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:PAR_MUX_SOCKET)) { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:PAR_MUX_PANE_ID)) { exit 0 }

# The same toward-matching bias as the daemon's argv matcher (foreground.rs
# argv_matches): the label may ride argv[0], a path component, or a package
# path an interpreter executes.
function Test-ClaudeCommand([string]$Line) {
    foreach ($token in $Line -split "\s+") {
        if (-not $token) { continue }
        foreach ($component in ($token -split "[\\/]")) {
            if ($component -eq "claude" -or $component.StartsWith("claude-") -or $component.StartsWith("claude_") -or $component.StartsWith("claude.")) {
                return $true
            }
        }
    }
    return $false
}

# Two or more claude processes above this hook mean the claude this hook
# serves was spawned by another claude (its Bash tool), whose pane claim
# must not be touched. A top-level claude's hook sees exactly one (its own
# parent). Best-effort: any failure reads as not nested — the hook keeps
# its primary function and the daemon's identity guards re-check on their
# side. Win32_Process exists on Windows only; elsewhere the walk finds
# nothing and the hook behaves as before.
function Test-NestedClaude {
    try {
        $byPid = @{}
        foreach ($proc in Get-CimInstance -ClassName Win32_Process -Property ProcessId, ParentProcessId, CommandLine) {
            $byPid[[uint32]$proc.ProcessId] = $proc
        }
        $current = [System.Diagnostics.Process]::GetCurrentProcess().Id
        $hits = 0
        for ($i = 0; $i -lt 64; $i++) {
            $proc = $byPid[[uint32]$current]
            if ($null -eq $proc) { break }
            $current = [uint32]$proc.ParentProcessId
            if ($current -le 1) { break }
            $line = $proc.CommandLine
            if ($line -and (Test-ClaudeCommand $line)) {
                $hits++
                if ($hits -ge 2) { return $true }
            }
        }
    } catch {
    }
    return $false
}

if (Test-NestedClaude) { exit 0 }

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
    $null = $socket.Send($bytes)
    $buffer = New-Object byte[] 4096
    $null = $socket.Receive($buffer)
    $socket.Close()
}

$action = if ($args.Count -gt 0) { "$($args[0])" } else { "" }
$seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() * 1000000

# The hook payload every arm reads: SessionStart, SessionEnd,
# UserPromptSubmit, Notification, and Stop all carry session_id;
# transcript_path rides along when claude has one.
$inputText = [Console]::In.ReadToEnd()
$payload = $null
try {
    if (-not [string]::IsNullOrWhiteSpace($inputText)) {
        $payload = $inputText | ConvertFrom-Json
    }
} catch {
    $payload = $null
}

if ($action -eq "release") {
    # pane.release_agent clears the pane claim (core aed41c2). The guards
    # are on the daemon side; a later timestamp always advances past the
    # session report seq from the same source. The session id rides along
    # as proof of ownership: the daemon no-ops a release whose proof does
    # not match the claim's session (a nested claude's exit never owns the
    # parent's claim).
    $params = [ordered]@{
        pane_id = "$($env:PAR_MUX_PANE_ID)"
        source = "par-mux:claude"
        agent = "claude"
        seq = $seq
    }
    if ($payload -and $payload.session_id) {
        $params["agent_session_id"] = "$($payload.session_id)"
    }
    $request = [ordered]@{
        id = "par-mux:claude:release:$seq"
        method = "pane.release_agent"
        params = $params
    } | ConvertTo-Json -Compress -Depth 6
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($request + "`n")
        Send-MuxReport $env:PAR_MUX_SOCKET $bytes
    } catch {
    }
    exit 0
}

# State reports keep a live session on the roster between its start and
# end events: a prompt submit is working, the permission prompt is a
# question, and the 60s-idle notification / a finished turn are idle.
# (prompt/notify/stop — the installer registers the same-named action
# under UserPromptSubmit/Notification/Stop.)
if ($action -in @("prompt", "notify", "stop")) {
    if ($action -eq "prompt") {
        $state = "working"
    } elseif ($action -eq "notify") {
        $state = "idle"
        $message = $payload.message
        if ($message -is [string] -and $message.ToLower().Contains("permission")) {
            $state = "question"
        }
    } else {
        $state = "idle"
    }
    # The session id and transcript path ride every state report: a claim
    # that lost its identity (a pre-proof nested release, a crash) is
    # re-established by the parent's next prompt/notify/stop without
    # waiting for a SessionStart.
    $params = [ordered]@{
        pane_id = "$($env:PAR_MUX_PANE_ID)"
        source = "par-mux:claude"
        agent = "claude"
        seq = $seq
        state = $state
    }
    if ($payload -and $payload.session_id) {
        $params["agent_session_id"] = "$($payload.session_id)"
    }
    if ($payload -and $payload.transcript_path) {
        $params["agent_session_path"] = "$($payload.transcript_path)"
    }
    $request = [ordered]@{
        id = "par-mux:claude:$action`:$seq"
        method = "pane.report_agent"
        params = $params
    } | ConvertTo-Json -Compress -Depth 6
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($request + "`n")
        Send-MuxReport $env:PAR_MUX_SOCKET $bytes
    } catch {
    }
    exit 0
}

$sessionId = $payload.session_id
if ([string]::IsNullOrWhiteSpace($sessionId)) { exit 0 }

$source = $payload.source
if ([string]::IsNullOrWhiteSpace($source)) { $source = "startup" }

$params = [ordered]@{
    pane_id = "$($env:PAR_MUX_PANE_ID)"
    source = "par-mux:claude"
    agent = "claude"
    seq = $seq
    session_start_source = "$source"
    agent_session_id = "$sessionId"
    session_resume_argv = @("claude", "--resume", "$sessionId")
}
if ($payload.transcript_path -is [string] -and -not [string]::IsNullOrWhiteSpace($payload.transcript_path)) {
    $params["agent_session_path"] = "$($payload.transcript_path)"
}

$request = [ordered]@{
    id = "par-mux:claude:$seq"
    method = "pane.report_agent_session"
    params = $params
} | ConvertTo-Json -Compress -Depth 6

try {
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($request + "`n")
    Send-MuxReport $env:PAR_MUX_SOCKET $bytes
} catch {
}
exit 0
