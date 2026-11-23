# installed by par-mux
# managed by par-mux; reinstalling or updating the integration overwrites
# this file. add custom hooks beside this file instead of editing it.
# PowerShell port of the POSIX reporter (par-mux-agy-session-hook.sh): agy
# (Antigravity CLI) camelCase JSON on stdin, `{}` on stdout, one JSON line
# per report over the control socket. Inert outside a par-mux pane and
# silent on every failure.
# PAR_MUX_INTEGRATION_ID=agy
# PAR_MUX_INTEGRATION_VERSION=1

param([string]$Action = "")

Write-Output "{}"
if ($Action -notin @("session", "working", "idle")) { exit 0 }
if ($env:PAR_MUX_ENV -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:PAR_MUX_SOCKET)) { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:PAR_MUX_PANE_ID)) { exit 0 }

# Transport mirrors the daemon's platform split (mux ipc.rs): named pipes
# on Windows, Unix-domain sockets elsewhere.
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

$stdinReader = New-Object System.IO.StreamReader([Console]::OpenStandardInput(), [System.Text.Encoding]::UTF8)
$inputText = $stdinReader.ReadToEnd()
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    $payload = $null
}

if ($null -eq $payload -or $payload.conversationId -isnot [string] -or [string]::IsNullOrWhiteSpace($payload.conversationId)) { exit 0 }
$sessionId = $payload.conversationId
$transcript = $null
if ($payload.transcriptPath -is [string] -and -not [string]::IsNullOrWhiteSpace($payload.transcriptPath)) { $transcript = $payload.transcriptPath }
if ($Action -eq "idle" -and $payload.fullyIdle -is [bool] -and -not $payload.fullyIdle) { exit 0 }

$seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() * 1000000
$source = "par-mux:agy"

function New-StateParams([string]$State, [int]$Offset) {
    $params = [ordered]@{
        pane_id = "$($env:PAR_MUX_PANE_ID)"
        source = $source
        agent = "agy"
        seq = $seq + $Offset
        state = $State
        agent_session_id = $sessionId
    }
    if ($null -ne $transcript) { $params["agent_session_path"] = $transcript }
    return $params
}

$requests = @()
if ($Action -eq "session") {
    $identity = [ordered]@{
        pane_id = "$($env:PAR_MUX_PANE_ID)"
        source = $source
        agent = "agy"
        seq = $seq
        session_start_source = "startup"
        agent_session_id = $sessionId
        session_resume_argv = @("agy", "--conversation", $sessionId)
    }
    if ($null -ne $transcript) { $identity["agent_session_path"] = $transcript }
    $requests += [ordered]@{ id = "${source}:session:$seq"; method = "pane.report_agent_session"; params = $identity }
    $requests += [ordered]@{ id = "${source}:session-idle:$seq"; method = "pane.report_agent"; params = (New-StateParams "idle" 1) }
} else {
    $requests += [ordered]@{ id = "${source}:${Action}:$seq"; method = "pane.report_agent"; params = (New-StateParams $Action 0) }
}

try {
    foreach ($request in $requests) {
        $line = ($request | ConvertTo-Json -Compress -Depth 6) + "`n"
        Send-MuxReport $env:PAR_MUX_SOCKET ([System.Text.Encoding]::UTF8.GetBytes($line))
    }
} catch {
}
exit 0
