# installed by par-mux
# managed by par-mux; reinstalling or updating the integration overwrites
# this file. add custom hooks beside this file instead of editing it.
# PowerShell port of the POSIX reporter (par-mux-devin-session-hook.sh),
# ported from herdr's devin integration (Apache License 2.0): herdr's variant
# shelled out to the herdr CLI; this one sends one JSON line over the control
# socket, reads one reply, closes. Inert outside a par-mux pane (env guards)
# and silent on every failure.
# PAR_MUX_INTEGRATION_ID=devin
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
    $payload = $null
}

$sessionId = $null
if ($null -ne $payload) {
    if ($payload.session_id -is [string]) { $sessionId = $payload.session_id }
    elseif ($payload.sessionId -is [string]) { $sessionId = $payload.sessionId }
}

# herdr's fallback: when the payload carries no session id, resolve it from
# `devin list` by working directory - except on the events that name a fresh
# turn (UserPromptSubmit, a startup SessionStart), where guessing would claim
# a stale session.
$event = if ($null -ne $payload -and $payload.hook_event_name -is [string]) { $payload.hook_event_name } else { "" }
$allowFallback = $event -ne "UserPromptSubmit" -and -not ($event -eq "SessionStart" -and $payload.source -eq "startup")
if ([string]::IsNullOrWhiteSpace($sessionId) -and $allowFallback) {
    $projectDir = if ([string]::IsNullOrWhiteSpace($env:DEVIN_PROJECT_DIR)) { (Get-Location).Path } else { $env:DEVIN_PROJECT_DIR }
    try {
        $sessions = & devin list --format json 2>$null | ConvertFrom-Json
        $normalizedProject = [IO.Path]::GetFullPath($projectDir).TrimEnd('\')
        foreach ($session in @($sessions)) {
            if ($session.id -isnot [string]) { continue }
            if ($session.working_directory -isnot [string]) { continue }
            $workingDirectory = [IO.Path]::GetFullPath($session.working_directory).TrimEnd('\')
            if ($workingDirectory -ieq $normalizedProject) {
                $sessionId = $session.id
                break
            }
        }
    } catch {
    }
}
if ([string]::IsNullOrWhiteSpace($sessionId)) { exit 0 }

$params = [ordered]@{
    pane_id = "$($env:PAR_MUX_PANE_ID)"
    source = "par-mux:devin"
    agent = "devin"
    seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() * 1000000
    agent_session_id = "$sessionId"
}

$request = [ordered]@{
    id = "par-mux:devin:$([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()):$('{0:D6}' -f (Get-Random -Minimum 0 -Maximum 999999))"
    method = "pane.report_agent_session"
    params = $params
} | ConvertTo-Json -Compress -Depth 6

try {
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($request + "`n")
    Send-MuxReport $env:PAR_MUX_SOCKET $bytes
} catch {
}
exit 0
