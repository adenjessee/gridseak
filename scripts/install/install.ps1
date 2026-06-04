# scripts/install/install.ps1 — GridSeak local-proof installer (Windows).
#
# Mirrors scripts/install/install.sh: reads a manifest, picks the artifact
# for this host's triple, verifies SHA256, and extracts it into
# $env:GRIDSEAK_HOME\bin (default: $env:USERPROFILE\.gridseak\bin).
#
# Local proof flow (in a PowerShell window):
#   bash scripts/install/build-cli-release.sh x86_64-pc-windows-msvc
#   cd target\cli-release\<version>
#   python -m http.server 8765
#   $env:GRIDSEAK_MANIFEST_URL = "http://localhost:8765/cli-manifest.json"
#   powershell -ExecutionPolicy Bypass -File scripts\install\install.ps1
#
# Constraints from the spec: readable, prints planned steps before doing
# them, never asks for admin, emits per-shell PATH guidance.

$ErrorActionPreference = 'Stop'

function Log($msg)  { Write-Host "[gridseak-install] $msg" }
function Warn($msg) { Write-Warning "[gridseak-install] $msg" }
function Plan($msg) { Write-Host "[gridseak-install] plan: $msg" }

# Production default — same URL the macOS / Linux install.sh reads from.
# For local-proof flows, override via `$env:GRIDSEAK_MANIFEST_URL`.
$ManifestUrl = if ($env:GRIDSEAK_MANIFEST_URL) { $env:GRIDSEAK_MANIFEST_URL } else { 'https://gridseak.com/install/cli-manifest.json' }
$HomeRoot    = if ($env:GRIDSEAK_HOME)         { $env:GRIDSEAK_HOME }         else { Join-Path $env:USERPROFILE '.gridseak' }
$BinDir      = Join-Path $HomeRoot 'bin'
$ShareDir    = Join-Path $HomeRoot ("share\" + (Get-Date -Format 'yyyyMMddTHHmmssZ'))

# Triple resolution — pilot only ships windows x64.
$arch = (Get-CimInstance -ClassName Win32_Processor | Select-Object -First 1).Architecture
switch ($arch) {
    9 { $Triple = 'x86_64-pc-windows-msvc' }
    12 { $Triple = 'aarch64-pc-windows-msvc' }
    default { throw "Unsupported processor architecture code: $arch" }
}

Log "manifest:   $ManifestUrl"
Log "host:       $Triple"
Log "install to: $BinDir"
Log "share to:   $ShareDir"

Plan '1. download the manifest'
Plan "2. locate artifact for $Triple in the manifest"
Plan '3. download and SHA256-verify the archive'
Plan "4. extract into $ShareDir (versioned)"
Plan "5. link into $BinDir (gridseak.exe, graphengine-parsing.exe, ge-analyze.exe)"
Plan '6. print PATH guidance'

$WorkDir = New-Item -ItemType Directory -Path (Join-Path $env:TEMP ("gridseak-install-" + [guid]::NewGuid())) | Select-Object -ExpandProperty FullName
try {
    Log '[1/6] download manifest'
    $ManifestPath = Join-Path $WorkDir 'cli-manifest.json'
    Invoke-WebRequest -UseBasicParsing -Uri $ManifestUrl -OutFile $ManifestPath

    Log "[2/6] locate artifact for $Triple"
    $Manifest = Get-Content -Raw $ManifestPath | ConvertFrom-Json
    $Artifact = $Manifest.artifacts | Where-Object { $_.target -eq $Triple } | Select-Object -First 1
    if (-not $Artifact) { throw "no artifact for $Triple in manifest" }

    if ($Artifact.url -match '^https?://') {
        $ArtUrl = $Artifact.url
    } else {
        $base = $ManifestUrl.Substring(0, $ManifestUrl.LastIndexOf('/'))
        $ArtUrl = "$base/$($Artifact.url)"
    }
    Log "         version=$($Manifest.version) url=$ArtUrl"

    Log '[3/6] download artifact'
    $ArtPath = Join-Path $WorkDir ([System.IO.Path]::GetFileName($Artifact.url))
    Invoke-WebRequest -UseBasicParsing -Uri $ArtUrl -OutFile $ArtPath
    $Got = (Get-FileHash -Algorithm SHA256 -Path $ArtPath).Hash.ToLowerInvariant()
    if ($Got -ne $Artifact.sha256.ToLowerInvariant()) {
        throw "SHA256 mismatch (expected $($Artifact.sha256) got $Got)"
    }
    Log '         sha256 verified'

    Log "[4/6] extract into $ShareDir"
    New-Item -ItemType Directory -Force -Path $ShareDir | Out-Null
    # tar.exe ships with Windows 10+; fall back to .NET if missing.
    if (Get-Command tar.exe -ErrorAction SilentlyContinue) {
        tar.exe -C "$ShareDir" -xzf "$ArtPath"
        if ($LASTEXITCODE -ne 0) { throw "tar.exe failed with exit $LASTEXITCODE" }
    } else {
        throw 'tar.exe not found. Install Windows 10 1803+ or a tar tool.'
    }

    Log "[5/6] link into $BinDir"
    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    foreach ($bin in @('gridseak.exe', 'graphengine-parsing.exe', 'ge-analyze.exe')) {
        $src = Join-Path $ShareDir $bin
        if (-not (Test-Path $src)) {
            Warn "skipping $bin (not in archive)"
            continue
        }
        $dst = Join-Path $BinDir $bin
        if (Test-Path $dst) { Remove-Item $dst -Force }
        # Symlinks need Developer Mode or admin on Windows; fall back to copy.
        try {
            New-Item -ItemType SymbolicLink -Path $dst -Target $src -ErrorAction Stop | Out-Null
        } catch {
            Copy-Item $src $dst -Force
        }
    }
    $configsSrc = Join-Path $ShareDir 'configs'
    if (Test-Path $configsSrc) {
        $configsDst = Join-Path $HomeRoot 'configs'
        if (Test-Path $configsDst) { Remove-Item $configsDst -Force -Recurse }
        try {
            New-Item -ItemType SymbolicLink -Path $configsDst -Target $configsSrc -ErrorAction Stop | Out-Null
        } catch {
            Copy-Item $configsSrc $configsDst -Recurse -Force
        }
    }
    Get-ChildItem $BinDir | Format-Table Name,Length,LastWriteTime

    Log '[6/6] PATH guidance'
    $pathEntries = $env:Path -split ';'
    if ($pathEntries -contains $BinDir) {
        Log "         $BinDir is already on PATH for this session."
    } else {
        @"

Add this to your user PATH so ``gridseak`` works in new sessions:

  PowerShell (user scope, persistent):
    [Environment]::SetEnvironmentVariable('Path', "`$([Environment]::GetEnvironmentVariable('Path','User'));$BinDir", 'User')

  Or for just this session:
    `$env:Path = "$BinDir;`$env:Path"

Smoke test now (absolute path so PATH change isn't required):
  & "$BinDir\gridseak.exe" --version
  & "$BinDir\gridseak.exe" scan .

"@ | Write-Host
    }
    Log "done. version=$($Manifest.version) root=$HomeRoot"

    @"

Windows SmartScreen note
------------------------
Windows may show "Microsoft Defender SmartScreen prevented an
unrecognized app from starting" on the first launch — the binaries
are not yet Authenticode-signed. Click "More info" -> "Run anyway"
once and Windows will remember the choice.

Next steps
----------
  1. Verify the install:
       & "$BinDir\gridseak.exe" doctor

  2. Wire GridSeak into your IDE(s) (writes mcp.json + Cursor rule):
       & "$BinDir\gridseak.exe" setup

  3. Run your first scan:
       & "$BinDir\gridseak.exe" scan .

  4. Open a fresh chat in your IDE and ask: "what's risky to refactor here?"
     The agent should call gridseak_get_recommendations within its first
     two tool calls. If it doesn't:
       & "$BinDir\gridseak.exe" setup --verify

Full walkthrough: https://gridseak.com/cli
"@ | Write-Host
}
finally {
    Remove-Item -Recurse -Force $WorkDir -ErrorAction SilentlyContinue
}
