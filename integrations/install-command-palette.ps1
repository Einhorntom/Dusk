# Builds the Dusk Command Palette extension and registers it for the
# current user from the build folder. Requires Developer Mode (Settings >
# System > Advanced > For developers). Command Palette picks the extension up
# by itself; if not, run "Reload" in Command Palette.
#
# Usage (works without changing the execution policy):
#   powershell -ExecutionPolicy Bypass -File integrations\install-command-palette.ps1
$ErrorActionPreference = 'Stop'
$project = Join-Path $PSScriptRoot 'CommandPalette\Dusk.CommandPalette.csproj'
$output = Join-Path $PSScriptRoot 'CommandPalette\bin\x64\Release\net9.0-windows10.0.26100.0'

$developerMode = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock' -ErrorAction SilentlyContinue).AllowDevelopmentWithoutDevLicense
if ($developerMode -ne 1) { throw 'Turn on Developer Mode first (Settings > System > Advanced > For developers).' }

# A running extension locks its files.
Get-Process -Name 'Dusk.CommandPalette', 'Dispcontrol.CommandPalette' -ErrorAction SilentlyContinue | Stop-Process -Force
# Before the rename to Dusk the extension was the "Dispcontrol.CommandPalette" package.
Get-AppxPackage -Name 'Dispcontrol.CommandPalette' | Remove-AppxPackage

dotnet build $project -c Release -p:Platform=x64
if ($LASTEXITCODE -ne 0) { throw 'build failed' }

Add-AppxPackage -Register (Join-Path $output 'AppxManifest.xml') -ForceApplicationShutdown
Get-AppxPackage -Name 'Dusk.CommandPalette' | Select-Object Name, Version, InstallLocation
Write-Host 'Registered. Open Command Palette and type "Dusk", a preset name, or open Dusk and type brightness 40.'
