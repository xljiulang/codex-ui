# 取 ONNX Runtime 的 Windows x64 动态库（onnxruntime.dll）到 setup/bin/。
#
# 背景：知识库向量化走 src-tauri 的 fastembed + ort（feature `api-24`），运行时从
# <exe 目录>/bin/onnxruntime.dll 动态加载，因此该 DLL 必须随安装包分发（build-release.bat 会调用本脚本）。
# **版本下限 1.24**：ort 以 GetApi(24) 申请接口，低于 1.24 的运行时拿不到 v24 API 会直接加载失败。
#
# 为什么默认从 NuGet 只下 6 MB：Microsoft.ML.OnnxRuntime 的 .nupkg 有 157 MB（含各平台原生库），
# 但它支持 HTTP Range，于是先取 zip 尾部中央目录定位条目，再只拉那一段压缩数据并解压——
# 实际传输 = 目标条目大小 + 少量目录字节。Range 不可用时用 -Full 走整包下载。
#
# 用法：
#   pwsh -File scripts/fetch-onnxruntime.ps1                # 默认 1.30.0（NuGet）→ setup/bin/onnxruntime.dll
#   pwsh -File scripts/fetch-onnxruntime.ps1 -AlsoDev       # 同时复制到 src-tauri/target/{debug,release}/bin
#   pwsh -File scripts/fetch-onnxruntime.ps1 -Source pypi   # 改从 PyPI wheel 取（同版本官方构建，整包约 14 MB）
#   pwsh -File scripts/fetch-onnxruntime.ps1 -Full          # 服务器不支持 Range 时的退路（整包下载后解压）

param(
    [string]$Version = "1.30.0",
    [ValidateSet("nuget", "pypi")]
    [string]$Source = "nuget",
    [string]$OutFile = "",
    [switch]$Full,
    [switch]$AlsoDev
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.IO.Compression
# Windows PowerShell 5.1 需要显式加载该程序集才能用 ZipFile（PS7 已内置，重复加载无害）
try { Add-Type -AssemblyName System.IO.Compression.FileSystem } catch { }

# ort 的 feature `api-24` 要求 ONNX Runtime >= 1.24（ORT 的 C API 版本号 == 次版本号）
$MinVersion = [version]"1.24.0"

$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $OutFile) {
    $OutFile = Join-Path $repoRoot "setup/bin/onnxruntime.dll"
}

function Get-SourceInfo {
    param([string]$Src, [string]$Ver)
    if ($Src -eq "nuget") {
        return @{
            Url   = "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/$Ver/microsoft.ml.onnxruntime.$Ver.nupkg"
            Entry = "runtimes/win-x64/native/onnxruntime.dll"
        }
    }
    # PyPI：挑该版本下最新的 win_amd64 wheel（与 NuGet 同源构建，体积最小）
    $meta = (Invoke-WebRequest -UseBasicParsing "https://pypi.org/pypi/onnxruntime/$Ver/json" -TimeoutSec 120).Content | ConvertFrom-Json
    $wheel = $meta.urls | Where-Object { $_.filename -like "*win_amd64.whl" } |
        Sort-Object -Property @{ Expression = { $_.filename } } -Descending | Select-Object -First 1
    if (-not $wheel) { throw "PyPI 上没有 onnxruntime $Ver 的 win_amd64 wheel" }
    return @{ Url = $wheel.url; Entry = "onnxruntime/capi/onnxruntime.dll" }
}

# zip 中央目录里定位条目 → 返回 localHeaderOffset / compressedSize / method
function Find-ZipEntry {
    param([byte[]]$Bytes, [string]$Name)
    for ($i = 0; $i -lt $Bytes.Length - 46; $i++) {
        if ($Bytes[$i] -ne 0x50 -or $Bytes[$i + 1] -ne 0x4B -or $Bytes[$i + 2] -ne 0x01 -or $Bytes[$i + 3] -ne 0x02) { continue }
        $nameLen = [int]$Bytes[$i + 28] -bor ([int]$Bytes[$i + 29] -shl 8)
        if ($i + 46 + $nameLen -gt $Bytes.Length) { continue }
        $candidate = [System.Text.Encoding]::UTF8.GetString($Bytes, $i + 46, $nameLen)
        if ($candidate -ne $Name) { continue }
        $method = [int]$Bytes[$i + 10] -bor ([int]$Bytes[$i + 11] -shl 8)
        $compSize = [int64]$Bytes[$i + 20] -bor ([int64]$Bytes[$i + 21] -shl 8) -bor ([int64]$Bytes[$i + 22] -shl 16) -bor ([int64]$Bytes[$i + 23] -shl 24)
        $localOffset = [int64]$Bytes[$i + 42] -bor ([int64]$Bytes[$i + 43] -shl 8) -bor ([int64]$Bytes[$i + 44] -shl 16) -bor ([int64]$Bytes[$i + 45] -shl 24)
        return @{ Method = $method; CompressedSize = $compSize; LocalOffset = $localOffset }
    }
    return $null
}

function Get-RangeBytes {
    param([string]$Url, [int64]$Start, [int64]$End)
    $resp = Invoke-WebRequest -UseBasicParsing $Url -Headers @{ Range = "bytes=$Start-$End" } -TimeoutSec 600
    if ($resp.StatusCode -ne 206) { throw "服务器未按 Range 返回（status=$($resp.StatusCode)）" }
    return [byte[]]$resp.Content
}

function Expand-DeflateBytes {
    param([byte[]]$Data)
    $input = New-Object System.IO.MemoryStream(, $Data)
    $deflate = New-Object System.IO.Compression.DeflateStream($input, [System.IO.Compression.CompressionMode]::Decompress)
    $output = New-Object System.IO.MemoryStream
    try { $deflate.CopyTo($output); return $output.ToArray() }
    finally { $deflate.Dispose(); $input.Dispose(); $output.Dispose() }
}

$info = Get-SourceInfo -Src $Source -Ver $Version
Write-Host "来源：$Source  版本：$Version"
Write-Host "URL ：$($info.Url)"

$dllBytes = $null
if (-not $Full) {
    try {
        # 1) 先用 1 字节探测总长度（Content-Range: bytes 0-0/<total>）
        $probe = Invoke-WebRequest -UseBasicParsing $info.Url -Headers @{ Range = "bytes=0-0" } -TimeoutSec 120
        $contentRange = [string]($probe.Headers['Content-Range'] | Select-Object -First 1)
        if ($probe.StatusCode -ne 206 -or $contentRange -notmatch "/(\d+)$") { throw "服务器未按 Range 返回（status=$($probe.StatusCode)）" }
        $total = [int64]$Matches[1]

        # 2) 取尾部 256 KB 解析中央目录
        $tailLen = [Math]::Min(262144, $total)
        $tail = Get-RangeBytes -Url $info.Url -Start ($total - $tailLen) -End ($total - 1)
        $entry = Find-ZipEntry -Bytes $tail -Name $info.Entry
        if (-not $entry) { throw "包内未找到 $($info.Entry)" }
        if ($entry.CompressedSize -le 0 -or $entry.LocalOffset -le 0) { throw "条目使用了 ZIP64/流式布局，无法按偏移量取值（请用 -Full）" }

        # 3) 读本地头（30 字节 + 文件名/扩展字段）确定数据起点
        $header = Get-RangeBytes -Url $info.Url -Start $entry.LocalOffset -End ($entry.LocalOffset + 4095)
        if ($header[0] -ne 0x50 -or $header[1] -ne 0x4B -or $header[2] -ne 0x03 -or $header[3] -ne 0x04) { throw "本地头签名异常（请用 -Full）" }
        $localNameLen = [int]$header[26] -bor ([int]$header[27] -shl 8)
        $localExtraLen = [int]$header[28] -bor ([int]$header[29] -shl 8)
        $dataStart = $entry.LocalOffset + 30 + $localNameLen + $localExtraLen

        # 4) 只拉该条目的压缩数据（约 6 MB）
        $payload = Get-RangeBytes -Url $info.Url -Start $dataStart -End ($dataStart + $entry.CompressedSize - 1)
        Write-Host ("已按 Range 取到条目：{0} 字节（压缩后），整包 {1} MB" -f $entry.CompressedSize, [math]::Round($total / 1MB, 1))
        $dllBytes = if ($entry.Method -eq 0) { $payload } else { Expand-DeflateBytes -Data $payload }
    }
    catch {
        Write-Warning "Range 取值失败（$($_.Exception.Message)），退化为整包下载"
        $dllBytes = $null
    }
}

if (-not $dllBytes) {
    # 退路：整包下载后在临时目录解压出目标条目
    $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("codexui-ort-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $tmpDir | Out-Null
    $pkg = Join-Path $tmpDir "onnxruntime.pkg.zip"
    Write-Host "整包下载中（可能较慢）…"
    Invoke-WebRequest -UseBasicParsing $info.Url -OutFile $pkg -TimeoutSec 1800
    $archive = [System.IO.Compression.ZipFile]::OpenRead($pkg)
    try {
        $zipEntry = $archive.Entries | Where-Object { $_.FullName -eq $info.Entry } | Select-Object -First 1
        if (-not $zipEntry) { throw "包内未找到 $($info.Entry)" }
        $stream = $zipEntry.Open()
        try {
            $ms = New-Object System.IO.MemoryStream
            $stream.CopyTo($ms)
            $dllBytes = $ms.ToArray()
            $ms.Dispose()
        }
        finally { $stream.Dispose() }
    }
    finally {
        $archive.Dispose()
        [System.IO.File]::Delete($pkg)
        [System.IO.Directory]::Delete($tmpDir, $true)
    }
}

if (-not $dllBytes -or $dllBytes.Length -lt 1MB) {
    throw "取到的 onnxruntime.dll 异常（$($dllBytes.Length) 字节）"
}

New-Item -ItemType Directory -Path (Split-Path -Parent $OutFile) -Force | Out-Null
[System.IO.File]::WriteAllBytes($OutFile, $dllBytes)

$versionInfo = (Get-Item $OutFile).VersionInfo
$fileVersion = $versionInfo.FileVersion
$sizeMb = [math]::Round((Get-Item $OutFile).Length / 1MB, 2)
Write-Host "已写入 $OutFile（$sizeMb MB，FileVersion=$fileVersion，ProductVersion=$($versionInfo.ProductVersion)）"

# 版本校验：ORT 的 FileVersion 形如 `1.30.0.20260909.8.f2c39fe`（段数不固定），
# 因此按前两段（major.minor）比较，而不是整串 [version] 解析。
$versionText = "$($versionInfo.ProductVersion) $fileVersion"
$match = [regex]::Match($versionText, "(\d+)\.(\d+)")
if (-not $match.Success) {
    Write-Warning "无法从 '$versionText' 解析版本号，请自行确认 >= $MinVersion"
}
else {
    $major = [int]$match.Groups[1].Value
    $minor = [int]$match.Groups[2].Value
    if ($major -lt $MinVersion.Major -or ($major -eq $MinVersion.Major -and $minor -lt $MinVersion.Minor)) {
        throw "版本过低：$major.$minor < $MinVersion（ort 的 api-24 需要 ONNX Runtime 1.24+）"
    }
    Write-Host "版本校验通过：$major.$minor >= $MinVersion"
}

if ($AlsoDev) {
    foreach ($profile in @("debug", "release")) {
        $devBin = Join-Path $repoRoot "src-tauri/target/$profile/bin"
        if (Test-Path (Join-Path $repoRoot "src-tauri/target/$profile")) {
            New-Item -ItemType Directory -Path $devBin -Force | Out-Null
            Copy-Item -LiteralPath $OutFile -Destination (Join-Path $devBin "onnxruntime.dll") -Force
            Write-Host "已复制到 $devBin\onnxruntime.dll"
        }
    }
}
