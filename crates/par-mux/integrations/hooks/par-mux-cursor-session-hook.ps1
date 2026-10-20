# installed by par-mux
# managed by par-mux; reinstalling or updating the integration overwrites
# this file. add custom hooks beside this file instead of editing it.
# PowerShell port of the POSIX reporter (par-mux-cursor-session-hook.sh),
# ported from herdr's cursor integration (Apache License 2.0): one JSON
# line over the control socket, one reply read, close. Inert outside a
# par-mux pane (env guards) and silent on every failure. Every exit prints
# `{}`, the empty hook response herdr's port answers cursor with.
# PAR_MUX_INTEGRATION_ID=cursor
# PAR_MUX_INTEGRATION_VERSION=1

param([string]$Action = "")

function Exit-Hook {
    Write-Output "{}"
    exit 0
}

if ($Action -ne "session") { Exit-Hook }
if ($env:PAR_MUX_ENV -ne "1") { Exit-Hook }
if ([string]::IsNullOrWhiteSpace($env:PAR_MUX_SOCKET)) { Exit-Hook }
if ([string]::IsNullOrWhiteSpace($env:PAR_MUX_PANE_ID)) { Exit-Hook }

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
$jsonStart = $inputText.IndexOf("{")
if ($jsonStart -gt 0) {
    $inputText = $inputText.Substring($jsonStart)
}
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    Exit-Hook
}

if ($null -eq $payload) { Exit-Hook }
$event = if ($payload.hook_event_name -is [string]) { $payload.hook_event_name } else { $payload.hookEventName }
if (-not [string]::IsNullOrWhiteSpace($event) -and $event -ne "sessionStart") { Exit-Hook }

$sessionId = $null
foreach ($name in @("session_id", "sessionId", "conversation_id", "conversationId")) {
    $value = $payload.$name
    if ($value -is [string] -and -not [string]::IsNullOrWhiteSpace($value)) {
        $sessionId = $value
        break
    }
}
if ([string]::IsNullOrWhiteSpace($sessionId)) { Exit-Hook }

# herdr's cursor resume shape; npm installs the Windows shim as a .cmd.
$cursorAgent = if ($env:OS -eq "Windows_NT") { "cursor-agent.cmd" } else { "cursor-agent" }

$request = [ordered]@{
    id = "par-mux:cursor:$([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()):$('{0:D6}' -f (Get-Random -Minimum 0 -Maximum 999999))"
    method = "pane.report_agent_session"
    params = [ordered]@{
        pane_id = "$($env:PAR_MUX_PANE_ID)"
        source = "par-mux:cursor"
        agent = "cursor"
        seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() * 1000000
        agent_session_id = "$sessionId"
        session_resume_argv = @($cursorAgent, "--resume", "$sessionId")
    }
} | ConvertTo-Json -Compress -Depth 6

try {
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($request + "`n")
    Send-MuxReport $env:PAR_MUX_SOCKET $bytes
} catch {
}
Exit-Hook
