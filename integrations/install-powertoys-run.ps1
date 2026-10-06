# Installs the Dusk PowerToys Run plugin into the current user's PowerToys
# Run plugin folder: the prebuilt plugin next to this script (release zip),
# or a fresh build of the source (repository). Restart PowerToys afterwards
# so it loads the plugin; then type "dusk" in PowerToys Run (Alt+Space).
#
# Usage (works without changing the execution policy):
#   powershell -ExecutionPolicy Bypass -File integrations\install-powertoys-run.ps1 [-Platform x64|ARM64]
param([ValidateSet('x64', 'ARM64')][string]$Platform = 'x64')

$ErrorActionPreference = 'Stop'
$project = Join-Path $PSScriptRoot 'PowerToysRun\Dusk.PowerToysRun.csproj'
$target = Join-Path $env:LOCALAPPDATA 'Microsoft\PowerToys\PowerToys Run\Plugins\Dusk'

if (Test-Path $project) {
    # Repository: build the plugin first.
    $output = Join-Path $PSScriptRoot "PowerToysRun\bin\$Platform\Release\net9.0-windows"
    dotnet build $project -c Release -p:Platform=$Platform
    if ($LASTEXITCODE -ne 0) { throw 'build failed' }
} else {
    # Release zip: the plugin is already built (x64).
    $output = Join-Path $PSScriptRoot 'PowerToysRun'
    if (-not (Test-Path (Join-Path $output 'plugin.json'))) { throw "No plugin found in $output." }
}

if (Get-Process -Name 'PowerToys.PowerLauncher' -ErrorAction SilentlyContinue) {
    Write-Warning 'PowerToys Run is running; if copying fails, quit PowerToys first.'
}
# Before the rename to Dusk the plugin was installed as "Dispcontrol" (same plugin ID).
$legacy = Join-Path $env:LOCALAPPDATA 'Microsoft\PowerToys\PowerToys Run\Plugins\Dispcontrol'
if (Test-Path $legacy) { Remove-Item -Recurse -Force $legacy }
New-Item -ItemType Directory -Force $target | Out-Null
Copy-Item -Path (Join-Path $output '*') -Destination $target -Recurse -Force
Write-Host "Installed to $target. Restart PowerToys, then type 'dusk' in PowerToys Run."
