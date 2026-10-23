$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# Exercise the real polling/cleanup script without launching desktop software.
$smoke = Join-Path $PSScriptRoot 'smoke-test-windows.ps1'
$temporary = Join-Path ([System.IO.Path]::GetTempPath()) ('popper-startup-tests-' + [Guid]::NewGuid().ToString('N'))
$previousLocalAppData = $env:LOCALAPPDATA
$fixture = [pscustomobject]@{ Process = $null; StopCount = 0; CaseName = ''; Root = '' }

function Start-Process {
    param([string]$FilePath, [switch]$PassThru)
    $fixture.Process = [pscustomobject]@{
        Id = 42424; HasExited = ($fixture.CaseName -eq 'early-exit'); ExitCode = 7; Disposed = $false
    }
    $fixture.Process | Add-Member ScriptMethod Refresh {}
    $fixture.Process | Add-Member ScriptMethod WaitForExit { param($Timeout) return $true }
    $fixture.Process | Add-Member ScriptMethod Dispose { $this.Disposed = $true }
    $portable = $fixture.CaseName -like 'portable-*'
    $log = if ($portable) { Join-Path $fixture.Root 'logs/startup.log' } else { Join-Path $env:LOCALAPPDATA 'Popper/startup.log' }
    $null = New-Item -ItemType Directory -Path (Split-Path $log) -Force
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $lines = @('1 process=42424 ready', "$now process=99999 ready", "$now process=42424 creating toolbar")
    if ($fixture.CaseName -in @('ready', 'portable-ready', 'portable-empty')) { $lines += "$now process=42424 ready" }
    if ($fixture.CaseName -eq 'native-failure') { $lines += "$now process=42424 failure: private diagnostic details" }
    if ($fixture.CaseName -ne 'missing-log') { $lines | Set-Content -LiteralPath $log -Encoding utf8 }
    if ($portable) {
        $null = New-Item -ItemType Directory -Path (Join-Path $fixture.Root 'webview') -Force
        $null = New-Item -ItemType Directory -Path (Join-Path $fixture.Root 'config') -Force
        '{}' | Set-Content -LiteralPath (Join-Path $fixture.Root 'config/settings.json')
        if ($fixture.CaseName -eq 'portable-ready') {
            'profile' | Set-Content -LiteralPath (Join-Path $fixture.Root 'webview/profile')
        }
    }
    return $fixture.Process
}
function Stop-Process {
    param([int]$Id, [switch]$Force, [string]$ErrorAction)
    if ($Id -ne 42424) { throw 'Attempted to stop an unrelated process' }
    $fixture.StopCount++
    $fixture.Process.HasExited = $true
}
function Start-Sleep { param([int]$Milliseconds) }

try {
    foreach ($caseName in @('ready', 'stale-ready', 'native-failure', 'missing-log', 'early-exit', 'portable-ready', 'portable-empty')) {
        $fixture.CaseName = $caseName
        $fixture.Root = Join-Path $temporary $fixture.CaseName
        $null = New-Item -ItemType Directory -Path $fixture.Root -Force
        $env:LOCALAPPDATA = Join-Path $fixture.Root 'local'
        $executable = Join-Path $fixture.Root 'popper.exe'
        'fixture' | Set-Content -LiteralPath $executable
        $portable = $fixture.CaseName -like 'portable-*'
        if ($portable) { '' | Set-Content -LiteralPath (Join-Path $fixture.Root 'portable.flag') }
        $fixture.StopCount = 0
        $failure = $null
        $output = [System.Collections.Generic.List[string]]::new()
        try {
            & $smoke -Executable $executable -Portable:$portable -StartupTimeoutSeconds 1 *>&1 |
                ForEach-Object { $output.Add([string]$_) }
        } catch { $failure = $_ }
        if ($fixture.CaseName -in @('ready', 'portable-ready')) {
            if ($failure) { throw $failure }
        } else {
            $expected = switch ($fixture.CaseName) {
                'stale-ready' { 'last checkpoint: creating toolbar' }
                'native-failure' { 'reported a native startup failure' }
                'missing-log' { 'last checkpoint: logger not observed' }
                'early-exit' { 'exited during startup (exit 7)' }
                'portable-empty' { 'waiting for portable WebView profile creation' }
            }
            if (-not $failure -or -not $failure.Exception.Message.Contains($expected)) {
                throw "Unexpected result for $($fixture.CaseName): $failure"
            }
            if (-not ($output -match '^Startup state:')) { throw 'Missing sanitized startup state' }
        }
        if (($output -join "`n").Contains('private diagnostic details')) { throw 'Raw diagnostic was exposed' }
        $expectedStops = if ($fixture.CaseName -eq 'early-exit') { 0 } else { 1 }
        if ($fixture.StopCount -ne $expectedStops -or -not $fixture.Process.Disposed) { throw 'Process cleanup failed' }
        Write-Host "PASS: $($fixture.CaseName)"
    }
} finally {
    $env:LOCALAPPDATA = $previousLocalAppData
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Recurse -Force }
}
