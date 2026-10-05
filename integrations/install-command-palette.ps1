# Builds the dispcontrol Command Palette extension and registers it for the
# current user from the build folder. Requires Developer Mode (Settings >
# System > Advanced > For developers). Command Palette picks the extension up
# by itself; if not, run "Reload" in Command Palette.
#
# Usage: powershell -File integrations\install-command-palette.ps1
$ErrorActionPreference = 'Stop'
$project = Join-Path $PSScriptRoot 'CommandPalette\Dispcontrol.CommandPalette.csproj'
$output = Join-Path $PSScriptRoot 'CommandPalette\bin\x64\Release\net9.0-windows10.0.26100.0'

$developerMode = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock' -ErrorAction SilentlyContinue).AllowDevelopmentWithoutDevLicense
if ($developerMode -ne 1) { throw 'Turn on Developer Mode first (Settings > System > Advanced > For developers).' }

# A running extension locks its files.
Get-Process -Name 'Dispcontrol.CommandPalette' -ErrorAction SilentlyContinue | Stop-Process -Force

dotnet build $project -c Release -p:Platform=x64
if ($LASTEXITCODE -ne 0) { throw 'build failed' }

Add-AppxPackage -Register (Join-Path $output 'AppxManifest.xml') -ForceApplicationShutdown
Get-AppxPackage -Name 'Dispcontrol.CommandPalette' | Select-Object Name, Version, InstallLocation
Write-Host 'Registered. Open Command Palette and type "dispcontrol", a preset name, or open dispcontrol and type brightness 40.'
