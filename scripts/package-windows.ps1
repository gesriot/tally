<#
.SYNOPSIS
Builds the release tally-ui.exe for Windows with the application icon embedded.

.DESCRIPTION
The window icon is set in the .slint files, but Explorer, shortcuts and the
taskbar read the icon from the executable's own resources. This script
compiles assets/icon.ico into a Windows resource (rc.exe from the Windows
SDK), links it into the release build, and copies the single exe to
dist/windows. No archive is created.

assets/icon.ico is committed; regenerate it from assets/icon.png with
scripts/make-icons.py.

The script is ASCII-only on purpose: Windows PowerShell 5.1 reads a UTF-8
file without BOM as ANSI and chokes on non-ASCII text.

.PARAMETER SkipBuild
Skip the Cargo build and only copy an already built executable.

.PARAMETER Target
Optional Rust target triple, for example x86_64-pc-windows-msvc.

.PARAMETER DistDir
Output folder (default: dist/windows).

.EXAMPLE
scripts/package-windows.ps1
#>

[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [string]$Target,
    [string]$DistDir
)

$ErrorActionPreference = "Stop"

function Invoke-Checked {
    param([string]$FilePath, [string[]]$Arguments, [string]$WorkingDirectory)

    Push-Location $WorkingDirectory
    try {
        & $FilePath @Arguments
        if ($LASTEXITCODE -ne 0) {
            throw "$FilePath failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
    }
}

function Get-RustHostTriple {
    $rustc = Get-Command rustc -ErrorAction SilentlyContinue
    if (-not $rustc) {
        throw "rustc was not found in PATH."
    }
    foreach ($line in (& $rustc.Source -vV)) {
        if ($line -match "^host:\s+(.+)$") {
            return $Matches[1]
        }
    }
    throw "Could not determine the rustc host triple."
}

function Get-RcArch {
    param([string]$Triple)

    if ($Triple -match "aarch64|arm64") { return "arm64" }
    if ($Triple -match "i686|i586") { return "x86" }
    return "x64"
}

function Find-RcExe {
    param([string]$Arch)

    $cmd = Get-Command rc.exe -ErrorAction SilentlyContinue
    if ($cmd) {
        return $cmd.Source
    }

    $roots = @()
    foreach ($programFiles in @([Environment]::GetEnvironmentVariable("ProgramFiles(x86)"),
                                [Environment]::GetEnvironmentVariable("ProgramFiles"))) {
        if ($programFiles) {
            $roots += Join-Path $programFiles "Windows Kits\10\bin"
        }
    }

    $archOrder = @($Arch, "x64", "x86", "arm64") | Select-Object -Unique
    foreach ($root in $roots) {
        if (-not (Test-Path -LiteralPath $root)) {
            continue
        }
        $versionDirs = Get-ChildItem -LiteralPath $root -Directory -ErrorAction SilentlyContinue |
            Sort-Object Name -Descending
        foreach ($versionDir in $versionDirs) {
            foreach ($candidateArch in $archOrder) {
                $candidate = Join-Path $versionDir.FullName (Join-Path $candidateArch "rc.exe")
                if (Test-Path -LiteralPath $candidate) {
                    return $candidate
                }
            }
        }
    }

    throw "rc.exe was not found. Install the Windows SDK or run this from a Visual Studio Developer PowerShell."
}

$rootPath = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if (-not $DistDir) {
    $DistDir = Join-Path $rootPath "dist\windows"
}
elseif (-not [System.IO.Path]::IsPathRooted($DistDir)) {
    $DistDir = Join-Path $rootPath $DistDir
}

$targetTriple = if ($Target) { $Target } else { Get-RustHostTriple }
if ($targetTriple -notmatch "windows-msvc") {
    throw "Only windows-msvc targets are supported. Target was: $targetTriple"
}

$iconIco = Join-Path $rootPath "assets\icon.ico"
if (-not (Test-Path -LiteralPath $iconIco)) {
    throw "Missing $iconIco. Create it with: python scripts/make-icons.py"
}

# The Cargo target directory may be overridden (CARGO_TARGET_DIR, .cargo/config).
$metadata = & cargo metadata --format-version 1 --no-deps --manifest-path (Join-Path $rootPath "Cargo.toml") | ConvertFrom-Json
$cargoTarget = $metadata.target_directory
$workDir = Join-Path $cargoTarget "windows-package"
$resourceRc = Join-Path $workDir "app.rc"
$resourceRes = Join-Path $workDir "app.res"

New-Item -ItemType Directory -Force -Path $workDir | Out-Null
Copy-Item -LiteralPath $iconIco -Destination (Join-Path $workDir "app.ico") -Force
Set-Content -LiteralPath $resourceRc -Encoding ASCII -Value '1 ICON "app.ico"'
$rcExe = Find-RcExe -Arch (Get-RcArch $targetTriple)
Invoke-Checked -FilePath $rcExe -Arguments @("/nologo", "/fo$resourceRes", $resourceRc) -WorkingDirectory $workDir

if (-not $SkipBuild) {
    $cargoArgs = @("rustc", "--release", "-p", "tally-ui", "--bin", "tally-ui")
    if ($Target) {
        $cargoArgs += @("--target", $Target)
    }
    $cargoArgs += @("--", "-C", "link-arg=$resourceRes")
    Invoke-Checked -FilePath "cargo" -Arguments $cargoArgs -WorkingDirectory $rootPath
}

$releaseDir = if ($Target) { Join-Path $cargoTarget "$Target\release" } else { Join-Path $cargoTarget "release" }
$builtExe = Join-Path $releaseDir "tally-ui.exe"
if (-not (Test-Path -LiteralPath $builtExe)) {
    throw "Built executable not found: $builtExe"
}

# Keep the name tally-ui.exe: `tally quick-add` looks for the window next to itself.
New-Item -ItemType Directory -Force -Path $DistDir | Out-Null
$outExe = Join-Path $DistDir "tally-ui.exe"
Copy-Item -LiteralPath $builtExe -Destination $outExe -Force

Write-Host "Done: $outExe"
