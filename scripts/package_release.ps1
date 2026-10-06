# Packages a Dusk release: Dusk-<version>-x64.zip plus its SHA-256 file in
# -OutDir. Expects the release executables and the two integrations to be
# built already (see .github/workflows/release.yml, which runs this).
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts\package_release.ps1 -Version 1.0.0 [-BinDir target\release] [-OutDir dist]
param(
    [Parameter(Mandatory)][string]$Version,
    [string]$BinDir = 'target\release',
    [string]$OutDir = 'dist'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

python scripts/set_version.py --check $Version
if ($LASTEXITCODE -ne 0) { throw "The sources are not at version $Version." }

$name = "Dusk-$Version-x64"
$stage = Join-Path $OutDir $name
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null

foreach ($exe in 'duskd.exe', 'dusk.exe') {
    $path = Join-Path $BinDir $exe
    if (-not (Test-Path $path)) { throw "Missing $path; build with: cargo build --release --bins" }
    Copy-Item $path $stage
}
Copy-Item README.md, LICENSE $stage

# The integrations, prebuilt; their install scripts use these folders.
$integrations = Join-Path $stage 'integrations'
$plugins = @{
    'PowerToysRun'   = 'integrations\PowerToysRun\bin\x64\Release\net9.0-windows'
    'CommandPalette' = 'integrations\CommandPalette\bin\x64\Release\net9.0-windows10.0.26100.0'
}
foreach ($plugin in $plugins.Keys) {
    $built = $plugins[$plugin]
    if (-not (Test-Path $built)) { throw "Missing $built; build the integrations first." }
    $destination = Join-Path $integrations $plugin
    New-Item -ItemType Directory -Force $destination | Out-Null
    Copy-Item (Join-Path $built '*') $destination -Recurse
}
Copy-Item integrations\install-powertoys-run.ps1, integrations\install-command-palette.ps1 $integrations

$zip = Join-Path $OutDir "$name.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
$hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLowerInvariant()
Set-Content -Path "$zip.sha256" -Value "$hash  $name.zip" -Encoding ascii
Write-Host "Packaged $zip ($([math]::Round((Get-Item $zip).Length / 1MB, 1)) MB), SHA-256 $hash"
