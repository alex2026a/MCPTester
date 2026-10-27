# MCP Tester 构建入口
#
# 用法：
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1 -Target portable
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1 -Target portable -SkipBuild
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1 -Target msi
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1 -Target msi -SkipBuild
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1 -Target all -Verify
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1 -Target portable -SizeOptimized
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1 -Target all -Verify -SizeOptimized
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1 -Target clean
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1 -Target clean -CleanMode full
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1 -Target rebuild
#
# 产物统一归档到 artifacts/<version>/，该目录及 zip/exe/msi 均不入 Git。

[CmdletBinding()]
param(
    [ValidateSet('portable', 'msi', 'all', 'clean', 'rebuild')]
    [string]$Target = 'portable',

    [ValidateSet('soft', 'app', 'full')]
    [string]$CleanMode = 'soft',

    [switch]$IncludeConfig,
    [switch]$Verify,
    [switch]$RequireClean,
    [switch]$SkipBuild,
    # 体积优先 profile：fat LTO + codegen-units=1 + opt-level=s，代价是全量重编
    [switch]$SizeOptimized,
    [string]$OutputDir = 'artifacts'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $root

function Write-Step {
    param([string]$Message)
    Write-Host ''
    Write-Host "==> $Message" -ForegroundColor Cyan
}

function Stop-Build {
    param([string]$Message)
    throw $Message
}

function Push-ProfileEnv {
    param([hashtable]$Values)
    $previous = @{}
    foreach ($key in $Values.Keys) {
        $previous[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
        [Environment]::SetEnvironmentVariable($key, [string]$Values[$key], 'Process')
    }
    return $previous
}

function Pop-ProfileEnv {
    param([hashtable]$Previous)
    foreach ($key in $Previous.Keys) {
        [Environment]::SetEnvironmentVariable($key, $Previous[$key], 'Process')
    }
}

function Remove-PathSafe {
    param([string]$Path)
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
    if (-not $item) {
        return
    }
    if ($item.PSIsContainer) {
        Remove-Item -LiteralPath $Path -Recurse -Force
    } else {
        Remove-Item -LiteralPath $Path -Force
    }
}

function Assert-Command {
    param([string]$Name)
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        Stop-Build "未找到命令 $Name，请先安装并加入 PATH。"
    }
}

function Invoke-Native {
    param(
        [string]$Description,
        [scriptblock]$Command
    )
    & $Command
    if ($LASTEXITCODE -ne 0) {
        Stop-Build "$Description 失败（退出码 $LASTEXITCODE）。"
    }
}

function Get-Sha256Hex {
    param([string]$Path)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $stream = [System.IO.File]::OpenRead($Path)
        try {
            $bytes = $sha.ComputeHash($stream)
        } finally {
            $stream.Dispose()
        }
    } finally {
        $sha.Dispose()
    }
    return ([BitConverter]::ToString($bytes) -replace '-', '').ToLowerInvariant()
}

function Clear-FrontendOutputs {
    Remove-PathSafe (Join-Path $root 'dist')
    Remove-PathSafe (Join-Path $root 'dist-portable')
    Remove-PathSafe (Join-Path $root 'MCPTester-portable.zip')
}

function Invoke-CargoClean {
    param([string]$Mode)
    if ($Mode -eq 'soft') {
        return
    }
    Push-Location (Join-Path $root 'src-tauri')
    try {
        if ($Mode -eq 'full') {
            Invoke-Native 'cargo clean' { & cargo clean }
        } else {
            Invoke-Native 'cargo clean -p mcptester --release' { & cargo clean -p mcptester --release }
        }
    } finally {
        Pop-Location
    }
}

function Invoke-Verify {
    Write-Step '质量校验'
    Invoke-Native 'pnpm lint' { & pnpm lint }
    Invoke-Native 'pnpm test' { & pnpm test }
    Push-Location (Join-Path $root 'src-tauri')
    try {
        Invoke-Native 'cargo test --features mock-server' { & cargo test --features mock-server }
        Invoke-Native 'cargo clippy --all-targets' { & cargo clippy --all-targets }
    } finally {
        Pop-Location
    }
}

# ---------------------------------------------------------------------------
# 环境与版本
# ---------------------------------------------------------------------------

Write-Step '检查构建环境'
foreach ($command in @('node', 'pnpm', 'cargo', 'rustc')) {
    Assert-Command $command
}

$nodeVersion = (& node -v | Out-String).Trim()
$pnpmVersion = (& pnpm -v | Out-String).Trim()
$rustcVersion = (& rustc -V | Out-String).Trim()

$nodeMajor = 0
if ($nodeVersion -match '^v(\d+)') {
    $nodeMajor = [int]$Matches[1]
}
if ($nodeMajor -lt 20) {
    Stop-Build "需要 Node 20+，当前为 $nodeVersion。"
}
Write-Host "Node $nodeVersion / pnpm $pnpmVersion / $rustcVersion"

Write-Step '校验版本号'
$packageJson = Get-Content -LiteralPath (Join-Path $root 'package.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$tauriJson = Get-Content -LiteralPath (Join-Path $root 'src-tauri\tauri.conf.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$version = [string]$tauriJson.version
if (-not $version) {
    Stop-Build 'src-tauri/tauri.conf.json 中未找到 version。'
}
if ([string]$packageJson.version -ne $version) {
    Stop-Build "版本不一致：package.json=$($packageJson.version)，tauri.conf.json=$version。"
}

$cargoToml = Get-Content -LiteralPath (Join-Path $root 'src-tauri\Cargo.toml') -Raw -Encoding UTF8
if ($cargoToml -notmatch '(?m)^version\s*=\s*"([^"]+)"') {
    Stop-Build 'src-tauri/Cargo.toml 中未找到 package version。'
}
$cargoVersion = $Matches[1]
if ($cargoVersion -ne $version) {
    Stop-Build "版本不一致：src-tauri/Cargo.toml=$cargoVersion，tauri.conf.json=$version。"
}
Write-Host "版本：$version"

# 体积优先构建单独归档，避免覆盖快速构建的产物
if ($SizeOptimized -and -not $PSBoundParameters.ContainsKey('OutputDir')) {
    $OutputDir = 'artifacts-size'
}

if (-not [IO.Path]::IsPathRooted($OutputDir)) {
    $OutputDir = Join-Path $root $OutputDir
}
$artifactDir = Join-Path $OutputDir $version

# ---------------------------------------------------------------------------
# Git 信息与前置检查
# ---------------------------------------------------------------------------

$commit = 'unknown'
$dirty = 'unknown'
if (Get-Command git -ErrorAction SilentlyContinue) {
    $commitOutput = @(& git -C $root rev-parse --short HEAD 2>$null)
    if ($LASTEXITCODE -eq 0 -and $commitOutput.Count -gt 0) {
        $commit = $commitOutput[0].Trim()
    }

    $statusOutput = @(& git -C $root status --porcelain 2>$null)
    if ($LASTEXITCODE -eq 0) {
        if ($statusOutput.Count -gt 0) {
            $dirty = 'true'
        } else {
            $dirty = 'false'
        }
    }
}

if ($RequireClean -and $Target -ne 'clean' -and $dirty -ne 'false') {
    Stop-Build "Git 工作区存在未提交改动（dirty=$dirty），正式发版请先提交，或去掉 -RequireClean。"
}

if ($Target -ne 'clean') {
    $running = @(Get-Process -Name 'mcptester' -ErrorAction SilentlyContinue)
    if ($running.Count -gt 0) {
        $pids = ($running | ForEach-Object { $_.Id }) -join ', '
        Stop-Build "检测到 mcptester 正在运行（PID: $pids）。请先退出应用；不要直接运行 target\release\mcptester.exe。"
    }
}

if (-not (Test-Path -LiteralPath (Join-Path $root 'node_modules'))) {
    Write-Step '安装前端依赖'
    Invoke-Native 'pnpm install --frozen-lockfile' { & pnpm install --frozen-lockfile }
}

# ---------------------------------------------------------------------------
# 清理任务
# ---------------------------------------------------------------------------

$effectiveClean = $CleanMode
if ($Target -eq 'rebuild' -and -not $PSBoundParameters.ContainsKey('CleanMode')) {
    $effectiveClean = 'app'
}

if ($SkipBuild -and $Target -in @('clean', 'rebuild')) {
    Stop-Build '-SkipBuild 不支持 -Target clean / rebuild。'
}
if ($SkipBuild -and $effectiveClean -ne 'soft') {
    Stop-Build '-SkipBuild 只支持 -CleanMode soft，避免清理掉现有 release exe。'
}
if ($SkipBuild -and $SizeOptimized) {
    Stop-Build '-SkipBuild 无法保证现有 exe 是体积优化产物；-SizeOptimized 必须重新编译。'
}

if ($Target -eq 'clean') {
    Write-Step "清理（$effectiveClean）"
    Clear-FrontendOutputs
    Remove-PathSafe (Join-Path $root 'src-tauri\target\release\bundle')
    Remove-PathSafe $artifactDir
    # 两个构建档位的同版本产物都清掉，避免残留混淆
    foreach ($name in @('artifacts', 'artifacts-size')) {
        Remove-PathSafe (Join-Path (Join-Path $root $name) $version)
    }
    Invoke-CargoClean $effectiveClean
    Write-Host '清理完成。' -ForegroundColor Green
    return
}

# ---------------------------------------------------------------------------
# 质量校验（在编译前执行，避免打包失败代码）
# ---------------------------------------------------------------------------

if ($Verify) {
    Invoke-Verify
}

# ---------------------------------------------------------------------------
# 清理旧产物
# ---------------------------------------------------------------------------

$buildKind = $Target
if ($Target -eq 'rebuild') {
    $buildKind = 'all'
}

Write-Step "准备构建：$buildKind（清理：$effectiveClean）"
Clear-FrontendOutputs

if ($buildKind -in @('msi', 'all')) {
    # 避免把 bundle/msi 中的历史 MSI 误当成本次产物
    Remove-PathSafe (Join-Path $root 'src-tauri\target\release\bundle')
}

if ($buildKind -eq 'all') {
    Remove-PathSafe $artifactDir
} elseif ($buildKind -eq 'portable') {
    Remove-PathSafe (Join-Path $artifactDir "MCPTester-portable-$version.zip")
} elseif ($buildKind -eq 'msi' -and (Test-Path -LiteralPath $artifactDir)) {
    Get-ChildItem -LiteralPath $artifactDir -Filter '*.msi' -File -ErrorAction SilentlyContinue |
        ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force }
}

Invoke-CargoClean $effectiveClean

# ---------------------------------------------------------------------------
# 编译
# ---------------------------------------------------------------------------

$exe = Join-Path $root 'src-tauri\target\release\mcptester.exe'
if ($SkipBuild) {
    Write-Warning "跳过编译，使用现有 release exe：$exe"
    if ($buildKind -in @('msi', 'all')) {
        Write-Step '使用现有 exe 生成 MSI'
        Invoke-Native 'pnpm tauri bundle --bundles msi' { & pnpm tauri bundle --bundles msi }
    }
} else {
    $sizeProfileEnv = @{
        CARGO_PROFILE_RELEASE_LTO           = 'fat'
        CARGO_PROFILE_RELEASE_CODEGEN_UNITS = '1'
        CARGO_PROFILE_RELEASE_OPT_LEVEL     = 's'
    }
    $previousProfileEnv = $null
    try {
        if ($SizeOptimized) {
            Write-Step '体积优先：fat LTO + codegen-units=1 + opt-level=s（全量重编，耗时明显增加）'
            $previousProfileEnv = Push-ProfileEnv $sizeProfileEnv
        }
        Write-Step "编译（$buildKind）"
        if ($buildKind -eq 'portable') {
            Invoke-Native 'pnpm tauri build --no-bundle' { & pnpm tauri build --no-bundle }
        } elseif ($buildKind -eq 'msi') {
            Invoke-Native 'pnpm tauri build --bundles msi' { & pnpm tauri build --bundles msi }
        } else {
            Invoke-Native 'pnpm tauri build' { & pnpm tauri build }
        }
    } finally {
        if ($null -ne $previousProfileEnv) {
            Pop-ProfileEnv $previousProfileEnv
        }
    }
}

if (-not (Test-Path -LiteralPath $exe)) {
    Stop-Build "编译后未找到 $exe。"
}
$exeVersion = [string](Get-Item -LiteralPath $exe).VersionInfo.ProductVersion
if ($exeVersion -and $exeVersion -ne $version -and $exeVersion -ne "$version.0") {
    Stop-Build "exe 版本 $exeVersion 与配置版本 $version 不一致，请重新编译。"
}

# ---------------------------------------------------------------------------
# 归档
# ---------------------------------------------------------------------------

New-Item -ItemType Directory -Path $artifactDir -Force | Out-Null

if ($buildKind -in @('msi', 'all')) {
    $msiDir = Join-Path $root 'src-tauri\target\release\bundle\msi'
    $msi = Get-ChildItem -LiteralPath $msiDir -Filter '*.msi' -File -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if (-not $msi) {
        Stop-Build "未找到 MSI：$msiDir"
    }
    if ($msi.Name -notlike "*$version*") {
        Stop-Build "MSI 文件名 '$($msi.Name)' 未包含版本 $version，可能不是本次构建产物。"
    }
    $msiDest = Join-Path $artifactDir $msi.Name
    Copy-Item -LiteralPath $msi.FullName -Destination $msiDest -Force
    Write-Host "MSI: $msiDest"
}

if ($buildKind -in @('portable', 'all')) {
    $portableScript = Join-Path $PSScriptRoot 'portable.ps1'
    & $portableScript -Version $version -OutputDir $OutputDir -IncludeConfig:$IncludeConfig
    $zip = Join-Path $artifactDir "MCPTester-portable-$version.zip"
    if (-not (Test-Path -LiteralPath $zip)) {
        Stop-Build "便携版 zip 未生成：$zip"
    }
    Write-Host "便携版: $zip"
}

$builtAt = (Get-Date).ToString('yyyy-MM-ddTHH:mm:sszzz')
$profileLabel = if ($SizeOptimized) {
    'size (fat LTO, codegen-units=1, opt-level=s)'
} else {
    'speed (thin LTO, codegen-units=16, opt-level=3)'
}
$buildInfo = @(
    'Product: MCP Tester',
    "Version: $version",
    "Target: $Target",
    "BuildKind: $buildKind",
    "CleanMode: $effectiveClean",
    "Commit: $commit",
    "Dirty: $dirty",
    "BuiltAt: $builtAt",
    "Node: $nodeVersion",
    "pnpm: $pnpmVersion",
    "Rust: $rustcVersion",
    "Verified: $($Verify.IsPresent)",
    "SkippedBuild: $($SkipBuild.IsPresent)",
    "SizeOptimized: $($SizeOptimized.IsPresent)",
    "CargoProfile: $profileLabel"
)
Set-Content -LiteralPath (Join-Path $artifactDir 'build-info.txt') -Value $buildInfo -Encoding UTF8

$checksumFiles = @(
    Get-ChildItem -LiteralPath $artifactDir -File |
        Where-Object { $_.Extension -in @('.msi', '.zip') } |
        Sort-Object Name
)
if ($checksumFiles.Count -eq 0) {
    Stop-Build "产物目录中没有 MSI/ZIP：$artifactDir"
}
$checksumLines = foreach ($file in $checksumFiles) {
    $hash = Get-Sha256Hex $file.FullName
    "$hash  $($file.Name)"
}
Set-Content -LiteralPath (Join-Path $artifactDir 'SHA256SUMS.txt') -Value $checksumLines -Encoding ASCII

Write-Host ''
Write-Host "构建完成：$artifactDir" -ForegroundColor Green
Get-ChildItem -LiteralPath $artifactDir |
    Sort-Object Name |
    Format-Table Name, @{ Name = 'SizeMB'; Expression = { [math]::Round($_.Length / 1MB, 2) } }, LastWriteTime -AutoSize
