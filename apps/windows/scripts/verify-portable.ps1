param([Parameter(Mandatory = $true)][string]$Archive)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$archivePath = (Resolve-Path -LiteralPath $Archive).Path
$temporary = Join-Path ([System.IO.Path]::GetTempPath()) ('Popper portable 测试 ' + [Guid]::NewGuid().ToString('N'))
$first = Join-Path $temporary 'apps/popper/first'
$second = Join-Path $temporary 'apps/popper/second'
$persist = Join-Path $temporary 'persist/popper'
$expected = @('Import-InstalledSettings.ps1', 'LICENSE.capture', 'LICENSE', 'LICENSE.selection-hook', 'README.txt', 'THIRD_PARTY_NOTICES.md', 'popper.exe', 'portable.flag')
try {
    foreach ($name in @('config', 'logs')) {
        $null = New-Item -ItemType Directory -Path (Join-Path $persist $name) -Force
    }
    foreach ($directory in @($first, $second)) {
        Expand-Archive -LiteralPath $archivePath -DestinationPath $directory
        $actual = @(Get-ChildItem -LiteralPath $directory -Force | Sort-Object Name)
        if ($actual.Count -ne $expected.Count -or @($actual | Where-Object { $_.PSIsContainer -or $_.Name -notin $expected }).Count -gt 0) {
            throw "Portable ZIP must contain only the $($expected.Count) expected distribution files, without user data"
        }
        foreach ($name in @('config', 'logs')) {
            $null = New-Item -ItemType Junction -Path (Join-Path $directory $name) -Target (Join-Path $persist $name)
        }
    }
    & (Join-Path $projectRoot '.github/scripts/smoke-test-windows.ps1') -Executable (Join-Path $first 'popper.exe') -Portable -StartupTimeoutSeconds 60
    $settings = Join-Path $persist 'config/settings.json'
    $settingsHash = (Get-FileHash -LiteralPath $settings).Hash
    [System.IO.File]::WriteAllText((Join-Path $persist 'logs/upgrade-check.txt'), 'keep across versions')
    [System.IO.File]::WriteAllText((Join-Path $first 'webview/old-cache.txt'), 'do not persist')
    & (Join-Path $projectRoot '.github/scripts/smoke-test-windows.ps1') -Executable (Join-Path $second 'popper.exe') -Portable -StartupTimeoutSeconds 60
    if ((Get-FileHash -LiteralPath $settings).Hash -ne $settingsHash -or
        [System.IO.File]::ReadAllText((Join-Path $persist 'logs/upgrade-check.txt')) -ne 'keep across versions') {
        throw 'Portable profile was changed or lost when switching Scoop version directories'
    }
    if ((Test-Path -LiteralPath (Join-Path $second 'webview/old-cache.txt')) -or
        (Test-Path -LiteralPath (Join-Path $persist 'webview'))) {
        throw 'WebView data was unexpectedly persisted between Scoop versions'
    }
    Write-Host 'Portable ZIP startup and Scoop-style junction upgrade passed.'
}
finally {
    # Remove junctions before deleting the fixture, never recursively follow a
    # Scoop-style config/logs links while cleaning up the simulated directories.
    foreach ($directory in @($first, $second)) {
        foreach ($name in @('config', 'logs')) {
            $link = Join-Path $directory $name
            if (Test-Path -LiteralPath $link) { [System.IO.Directory]::Delete($link) }
        }
    }
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Recurse -Force -ErrorAction SilentlyContinue }
}
