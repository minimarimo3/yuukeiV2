[CmdletBinding()]
param(
    [string]$ModelPath = "",
    [string]$OutputPath = ""
)

$ErrorActionPreference = "Stop"

$packageRoot = Split-Path -Parent $PSScriptRoot
$repoRoot = (Resolve-Path (Join-Path $packageRoot "..\..")).Path
$defaultModel = Join-Path $env:USERPROFILE "Downloads\models\gemma-4-e4b\gemma-4-E4B-it.litertlm"
if (-not $ModelPath) {
    $ModelPath = if ($env:YUUKEI_LITERT_MODEL_SOURCE) {
        $env:YUUKEI_LITERT_MODEL_SOURCE
    } else {
        $defaultModel
    }
}
$resolvedModel = (Resolve-Path -LiteralPath $ModelPath).Path

$distRoot = Join-Path $packageRoot "dist"
if (-not $OutputPath) {
    $OutputPath = Join-Path $distRoot "yuukei-intelligence"
}
$outputParent = Split-Path -Parent $OutputPath
New-Item -ItemType Directory -Force -Path $outputParent | Out-Null
$resolvedOutputParent = (Resolve-Path -LiteralPath $outputParent).Path
$resolvedDistRoot = (Resolve-Path -LiteralPath $distRoot).Path
if (-not $resolvedOutputParent.StartsWith($resolvedDistRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "OutputPath must stay inside $resolvedDistRoot"
}
$fullOutput = [IO.Path]::GetFullPath($OutputPath)
if (Test-Path -LiteralPath $fullOutput) {
    Remove-Item -LiteralPath $fullOutput -Recurse -Force
}

$expectedModelSha256 = "0b2a8980ce155fd97673d8e820b4d29d9c7d99b8fa6806f425d969b145bd52e0"
$actualModelSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $resolvedModel).Hash.ToLowerInvariant()
if ($actualModelSha256 -ne $expectedModelSha256) {
    throw "Unexpected model SHA-256: $actualModelSha256"
}

Push-Location $repoRoot
try {
    cargo build --release -p yuukei-intelligence
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

$temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) ("yuukei-litert-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $temporaryRoot | Out-Null
try {
    $wheelUrl = "https://files.pythonhosted.org/packages/d9/ef/36831a18c29b5a8f8283578c77fdd0c823dc8851b05f7a12689ac603c599/litert_lm_api-0.14.0-py3-none-win_amd64.whl"
    $wheelSha256 = "b20c555a74f1e15bbe988d86a2fb7316603779fbb71a3c085aa9dbcd5d39c2e3"
    $wheelPath = Join-Path $temporaryRoot "litert-lm-api.zip"
    Invoke-WebRequest -Uri $wheelUrl -OutFile $wheelPath
    $actualWheelSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $wheelPath).Hash.ToLowerInvariant()
    if ($actualWheelSha256 -ne $wheelSha256) {
        throw "Unexpected LiteRT-LM wheel SHA-256: $actualWheelSha256"
    }
    $wheelRoot = Join-Path $temporaryRoot "wheel"
    Expand-Archive -LiteralPath $wheelPath -DestinationPath $wheelRoot

    $binPath = Join-Path $fullOutput "bin"
    $modelOutputPath = Join-Path $fullOutput "model"
    $licensePath = Join-Path $fullOutput "licenses"
    New-Item -ItemType Directory -Force -Path $binPath, $modelOutputPath, $licensePath | Out-Null

    Copy-Item -LiteralPath (Join-Path $repoRoot "target\release\yuukei-intelligence.exe") -Destination $binPath
    foreach ($runtimeFile in @("litert-lm.dll", "dxcompiler.dll", "dxil.dll")) {
        Copy-Item -LiteralPath (Join-Path $wheelRoot "litert_lm\$runtimeFile") -Destination $binPath
    }
    Copy-Item -LiteralPath $resolvedModel -Destination (Join-Path $modelOutputPath "gemma-4-E4B-it.litertlm")
    Copy-Item -LiteralPath (Join-Path $packageRoot "manifest.json") -Destination $fullOutput
    Copy-Item -LiteralPath (Join-Path $packageRoot "README.md") -Destination $fullOutput
    Copy-Item -LiteralPath (Join-Path $packageRoot "THIRD_PARTY_NOTICES.md") -Destination $fullOutput
    Invoke-WebRequest `
        -Uri "https://raw.githubusercontent.com/google-ai-edge/LiteRT-LM/v0.14.0/LICENSE" `
        -OutFile (Join-Path $licensePath "Apache-2.0.txt")

    $outputPrefix = $fullOutput.TrimEnd("\") + "\"
    $files = Get-ChildItem -LiteralPath $fullOutput -Recurse -File | ForEach-Object {
        [ordered]@{
            path = $_.FullName.Substring($outputPrefix.Length).Replace("\", "/")
            size = $_.Length
            sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant()
        }
    }
    [ordered]@{
        package = "yuukei-intelligence"
        version = "0.1.0"
        litertLmVersion = "0.14.0"
        model = "gemma-4-E4B-it"
        files = @($files)
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $fullOutput "package-manifest.json") -Encoding utf8
} finally {
    if (Test-Path -LiteralPath $temporaryRoot) {
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
    }
}

Write-Output $fullOutput
