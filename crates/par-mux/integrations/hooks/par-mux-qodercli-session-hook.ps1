# installed by par-mux
# managed by par-mux; reinstalling or updating the integration overwrites
# this file. add custom hooks beside this file instead of editing it.
# PowerShell port of the POSIX reporter (par-mux-droid-session-hook.sh),
# ported from herdr's qodercli integration (herdr's variant shelled out to
# the herdr CLI; this one speaks the pipe directly) (Apache License 2.0): one JSON line
# over the control socket, one reply read, close. Inert outside a par-mux
# pane (env guards) and silent on every failure.
# PAR_MUX_INTEGRATION_ID=qodercli
# PAR_MUX_INTEGRATION_VERSION=1

param([string]$Action = "")

if ($Action -ne "session") { exit 0 }
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
    exit 0
}
if ($null -eq $payload) { exit 0 }

# Registered under SessionStart only; a missing field is allowed.
$hookEventName = $payload.hook_event_name
if ($hookEventName -is [string] -and $hookEventName -ne "SessionStart") { exit 0 }

$sessionId = $payload.session_id
if (-not ($sessionId -is [string]) -or [string]::IsNullOrWhiteSpace($sessionId)) { exit 0 }

$params = [ordered]@{
    pane_id = "$($env:PAR_MUX_PANE_ID)"
    source = "par-mux:qodercli"
    agent = "qodercli"
    seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() * 1000000
    agent_session_id = "$sessionId"
}
$startSource = $payload.source
if ($startSource -is [string] -and -not [string]::IsNullOrWhiteSpace($startSource)) {
    $params["session_start_source"] = $startSource
}

$request = [ordered]@{
    id = "par-mux:qodercli:$([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()):$('{0:D6}' -f (Get-Random -Minimum 0 -Maximum 999999))"
    method = "pane.report_agent_session"
    params = $params
} | ConvertTo-Json -Compress -Depth 6

try {
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($request + "`n")
    Send-MuxReport $env:PAR_MUX_SOCKET $bytes
} catch {
}
exit 0
