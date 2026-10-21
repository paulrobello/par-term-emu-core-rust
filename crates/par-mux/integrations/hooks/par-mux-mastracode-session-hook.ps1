# installed by par-mux
# managed by par-mux; reinstalling or updating the integration overwrites
# this file. add custom hooks beside this file instead of editing it.
# PowerShell port of the POSIX reporter (par-mux-mastracode-session-hook.sh),
# ported from herdr's mastracode integration (Apache License 2.0): herdr's
# variant shelled out to the herdr CLI; this one sends one JSON line over the
# control socket, reads one reply, closes. Inert outside a par-mux pane (env
# guards) and silent on every failure.
# PAR_MUX_INTEGRATION_ID=mastracode
# PAR_MUX_INTEGRATION_VERSION=1

param([string]$Action = "")

if ($Action -notin @("session", "working", "idle", "blocked")) { exit 0 }
if ($env:PAR_MUX_ENV -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:PAR_MUX_SOCKET)) { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:PAR_MUX_PANE_ID)) { exit 0 }

# Transport mirrors the daemon's platform split (mux ipc.rs): named pipes
# on Windows - the socket path string becomes the pipe name after
# \\.\pipe\, exactly as the interprocess crate derives it - and Unix-domain
# sockets elsewhere.
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

# Read hook input as raw bytes so UTF-8 payloads decode correctly even when
# the console input code page is not UTF-8.
$stdinReader = New-Object System.IO.StreamReader([Console]::OpenStandardInput(), [System.Text.Encoding]::UTF8)
$inputText = $stdinReader.ReadToEnd()
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    $payload = $null
}

$sessionId = $null
if ($null -ne $payload -and $payload.session_id -is [string]) { $sessionId = $payload.session_id }

$params = [ordered]@{
    pane_id = "$($env:PAR_MUX_PANE_ID)"
    source = "par-mux:mastracode"
    agent = "mastracode"
    seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() * 1000000
}
$method = "pane.report_agent"
if ($Action -eq "session") {
    if ([string]::IsNullOrWhiteSpace($sessionId)) { exit 0 }
    $method = "pane.report_agent_session"
    $params["agent_session_id"] = "$sessionId"
    $params["session_start_source"] = "startup"
} else {
    $params["state"] = $Action
    if (-not [string]::IsNullOrWhiteSpace($sessionId)) {
        $params["agent_session_id"] = "$sessionId"
    }
}

$request = [ordered]@{
    id = "par-mux:mastracode:$([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()):$('{0:D6}' -f (Get-Random -Minimum 0 -Maximum 999999))"
    method = $method
    params = $params
} | ConvertTo-Json -Compress -Depth 6

try {
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($request + "`n")
    Send-MuxReport $env:PAR_MUX_SOCKET $bytes
} catch {
}
exit 0
