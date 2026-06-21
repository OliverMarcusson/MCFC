[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$DryRun,
    [string]$IsccPath
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $repoRoot 'mcfd\Cargo.toml'
$installerPath = Join-Path $repoRoot 'installer\mcfd.iss'
$releaseDir = Join-Path $repoRoot 'target\release'
$distDir = Join-Path $repoRoot 'dist'
$stagingDir = Join-Path $repoRoot 'target\package\mcfd'
$exePath = Join-Path $releaseDir 'mcfd.exe'

function Get-McfdVersion {
    $match = Select-String -Path $manifestPath -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if (-not $match) { throw "Could not read the mcfd version from $manifestPath" }
    return $match.Matches[0].Groups[1].Value
}

function Find-Iscc {
    if ($IsccPath) {
        if (-not (Test-Path -LiteralPath $IsccPath)) { throw "Inno Setup compiler not found at $IsccPath" }
        return (Resolve-Path -LiteralPath $IsccPath).Path
    }
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    $registryPaths = @(
        'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'
    )
    foreach ($entry in Get-ItemProperty -Path $registryPaths -ErrorAction SilentlyContinue) {
        if ($entry.DisplayName -notmatch '^Inno Setup') { continue }
        $candidate = Join-Path $entry.InstallLocation 'ISCC.exe'
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    foreach ($default in @(
        'C:\Program Files (x86)\Inno Setup 6\ISCC.exe',
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe')
    )) {
        if (Test-Path -LiteralPath $default) { return $default }
    }
    throw 'Inno Setup 6 was not found. Install it or pass -IsccPath <path-to-ISCC.exe>.'
}

function Invoke-OptionalSigning([string]$Path) {
    if (-not $env:MCFD_SIGN_COMMAND) { return }
    $command = $env:MCFD_SIGN_COMMAND.Replace('{file}', ('"' + $Path + '"'))
    Write-Host "Signing $Path"
    Invoke-Expression $command
    if ($LASTEXITCODE -ne 0) { throw "Signing failed for $Path" }
}

$version = Get-McfdVersion
$installerOutput = Join-Path $distDir "mcfd-$version-x64-setup.exe"
Write-Host "mcfd version: $version"
Write-Host "installer: $installerOutput"

if ($DryRun) {
    Write-Host 'Dry run: build, signing, installer compilation, and writes skipped.'
    exit 0
}

if (-not $SkipBuild) {
    & cargo build -p mcfd --release --manifest-path (Join-Path $repoRoot 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'cargo build -p mcfd --release failed.' }
}
if (-not (Test-Path -LiteralPath $exePath)) { throw "mcfd release binary not found at $exePath" }

$iscc = Find-Iscc
New-Item -ItemType Directory -Force -Path $distDir | Out-Null
New-Item -ItemType Directory -Force -Path $stagingDir | Out-Null
Invoke-OptionalSigning $exePath
Copy-Item -Force -LiteralPath $exePath -Destination (Join-Path $stagingDir 'mcfd.exe')
Copy-Item -Force -LiteralPath (Join-Path $repoRoot 'installer\README.txt') -Destination (Join-Path $stagingDir 'README.txt')

& $iscc "/DMyAppVersion=$version" "/DMyAppSource=$stagingDir" $installerPath
if ($LASTEXITCODE -ne 0) { throw 'Inno Setup compilation failed.' }
if (-not (Test-Path -LiteralPath $installerOutput)) { throw "Expected installer was not created: $installerOutput" }

Invoke-OptionalSigning $installerOutput
$checksum = (Get-FileHash -Algorithm SHA256 -LiteralPath $installerOutput).Hash.ToLowerInvariant()
Set-Content -NoNewline -Encoding ascii -Path "$installerOutput.sha256" -Value "$checksum *$(Split-Path -Leaf $installerOutput)"
Write-Host "Created $installerOutput"
Write-Host "Created $installerOutput.sha256"
