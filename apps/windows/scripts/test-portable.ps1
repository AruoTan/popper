$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$temporary = Join-Path ([System.IO.Path]::GetTempPath()) ('popper-portable-tests-' + [Guid]::NewGuid().ToString('N'))
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$version = (Get-Content (Join-Path $projectRoot 'package.json') -Raw | ConvertFrom-Json).version
$import = Join-Path $projectRoot 'apps/windows/portable/Import-InstalledSettings.ps1'
function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
try {
    $source = Join-Path $temporary 'source'
    $data = Join-Path $temporary 'portable data'
    $null = New-Item -ItemType Directory -Path $source -Force
    [System.IO.File]::WriteAllText((Join-Path $source 'settings.json'), '{"version":1}')
    [System.IO.File]::WriteAllText((Join-Path $source 'api-keys.enc.json'), 'test ciphertext')
    [System.IO.File]::WriteAllText((Join-Path $source '.api-keys.key'), 'test local key')
    & $import -DataDirectory $data -SourceDirectory $source
    foreach ($name in @('settings.json', 'api-keys.enc.json', '.api-keys.key')) {
        Assert-True ((Get-FileHash -LiteralPath (Join-Path $source $name)).Hash -eq (Get-FileHash -LiteralPath (Join-Path $data "config/$name")).Hash) "Import changed $name"
    }
    $refused = $false
    try { & $import -DataDirectory $data -SourceDirectory $source } catch { $refused = $true }
    Assert-True $refused 'Import overwrote an existing profile'
    Remove-Item -LiteralPath (Join-Path $source '.api-keys.key') -Force
    $refused = $false
    $brokenData = Join-Path $temporary 'broken data'
    try { & $import -DataDirectory $brokenData -SourceDirectory $source } catch { $refused = $true }
    Assert-True ($refused -and -not (Test-Path -LiteralPath $brokenData)) 'Import accepted ciphertext without its key'

    $release = Join-Path $temporary 'release'
    $null = New-Item -ItemType Directory -Path (Join-Path $release 'data/config') -Force
    [System.IO.File]::WriteAllText((Join-Path $release 'popper.exe'), 'test executable')
    [System.IO.File]::WriteAllText((Join-Path $release 'data/config/.api-keys.key'), 'must not be distributed')
    [System.IO.File]::WriteAllText((Join-Path $release 'unrelated.dll'), 'must not be distributed')
    & (Join-Path $PSScriptRoot 'package-portable.ps1') -ReleaseDirectory $release
    $extracted = Join-Path $temporary 'extracted'
    Expand-Archive -LiteralPath (Join-Path $release "bundle/portable/Popper_${version}_x64-portable.zip") -DestinationPath $extracted
    $files = @(Get-ChildItem -LiteralPath $extracted -Recurse -Force)
    $expected = @('popper.exe', 'portable.flag', 'README.txt', 'Import-InstalledSettings.ps1', 'THIRD_PARTY_NOTICES.md', 'LICENSE.selection-hook')
    Assert-True ($files.Count -eq $expected.Count) 'Portable ZIP contains unexpected files'
    foreach ($file in $files) { Assert-True ($file.Name -in $expected -and -not $file.PSIsContainer) "Unexpected archive entry: $($file.Name)" }
    Assert-True ((Get-FileHash -LiteralPath (Join-Path $release 'popper.exe')).Hash -eq (Get-FileHash -LiteralPath (Join-Path $extracted 'popper.exe')).Hash) 'ZIP changed the executable'
    Write-Host 'Portable import and ZIP packaging tests passed.'
}
finally {
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Recurse -Force }
}
