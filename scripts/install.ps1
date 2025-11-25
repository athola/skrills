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
    # Explicitly call native curl.exe to avoid the PowerShell alias that
    # maps "curl" to Invoke-WebRequest (which doesn't support -fsSL).
    Require "curl.exe"
    $apiUrl = Get-ApiUrl
    $json = curl.exe -fsSL $apiUrl
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
    Require "curl.exe"
    Require "tar"
    $tmp = New-Item -ItemType Directory -Path ([IO.Path]::GetTempPath()) -Name ("codex-install-" + [IO.Path]::GetRandomFileName())
    try {
        $archive = Join-Path $tmp "pkg.tar.gz"
        curl.exe -fL $url -o $archive
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

# --- MCP registration (Codex MCP clients require type="stdio") ---
$mcpPath = Join-Path $HOME ".codex/mcp_servers.json"
$mcpDir = Split-Path $mcpPath -Parent
if (-not (Test-Path $mcpDir)) { New-Item -ItemType Directory -Path $mcpDir -Force | Out-Null }

$mcpJson = if (Test-Path $mcpPath) {
    Get-Content $mcpPath -Raw | ConvertFrom-Json
} else {
    [pscustomobject]@{ mcpServers = @{} }
}

if (-not $mcpJson.mcpServers) { $mcpJson | Add-Member -NotePropertyName mcpServers -NotePropertyValue @{} }
$mcpJson.mcpServers."codex-skills" = @{
    type    = "stdio"
    command = (Join-Path $binDir $binName)
    args    = @("serve")
}
$mcpJson | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 $mcpPath
Write-Output "Registered codex-skills MCP server in $mcpPath"

# Keep config.toml in sync (preferred by Codex)
$configToml = Join-Path $HOME ".codex/config.toml"
if (-not (Test-Path $configToml)) {
    if (-not (Test-Path (Split-Path $configToml -Parent))) { New-Item -ItemType Directory -Path (Split-Path $configToml -Parent) -Force | Out-Null }
    "model = ""gpt-5.1-codex-max`""" | Set-Content -Encoding UTF8 $configToml
}
$config = Get-Content $configToml -Raw
if ($config -notmatch '\[mcp_servers\."codex-skills"\]') {
    Add-Content -Encoding UTF8 $configToml @"

[mcp_servers."codex-skills"]
type = "stdio"
command = "$(Join-Path $binDir $binName)"
args = ["serve"]
"@
    Write-Output "Added codex-skills section to $configToml"
} elseif ($config -notmatch '(?ms)\[mcp_servers\."codex-skills"\].*type\s*=') {
    $updated = $config -replace '(\[mcp_servers\."codex-skills"\]\s*)', "`$1type = ""stdio""`n"
    $updated | Set-Content -Encoding UTF8 $configToml
    Write-Output "Ensured type = \"stdio\" in $configToml"
}
