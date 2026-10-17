# par-mux muse session hook (Windows) — reports session identity to the
# par-mux daemon on muse SessionStart events. PowerShell port of the POSIX
# reporter (par-mux-muse-session-hook.sh): one JSON line per discovered
# daemon socket, one reply read, close. Muse clears the hook environment
# (only MUSE_SESSION_ID/MUSE_THREAD_ID survive), so unlike the env-guarded
# reporters this one discovers daemon sockets itself and lets each daemon
# resolve the pane from the payload's cwd. Silent on every failure.
# PAR_MUX_INTEGRATION_ID=muse
# PAR_MUX_INTEGRATION_VERSION=1

param([string]$Action = "")

if ($Action -ne "session") { exit 0 }

# Transport mirrors the daemon's platform split (par-term-emu-core-rust
# mux ipc.rs): named pipes on Windows - the marker-file path string becomes
# the pipe name after \\.\pipe\, exactly as the interprocess crate derives
# it - and Unix-domain sockets elsewhere. One JSON line out, one reply
# read, close.
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

# muse names its events claude-style; only session starts are ours to report.
if ($null -eq $payload) { exit 0 }
if ($payload.hook_event_name -isnot [string] -or $payload.hook_event_name -cne "SessionStart") { exit 0 }

# muse exports the top-level session id into every hook process; the
# payload field is the fallback.
$sessionId = $env:MUSE_SESSION_ID
if ([string]::IsNullOrWhiteSpace($sessionId)) { $sessionId = $payload.session_id }
if ([string]::IsNullOrWhiteSpace($sessionId)) { exit 0 }

# The cwd is the pane identity surrogate: muse never changes its own
# working directory, and the daemon resolves panes from it.
$cwd = $payload.cwd
if ([string]::IsNullOrWhiteSpace($cwd)) { exit 0 }

$source = $payload.source

$seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() * 1000000
$requestId = "par-mux:muse:$([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()):$('{0:D6}' -f (Get-Random -Minimum 0 -Maximum 999999))"
$params = [ordered]@{
    source = "par-mux:muse"
    agent = "muse"
    seq = $seq
    agent_session_id = "$sessionId"
    cwd = "$cwd"
}
if (-not [string]::IsNullOrWhiteSpace($source)) {
    $params["session_start_source"] = "$source"
}

$request = [ordered]@{
    id = $requestId
    method = "pane.report_agent_session_cwd"
    params = $params
} | ConvertTo-Json -Compress -Depth 6

# The daemon sockets live at default_socket_path's Windows tier: marker
# files named par-mux-<name>.sock in the temp directory, whose full path
# string is the pipe name. muse strips the environment, so TEMP falls back
# to the .NET temp root.
$tempRoot = $env:TEMP
if ([string]::IsNullOrWhiteSpace($tempRoot)) { $tempRoot = [System.IO.Path]::GetTempPath() }
$markers = @()
try {
    $markers = @(Get-ChildItem -Path $tempRoot -Filter 'par-mux-*.sock' -File -ErrorAction SilentlyContinue |
        Sort-Object -Property Name)
} catch {
}
foreach ($marker in $markers) {
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($request + "`n")
        Send-MuxReport "$($marker.FullName)" $bytes
    } catch {
    }
}
exit 0
