# 生成知识库向量模型（直接铺文件，不打归档）：默认落到 setup\bin\model\bge-small-zh-v1.5\，
# 随安装包与 codexui-kb.exe / onnxruntime.dll 同目录分发（CLI 自包含，运行期只读、不做物化）。
#
# 目标结构（与 src-tauri/crates/knowledge-cli 的 MODEL_FILES 一致，缺一不可）：
#   bge-small-zh-v1.5/{model.onnx, tokenizer.json, config.json, special_tokens_map.json, tokenizer_config.json}
#
# 用法：
#   pwsh -File scripts/build-knowledge-model.ps1                # 默认量化版（约 23 MB，落 setup\bin\model）
#   pwsh -File scripts/build-knowledge-model.ps1 -Variant fp32 # fp32（约 95 MB，效果相同、体积更大）
#   pwsh -File scripts/build-knowledge-model.ps1 -AlsoDev      # 同时铺到 src-tauri\target\{debug,release}\model
#   pwsh -File scripts/build-knowledge-model.ps1 -Repo <镜像仓库> -OutDir <目录>
#
# 说明：模型文件由本脚本生成、不入库（.gitignore 已忽略 /setup/bin/model/）；换模型/换维度需同时改
# crates/knowledge-cli/src/paths.rs 的 MODEL_ID / EMBED_DIM，并删除已有知识库重建。

param(
    [string]$Repo = "Xenova/bge-small-zh-v1.5",
    [string]$ModelId = "bge-small-zh-v1.5",
    [ValidateSet("int8", "fp32")]
    [string]$Variant = "int8",
    [string]$OutDir = "",
    [switch]$AlsoDev,
    [switch]$Force
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) {
    $OutDir = Join-Path $repoRoot "setup/bin/model/$ModelId"
}

$onnxSource = if ($Variant -eq "int8") { "onnx/model_quantized.onnx" } else { "onnx/model.onnx" }
$baseUrl = "https://huggingface.co/$Repo/resolve/main"

# 归档内文件名固定为模型目录约定（int8 也落成 model.onnx，运行期不区分来源）
$files = [ordered]@{
    $onnxSource               = "model.onnx"
    "tokenizer.json"          = "tokenizer.json"
    "config.json"             = "config.json"
    "special_tokens_map.json" = "special_tokens_map.json"
    "tokenizer_config.json"   = "tokenizer_config.json"
}

function Get-ModelFiles {
    param([string]$Target)

    New-Item -ItemType Directory -Path $Target -Force | Out-Null
    $total = 0
    foreach ($src in $files.Keys) {
        $dest = Join-Path $Target $files[$src]
        if ((Test-Path $dest) -and -not $Force -and (Get-Item $dest).Length -gt 0) {
            Write-Host "已存在，跳过 $($files[$src])"
        }
        else {
            Write-Host "下载 $src → $($files[$src])"
            Invoke-WebRequest -UseBasicParsing "$baseUrl/$src" -OutFile $dest -TimeoutSec 600
        }
        $size = (Get-Item $dest).Length
        if ($size -le 0) { throw "模型文件为空：$dest" }
        $total += $size
    }
    return $total
}

$bytes = Get-ModelFiles -Target $OutDir
Write-Host ("已铺到 {0}（{1:N2} MB）" -f $OutDir, ($bytes / 1MB))

if ($AlsoDev) {
    foreach ($profile in @("debug", "release")) {
        $devDir = Join-Path $repoRoot "src-tauri/target/$profile/model/$ModelId"
        New-Item -ItemType Directory -Path $devDir -Force | Out-Null
        foreach ($name in $files.Values) {
            Copy-Item -LiteralPath (Join-Path $OutDir $name) -Destination (Join-Path $devDir $name) -Force
        }
        Write-Host "已复制到 $devDir"
    }
}
