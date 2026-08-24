# setup\download-codex-runtime.ps1
# Download the codex-primary-runtime bundle into setup\marketplaces\codex-primary-runtime.tar.xz.
# That single file is the only gitignored build input (see .gitignore); openai-bundled.tar.xz
# in setup\marketplaces is committed. Both archives are shipped under {app}\marketplaces and extracted
# by codex-ui at startup (Rust native xz+tar), so no tar.exe is bundled anymore.
#
# Usage:
#   .\setup\download-codex-runtime.ps1                    # default 26.426.12240
#   .\setup\download-codex-runtime.ps1 -Version 26.614.11602 -Force
param(
    [string]$Version = '26.426.12240',
    [switch]$Force
)

$ErrorActionPreference = 'Stop'

$BaseUrl    = "https://persistent.oaistatic.com/codex-primary-runtime/$Version/codex-primary-runtime-win32-x64-$Version.tar.xz"
$SetupDir   = $PSScriptRoot                                            # setup\
$CompDir    = [System.IO.Path]::GetFullPath((Join-Path $SetupDir 'marketplaces'))
$RuntimeTar = Join-Path $CompDir 'codex-primary-runtime.tar.xz'
$SysTar     = Join-Path $env:WINDIR 'System32\tar.exe'

# Guard: keep output inside the script's own setup folder.
if (-not $CompDir.StartsWith($SetupDir, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing: output $CompDir is outside script directory $SetupDir"
}

# Idempotence: skip if the bundle already exists (unless -Force).
if ((Test-Path $RuntimeTar) -and -not $Force) {
    Write-Host "Existing $RuntimeTar; skipping download. Use -Force to overwrite."
    exit 0
}

New-Item -ItemType Directory -Path $CompDir -Force | Out-Null

Write-Host "Downloading: $BaseUrl"
curl.exe -sSL -o $RuntimeTar $BaseUrl
if ($LASTEXITCODE -ne 0) { throw "Download failed (curl exit code $LASTEXITCODE)" }

if (-not (Test-Path $RuntimeTar)) { throw "Missing expected output: $RuntimeTar" }

$first = (& $SysTar -tf $RuntimeTar | Select-Object -First 1)
if ($LASTEXITCODE -ne 0) { throw "Listing archive failed" }
if ($first -ne 'codex-primary-runtime/') { throw "Unexpected runtime archive top level: $first" }

Write-Host "Done. Input ready: $RuntimeTar"
