# Registers the Dusk Command Palette extension for the current user: the
# prebuilt extension next to this script (release zip; keep the folder where
# it is, Windows runs it from there), or a fresh build of the source
# (repository). Requires Developer Mode (Settings > System > Advanced > For
# developers). Command Palette picks the extension up by itself; if not, run
# "Reload" in Command Palette.
#
# Usage (works without changing the execution policy):
#   powershell -ExecutionPolicy Bypass -File integrations\install-command-palette.ps1
$ErrorActionPreference = 'Stop'
$project = Join-Path $PSScriptRoot 'CommandPalette\Dusk.CommandPalette.csproj'
$output = if (Test-Path $project) {
    Join-Path $PSScriptRoot 'CommandPalette\bin\x64\Release\net9.0-windows10.0.26100.0'
} else {
    Join-Path $PSScriptRoot 'CommandPalette'
}

$developerMode = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock' -ErrorAction SilentlyContinue).AllowDevelopmentWithoutDevLicense
if ($developerMode -ne 1) { throw 'Turn on Developer Mode first (Settings > System > Advanced > For developers).' }

# The extension is a .NET 9 program (not bundled, to keep the download small).
$runtimes = & dotnet --list-runtimes 2>$null
if (-not ($runtimes -match '^Microsoft\.NETCore\.App 9\.')) {
    throw 'The Command Palette extension needs the .NET 9 Runtime (x64): https://dotnet.microsoft.com/download/dotnet/9.0 . Install it, then run this script again.'
}

# A running extension locks its files.
Get-Process -Name 'Dusk.CommandPalette', 'Dispcontrol.CommandPalette' -ErrorAction SilentlyContinue | Stop-Process -Force
# Before the rename to Dusk the extension was the "Dispcontrol.CommandPalette" package.
Get-AppxPackage -Name 'Dispcontrol.CommandPalette' | Remove-AppxPackage

if (Test-Path $project) {
    # Repository: build the extension first.
    dotnet build $project -c Release -p:Platform=x64
    if ($LASTEXITCODE -ne 0) { throw 'build failed' }
} elseif (-not (Test-Path (Join-Path $output 'AppxManifest.xml'))) {
    throw "No extension found in $output."
}

Add-AppxPackage -Register (Join-Path $output 'AppxManifest.xml') -ForceApplicationShutdown
Get-AppxPackage -Name 'Dusk.CommandPalette' | Select-Object Name, Version, InstallLocation
Write-Host 'Registered. Open Command Palette and type "Dusk", a preset name, or open Dusk and type brightness 40.'
