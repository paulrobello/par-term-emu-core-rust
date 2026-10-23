# Installer for the par-term-emu-core-rust standalone release binaries.
#
# Typical use (Windows PowerShell):
#   irm https://raw.githubusercontent.com/paulrobello/par-term-emu-core-rust/main/install.ps1 | iex
#
# Installs par-term-streamer and/or par-mux into $HOME\.cargo\bin (or -BinDir),
# verifying every download against its release SHA-256 before anything is
# moved into place. Never touches services or startup entries.
#
# Default (no switches): the plain streamer and par-mux. -Mux may combine with
# either streamer variant; -Streamer and -StreamerMux are mutually exclusive.

param(
    [switch]$Streamer,
    [switch]$StreamerMux,
    [switch]$Mux,
    [string]$Version,
    [string]$BinDir
)

$ErrorActionPreference = 'Stop'
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch {
    # Older hosts where the property set fails already negotiate TLS 1.2+.
}

$Repo = 'paulrobello/par-term-emu-core-rust'
$ApiLatest = "https://api.github.com/repos/$Repo/releases/latest"
$DlBase = "https://github.com/$Repo/releases/download"
$Headers = @{ 'User-Agent' = 'par-term-installer' }

function Die([string]$Message) {
    Write-Host "install.ps1: error: $Message" -ForegroundColor Red
    exit 1
}

# Strip a leading v and require strict X.Y.Z. Returns the normalized version.
function Get-NormalizedVersion([string]$Value) {
    $v = $Value.TrimStart('v')
    if ($v -notmatch '^\d+\.\d+\.\d+$') {
        return $null
    }
    return $v
}

# Checksum entry for exactly $AssetName from sha256sum-format text; $null if
# missing, malformed, or duplicated.
function Get-ChecksumFromSums([string]$SumsText, [string]$AssetName) {
    $found = $null
    $seen = $false
    foreach ($line in ($SumsText -split "`n")) {
        if ($line -match '^\s*([0-9a-fA-F]{64})\s+\*?(\S+)\s*$') {
            if ($Matches[2] -eq $AssetName) {
                if ($seen) { return $null }
                $seen = $true
                $found = $Matches[1].ToLowerInvariant()
            }
        }
    }
    return $found
}

# Download $Url to $Dest; returns $false on failure instead of throwing.
function Get-RemoteFile([string]$Url, [string]$Dest) {
    if (-not $Url.StartsWith('https://')) {
        Write-Host "install.ps1: error: refusing non-HTTPS URL: $Url" -ForegroundColor Red
        return $false
    }
    try {
        Invoke-WebRequest -Uri $Url -Headers $Headers -OutFile $Dest -UseBasicParsing
        return $true
    } catch {
        return $false
    }
}

if ($Streamer -and $StreamerMux) {
    Die "-Streamer and -StreamerMux are mutually exclusive (pick one streamer variant per run)"
}
$anySelector = $Streamer -or $Mux -or $StreamerMux
$wantStreamer = $Streamer -or -not $anySelector
$wantMux = $Mux -or -not $anySelector
$wantStreamerMux = $StreamerMux

# Windows-only; other platforms use install.sh.
if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
    Die "unsupported Windows architecture '$($env:PROCESSOR_ARCHITECTURE)'; releases publish Windows x86_64 binaries only - pick an asset manually from https://github.com/$Repo/releases"
}

# Resolve the release tag and normalized version.
if ($Version) {
    $version = Get-NormalizedVersion $Version
    if (-not $version) {
        Die "invalid -Version value '$Version' (expected vX.Y.Z)"
    }
} else {
    Write-Host "install.ps1: resolving latest release"
    try {
        $release = Invoke-RestMethod -Uri $ApiLatest -Headers $Headers
    } catch {
        Die "could not fetch the latest release from $ApiLatest"
    }
    $version = Get-NormalizedVersion $release.tag_name
    if (-not $version) {
        Die "release tag is not a plain version: $($release.tag_name)"
    }
}

if (-not $BinDir) {
    $BinDir = "$HOME\.cargo\bin"
}

$tempRoot = Join-Path ([IO.Path]::GetTempPath()) "par-term-install-$(Get-Random)"
New-Item -ItemType Directory -Path $tempRoot | Out-Null

try {
    $targets = @()
    if ($wantStreamer) {
        $targets += [pscustomobject]@{
            Kind = 'streamer'
            Asset = 'par-term-streamer-windows-x86_64.exe'
            Sidecar = 'par-term-streamer-windows-x86_64.exe.sha256'
            DestName = 'par-term-streamer.exe'
        }
    }
    if ($wantStreamerMux) {
        $targets += [pscustomobject]@{
            Kind = 'streamer-mux'
            Asset = 'par-term-streamer-mux-windows-x86_64.exe'
            Sidecar = 'par-term-streamer-mux-windows-x86_64.exe.sha256'
            DestName = 'par-term-streamer.exe'
        }
    }
    if ($wantMux) {
        $targets += [pscustomobject]@{
            Kind = 'mux'
            Asset = "par-mux-v$version-x86_64-pc-windows-msvc.zip"
            Sidecar = "par-mux-v$version-SHA256SUMS.txt"
            DestName = 'par-mux.exe'
        }
    }

    foreach ($target in $targets) {
        $asset = $target.Asset
        Write-Host "install.ps1: fetching $asset (v$version, windows/x86_64)"

        $sumsFile = Join-Path $tempRoot $target.Sidecar
        if (-not (Get-RemoteFile "$DlBase/v$version/$($target.Sidecar)" $sumsFile)) {
            if ($target.Kind -ne 'mux') {
                Die "no checksum sidecar $($target.Sidecar) for this release; releases older than the sidecar feature ship no sidecar - install manually from https://github.com/$Repo/releases"
            }
            Die "checksum file $($target.Sidecar) not found for release v$version"
        }
        $expected = Get-ChecksumFromSums (Get-Content $sumsFile -Raw) $asset
        if (-not $expected) {
            Die "checksum file has no valid entry for $asset"
        }

        $assetFile = Join-Path $tempRoot $asset
        if (-not (Get-RemoteFile "$DlBase/v$version/$asset" $assetFile)) {
            Die "download failed: $asset"
        }
        $actual = (Get-FileHash -Path $assetFile -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) {
            Die "SHA-256 mismatch for ${asset}: expected $expected, got $actual; nothing was installed"
        }

        $dest = Join-Path $BinDir $target.DestName
        New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
        if (Test-Path $dest) {
            Move-Item -Force $dest "$dest.previous"
            Write-Host "install.ps1: kept previous binary at $dest.previous"
        }

        if ($target.Kind -eq 'mux') {
            $top = "par-mux-v$version-x86_64-pc-windows-msvc"
            $extractDir = Join-Path $tempRoot 'extract'
            Expand-Archive -Path $assetFile -DestinationPath $extractDir -Force
            $member = Join-Path (Join-Path $extractDir $top) 'par-mux.exe'
            if (-not (Test-Path $member)) {
                Die "could not extract $top\par-mux.exe from $asset"
            }
            Move-Item -Force $member $dest
        } else {
            Move-Item -Force $assetFile $dest
        }
        Write-Host "install.ps1: installed $dest"
    }

    $pathParts = $env:Path -split ';' | Where-Object { $_ }
    if ($pathParts -notcontains $BinDir) {
        Write-Host "install.ps1: warning: $BinDir is not on your PATH; add it to your environment" -ForegroundColor Yellow
    }

    Write-Host ""
    Write-Host "install.ps1: done (v$version). Next steps:"
    foreach ($target in $targets) {
        $dest = Join-Path $BinDir $target.DestName
        if ($target.Kind -ne 'mux') {
            Write-Host "  par-term-streamer: $dest"
            Write-Host "    serve the web terminal:  par-term-streamer.exe --port 8099  (then open http://127.0.0.1:8099)"
        } else {
            Write-Host "  par-mux: $dest  (running 'par-mux' starts the daemon)"
        }
    }
} finally {
    Remove-Item -Recurse -Force $tempRoot -ErrorAction SilentlyContinue
}
