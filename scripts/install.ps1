# Install codex-mcp-skills on Windows from GitHub releases (uv-style).
# Usage:
#   powershell -ExecutionPolicy ByPass -c "irm https://raw.githubusercontent.com/$Env:CODEX_SKILLS_GH_REPO/HEAD/scripts/install.ps1 | iex"
# Env overrides:
#   CODEX_SKILLS_GH_REPO   owner/repo (default: codex-mcp-skills/codex-mcp-skills)
#   CODEX_SKILLS_VERSION   release tag without leading v (default: latest)
#   CODEX_SKILLS_BIN_DIR   install directory (default: $HOME\.codex\bin)
#   CODEX_SKILLS_BIN_NAME  binary name (default: codex-mcp-skills.exe)
#   CODEX_SKILLS_TARGET    target triple override (default: x86_64-pc-windows-msvc or aarch64-pc-windows-msvc)

$ErrorActionPreference = "Stop"

function Fail($msg) { Write-Error $msg; exit 1 }
function Require($cmd) { if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) { Fail "Missing required command: $cmd" } }

function Get-OsArchTarget {
    $arch = $env:CODEX_SKILLS_TARGET
    if ($arch) { return $arch }
    $os = "windows"
    $cpu = (Get-CimInstance Win32_Processor).Architecture
    switch ($cpu) {
        9 { $archName = "x86_64" } # x64
        12 { $archName = "aarch64" } # arm64
        default { Fail "Unsupported CPU architecture: $cpu" }
    }
    return "$archName-pc-windows-msvc"
}

function Get-Repo {
    if ($env:CODEX_SKILLS_GH_REPO) { return $env:CODEX_SKILLS_GH_REPO }
    return "athola/codex-mcp-skills"
}

function Get-ApiUrl {
    $repo = Get-Repo
    if ($env:CODEX_SKILLS_VERSION) {
        return "https://api.github.com/repos/$repo/releases/tags/v$($env:CODEX_SKILLS_VERSION)"
    }
    return "https://api.github.com/repos/$repo/releases/latest"
}

function Select-AssetUrl {
    Require "curl"
    $apiUrl = Get-ApiUrl
    $json = curl -fsSL $apiUrl
    $target = Get-OsArchTarget
    $obj = $json | ConvertFrom-Json
    foreach ($a in $obj.assets) {
        if ($a.name -like "*${target}*") {
            return $a.browser_download_url
        }
    }
    Fail "No asset found for target $target"
}

function Download-And-Extract($url, $binDir, $binName) {
    Require "curl"
    Require "tar"
    $tmp = New-Item -ItemType Directory -Path ([IO.Path]::GetTempPath()) -Name ("codex-install-" + [IO.Path]::GetRandomFileName())
    try {
        $archive = Join-Path $tmp "pkg.tar.gz"
        curl -fL $url -o $archive
        $out = Join-Path $tmp "out"
        New-Item -ItemType Directory -Path $out | Out-Null
        tar -xzf $archive -C $out
        if (-not (Test-Path $binDir)) { New-Item -ItemType Directory -Path $binDir | Out-Null }
        $candidate = Get-ChildItem -Path $out -Recurse -Filter $binName | Select-Object -First 1
        if (-not $candidate) { Fail "Binary $binName not found in archive" }
        Copy-Item $candidate.FullName (Join-Path $binDir $binName) -Force
    }
    finally { Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue }
}

$binName = if ($env:CODEX_SKILLS_BIN_NAME) { $env:CODEX_SKILLS_BIN_NAME } else { "codex-mcp-skills.exe" }
$binDir = if ($env:CODEX_SKILLS_BIN_DIR) { $env:CODEX_SKILLS_BIN_DIR } else { Join-Path $HOME ".codex/bin" }
$assetUrl = Select-AssetUrl
Download-And-Extract $assetUrl $binDir $binName
Write-Output "Installed $binName to $binDir"
if (-not ($env:PATH -split ';' | Where-Object { $_ -eq $binDir })) {
    Write-Output "Add $binDir to PATH (setx PATH \"$binDir;%PATH%\")"
}
