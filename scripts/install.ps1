#Requires -Version 5.1
[CmdletBinding()]
param(
    [string]$Version = "v0.1.2",
    [string]$InstallDir = "",
    [switch]$SkipCodexSetup,
    [switch]$EnableCommandGuard,
    [switch]$NoPathUpdate
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    $localAppData = $env:LOCALAPPDATA
    if ([string]::IsNullOrWhiteSpace($localAppData)) {
        $localAppData = Join-Path $HOME "AppData\Local"
    }
    $InstallDir = Join-Path $localAppData "CDXCore\bin"
}

$InstallDir = [IO.Path]::GetFullPath($InstallDir)
$repo = "ikhdark/CDXCore"
$target = "x86_64-pc-windows-msvc"
$assetName = "cdxcore-$Version-$target.zip"
$releaseBase = "https://github.com/$repo/releases/download/$Version"
$zipUrl = "$releaseBase/$assetName"
$sumsUrl = "$releaseBase/SHA256SUMS.txt"

function Invoke-CDXCoreDownload {
    param(
        [Parameter(Mandatory = $true)][string]$Uri,
        [Parameter(Mandatory = $true)][string]$OutFile
    )

    Invoke-WebRequest -UseBasicParsing -Uri $Uri -OutFile $OutFile
}

function Add-CDXCoreUserPath {
    param([Parameter(Mandatory = $true)][string]$PathToAdd)

    $separator = [IO.Path]::PathSeparator
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $parts = @()
    if (-not [string]::IsNullOrWhiteSpace($userPath)) {
        $parts = $userPath -split [regex]::Escape($separator) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    }

    $alreadyPresent = $false
    foreach ($part in $parts) {
        if ([string]::Equals($part.TrimEnd('\'), $PathToAdd.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)) {
            $alreadyPresent = $true
            break
        }
    }

    if (-not $alreadyPresent) {
        $newUserPath = if ($parts.Count -gt 0) {
            ($parts + $PathToAdd) -join $separator
        } else {
            $PathToAdd
        }
        [Environment]::SetEnvironmentVariable("Path", $newUserPath, "User")
    }

    $currentParts = @()
    if (-not [string]::IsNullOrWhiteSpace($env:Path)) {
        $currentParts = $env:Path -split [regex]::Escape($separator) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    }
    $inCurrentProcess = $false
    foreach ($part in $currentParts) {
        if ([string]::Equals($part.TrimEnd('\'), $PathToAdd.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)) {
            $inCurrentProcess = $true
            break
        }
    }
    if (-not $inCurrentProcess) {
        $env:Path = "$PathToAdd$separator$env:Path"
    }
}

$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ("cdxcore-install-" + [guid]::NewGuid())
$zipPath = Join-Path $tempRoot $assetName
$sumsPath = Join-Path $tempRoot "SHA256SUMS.txt"
$extractDir = Join-Path $tempRoot "extract"

try {
    New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null
    New-Item -ItemType Directory -Force -Path $extractDir | Out-Null

    Write-Host "Downloading $assetName..."
    Invoke-CDXCoreDownload -Uri $zipUrl -OutFile $zipPath
    Invoke-CDXCoreDownload -Uri $sumsUrl -OutFile $sumsPath

    $sumLine = [IO.File]::ReadAllLines($sumsPath) | Where-Object { $_ -match [regex]::Escape($assetName) } | Select-Object -First 1
    if ([string]::IsNullOrWhiteSpace($sumLine)) {
        throw "Could not find $assetName in SHA256SUMS.txt."
    }
    $expectedHash = ($sumLine -split '\s+')[0].ToLowerInvariant()
    $actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zipPath).Hash.ToLowerInvariant()
    if ($actualHash -ne $expectedHash) {
        throw "Checksum mismatch for $assetName. Expected $expectedHash but got $actualHash."
    }

    Expand-Archive -LiteralPath $zipPath -DestinationPath $extractDir -Force

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Copy-Item -LiteralPath (Join-Path $extractDir "cdxcore.exe") -Destination (Join-Path $InstallDir "cdxcore.exe") -Force
    foreach ($name in @("README.md", "LICENSE")) {
        $source = Join-Path $extractDir $name
        if (Test-Path -LiteralPath $source) {
            Copy-Item -LiteralPath $source -Destination (Join-Path $InstallDir $name) -Force
        }
    }
    $schemaSource = Join-Path $extractDir "schemas"
    if (Test-Path -LiteralPath $schemaSource) {
        Copy-Item -LiteralPath $schemaSource -Destination $InstallDir -Recurse -Force
    }

    if (-not $NoPathUpdate) {
        Add-CDXCoreUserPath -PathToAdd $InstallDir
    } else {
        $env:Path = "$InstallDir$([IO.Path]::PathSeparator)$env:Path"
    }

    $exe = Join-Path $InstallDir "cdxcore.exe"
    & $exe --version

    if (-not $SkipCodexSetup) {
        $setupArgs = @("setup", "codex")
        if ($EnableCommandGuard) {
            $setupArgs += "--enable-command-guard"
        }
        & $exe @setupArgs
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "CDXCore was installed, but Codex setup did not complete. Run 'cdxcore setup codex' after Codex is available on PATH."
        }
    }

    Write-Host "Installed CDXCore to $InstallDir"
    Write-Host "Open a new terminal or restart Codex if it does not see the updated PATH."
} finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}
