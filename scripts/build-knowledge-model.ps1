# 生成知识库向量模型归档（随安装包分发到 <应用目录>/marketplaces/knowledge-model.tar.gz）。
#
# 归档结构（顶层目录名必须与 src-tauri/src/codex/knowledge/paths.rs 的 MODEL_ID 一致）：
#   bge-small-zh-v1.5/{model.onnx, tokenizer.json, config.json, special_tokens_map.json, tokenizer_config.json}
#   bge-small-zh-v1.5/.materialization-key   # {"appVersion": "<版本>"}，供启动期版本比对
#
# 用法：
#   pwsh -File scripts/build-knowledge-model.ps1                     # 默认量化版（推荐，约 24 MB）
#   pwsh -File scripts/build-knowledge-model.ps1 -Variant fp32      # fp32（约 95 MB，效果相同、体积更大）
#
# 说明：codex-ui 运行期从该归档解压到 <app data dir>/knowledge/model/，全程离线；
# 换模型/换维度需同时改 paths.rs 的 MODEL_ID / EMBED_DIM，并删除已有知识库重建。

param(
    [string]$Repo = "Xenova/bge-small-zh-v1.5",
    [string]$Top = "bge-small-zh-v1.5",
    [ValidateSet("int8", "fp32")]
    [string]$Variant = "int8",
    [string]$OutFile = "",
    [string]$AppVersion = "1.0.0"
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $OutFile) {
    $OutFile = Join-Path $repoRoot "setup/marketplaces/knowledge-model.tar.gz"
}

$onnxSource = if ($Variant -eq "int8") { "onnx/model_quantized.onnx" } else { "onnx/model.onnx" }
$baseUrl = "https://huggingface.co/$Repo/resolve/main"

# 归档内文件名固定为模型目录约定（int8 也落成 model.onnx，运行期不区分来源）
$files = [ordered]@{
    $onnxSource                    = "model.onnx"
    "tokenizer.json"               = "tokenizer.json"
    "config.json"                  = "config.json"
    "special_tokens_map.json"      = "special_tokens_map.json"
    "tokenizer_config.json"        = "tokenizer_config.json"
}

$staging = Join-Path ([System.IO.Path]::GetTempPath()) ("codexui-knowledge-model-" + [guid]::NewGuid().ToString("N"))
$target = Join-Path $staging $Top
New-Item -ItemType Directory -Path $target -Force | Out-Null

try {
    foreach ($src in $files.Keys) {
        $dest = Join-Path $target $files[$src]
        Write-Host "下载 $src → $($files[$src])"
        Invoke-WebRequest -UseBasicParsing "$baseUrl/$src" -OutFile $dest -TimeoutSec 600
        $size = [math]::Round((Get-Item $dest).Length / 1MB, 2)
        Write-Host "  $size MB"
    }

    # 物化键：与该归档版本比对（仅升级、不降级），沿用 bundled.rs 的约定
    (@{ appVersion = $AppVersion } | ConvertTo-Json -Compress) | Set-Content -Path (Join-Path $target ".materialization-key") -Encoding utf8

    New-Item -ItemType Directory -Path (Split-Path -Parent $OutFile) -Force | Out-Null
    if (Test-Path $OutFile) { Remove-Item -LiteralPath $OutFile -Force }
    # Windows 自带 bsdtar；-C 保证归档顶层目录为 $Top
    tar -czf $OutFile -C $staging $Top
    $total = [math]::Round((Get-Item $OutFile).Length / 1MB, 2)
    Write-Host "已生成 $OutFile（$total MB）"
}
finally {
    if (Test-Path $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
}
