param([Parameter(Mandatory = $true)][string]$Archive)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$archivePath = (Resolve-Path -LiteralPath $Archive).Path
$temporary = Join-Path ([System.IO.Path]::GetTempPath()) ('Popper portable 测试 ' + [Guid]::NewGuid().ToString('N'))
$first = Join-Path $temporary 'apps/popper/first'
$second = Join-Path $temporary 'apps/popper/second'
$persist = Join-Path $temporary 'persist/popper/data'
$expected = @('Import-InstalledSettings.ps1', 'LICENSE.selection-hook', 'README.txt', 'THIRD_PARTY_NOTICES.md', 'popper.exe', 'portable.flag')
try {
    $null = New-Item -ItemType Directory -Path $persist -Force
    foreach ($directory in @($first, $second)) {
        Expand-Archive -LiteralPath $archivePath -DestinationPath $directory
        $actual = @(Get-ChildItem -LiteralPath $directory -Force | Sort-Object Name)
        if ($actual.Count -ne $expected.Count -or @($actual | Where-Object { $_.PSIsContainer -or $_.Name -notin $expected }).Count -gt 0) {
            throw 'Portable ZIP must contain only the six expected distribution files, without user data'
        }
        $null = New-Item -ItemType Junction -Path (Join-Path $directory 'data') -Target $persist
    }
    & (Join-Path $projectRoot '.github/scripts/smoke-test-windows.ps1') -Executable (Join-Path $first 'popper.exe') -Portable
    $settings = Join-Path $persist 'config/settings.json'
    $settingsHash = (Get-FileHash -LiteralPath $settings).Hash
    [System.IO.File]::WriteAllText((Join-Path $persist 'upgrade-check.txt'), 'keep across versions')
    & (Join-Path $projectRoot '.github/scripts/smoke-test-windows.ps1') -Executable (Join-Path $second 'popper.exe') -Portable
    if ((Get-FileHash -LiteralPath $settings).Hash -ne $settingsHash -or
        [System.IO.File]::ReadAllText((Join-Path $persist 'upgrade-check.txt')) -ne 'keep across versions') {
        throw 'Portable profile was changed or lost when switching Scoop version directories'
    }
    Write-Host 'Portable ZIP startup and Scoop-style junction upgrade passed.'
}
finally {
    # Remove junctions before deleting the fixture, never recursively follow a
    # Scoop-style data link while cleaning up the simulated app directories.
    foreach ($directory in @($first, $second)) {
        $link = Join-Path $directory 'data'
        if (Test-Path -LiteralPath $link) { [System.IO.Directory]::Delete($link) }
    }
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Recurse -Force -ErrorAction SilentlyContinue }
}
