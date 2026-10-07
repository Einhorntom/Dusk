# Builds the MSIX package for the Microsoft Store: Dusk-<version>-x64.msix in
# -OutDir, plus the unpacked layout next to it for local testing. Expects the
# release executables to be built (cargo build --release --bins).
#
# The package is unsigned: the Store signs it after certification. To try it
# locally, register the layout (Developer Mode):
#   Add-AppxPackage -Register dist\msix\Dusk-<version>-x64\AppxManifest.xml
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts\package_msix.ps1 -Version 1.0.0 [-BinDir target\release] [-OutDir dist]
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

$identity = Get-Content packaging\msix\store.json -Raw | ConvertFrom-Json
if ($identity.identity_name -like 'PLACEHOLDER*') {
    Write-Warning 'packaging\msix\store.json still has placeholder identity values; the Store will reject this package.'
}

$name = "Dusk-$Version-x64"
$stage = Join-Path $OutDir "msix\$name"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null

foreach ($exe in 'duskd.exe', 'dusk.exe') {
    $path = Join-Path $BinDir $exe
    if (-not (Test-Path $path)) { throw "Missing $path; build with: cargo build --release --bins" }
    Copy-Item $path $stage
}
Copy-Item packaging\msix\Assets $stage -Recurse
Copy-Item LICENSE $stage

$manifest = Get-Content packaging\msix\AppxManifest.xml -Raw
$values = @{
    '{IDENTITY_NAME}'          = $identity.identity_name
    '{PUBLISHER}'              = $identity.publisher
    '{PUBLISHER_DISPLAY_NAME}' = $identity.publisher_display_name
    '{DISPLAY_NAME}'           = $identity.display_name
    '{VERSION}'                = $Version
}
foreach ($key in $values.Keys) {
    $manifest = $manifest.Replace($key, [System.Security.SecurityElement]::Escape($values[$key]))
}
[System.IO.File]::WriteAllText((Join-Path $stage 'AppxManifest.xml'), $manifest, [System.Text.UTF8Encoding]::new($false))

# makeappx: from the Windows SDK, or the NuGet SDK build tools.
$makeappx = @(
    Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\makeappx.exe' -ErrorAction SilentlyContinue
    Get-ChildItem "$env:USERPROFILE\.nuget\packages\microsoft.windows.sdk.buildtools\*\bin\*\x64\makeappx.exe" -ErrorAction SilentlyContinue
) | Sort-Object { [version]($_.Directory.Parent.Name -replace '[^\d.]', '') } -Descending | Select-Object -First 1
if (-not $makeappx) { throw 'makeappx.exe not found; install the Windows SDK.' }

$package = Join-Path $OutDir "$name.msix"
& $makeappx.FullName pack /d $stage /p $package /o /h SHA256
if ($LASTEXITCODE -ne 0) { throw 'makeappx failed (see above; it validates the manifest).' }
Write-Host "Packaged $package ($([math]::Round((Get-Item $package).Length / 1MB, 1)) MB) with $($makeappx.FullName)"
