# setup\download-codex-runtime.ps1
# Download the codex-primary-runtime bundle from OpenAI static resources and
# extract it into setup\codex-runtimes. The resulting layout matches the
# .\codex-runtimes\codex-primary-runtime\* path referenced by setup.iss.
#
# Usage:
#   .\setup\download-codex-runtime.ps1                    # default 26.426.12240
#   .\setup\download-codex-runtime.ps1 -Version 26.614.11602 -Force
param(
    [string]$Version = '26.426.12240',
    [switch]$Force
)

$ErrorActionPreference = 'Stop'

$BaseUrl  = "https://persistent.oaistatic.com/codex-primary-runtime/$Version/codex-primary-runtime-win32-x64-$Version.tar.xz"
$SetupDir = $PSScriptRoot                                            # setup\
$DestDir  = [System.IO.Path]::GetFullPath((Join-Path $SetupDir 'codex-runtimes'))
$Target   = Join-Path $DestDir 'codex-primary-runtime'
$Marker   = Join-Path $Target 'runtime.json'

# Guard: keep the target directory inside the script's own setup folder.
if (-not $DestDir.StartsWith($SetupDir, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing: target $DestDir is outside script directory $SetupDir"
}

# Idempotence: skip if already present (unless -Force).
if ((Test-Path $Marker) -and -not $Force) {
    Write-Host "Existing $Target; skipping download. Use -Force to overwrite."
    exit 0
}

New-Item -ItemType Directory -Path $DestDir -Force | Out-Null

$Archive = Join-Path $env:TEMP "codex-primary-runtime-win32-x64-$Version.tar.xz"
try {
    Write-Host "Downloading: $BaseUrl"
    curl.exe -sSL -o $Archive $BaseUrl
    if ($LASTEXITCODE -ne 0) { throw "Download failed (curl exit code $LASTEXITCODE)" }

    if ($Force -and (Test-Path $Target)) {
        Write-Host "Deleting old directory: $Target"
        Remove-Item -LiteralPath $Target -Recurse -Force
    }

    Write-Host "Extracting to: $DestDir"
    tar -xf $Archive -C $DestDir
    if ($LASTEXITCODE -ne 0) { throw "Extract failed (tar exit code $LASTEXITCODE)" }

    if (-not (Test-Path $Marker)) {
        throw "Extracted result is missing $Marker; unexpected layout"
    }
    Write-Host "Done: $Target"
}
finally {
    if (Test-Path $Archive) { Remove-Item -LiteralPath $Archive -Force }
}
