# Builds the dispcontrol PowerToys Run plugin and copies it into the current
# user's PowerToys Run plugin folder. Restart PowerToys afterwards so it loads
# the plugin; then type "dc" in PowerToys Run (Alt+Space).
#
# Usage: powershell -File integrations\install-powertoys-run.ps1 [-Platform x64|ARM64]
param([ValidateSet('x64', 'ARM64')][string]$Platform = 'x64')

$ErrorActionPreference = 'Stop'
$project = Join-Path $PSScriptRoot 'PowerToysRun\Dispcontrol.PowerToysRun.csproj'
$output = Join-Path $PSScriptRoot "PowerToysRun\bin\$Platform\Release\net9.0-windows"
$target = Join-Path $env:LOCALAPPDATA 'Microsoft\PowerToys\PowerToys Run\Plugins\Dispcontrol'

dotnet build $project -c Release -p:Platform=$Platform
if ($LASTEXITCODE -ne 0) { throw 'build failed' }

if (Get-Process -Name 'PowerToys.PowerLauncher' -ErrorAction SilentlyContinue) {
    Write-Warning 'PowerToys Run is running; if copying fails, quit PowerToys first.'
}
New-Item -ItemType Directory -Force $target | Out-Null
Copy-Item -Path (Join-Path $output '*') -Destination $target -Recurse -Force
Write-Host "Installed to $target. Restart PowerToys, then type 'dc' in PowerToys Run."
