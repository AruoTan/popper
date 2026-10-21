param(
    [Parameter(Mandatory = $true)]
    [string]$Executable,
    [switch]$Portable
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# Library tests do not exercise native application startup. Check the actual
# release executable and keep its diagnostics local to the Windows runner.
$executablePath = (Resolve-Path -LiteralPath $Executable).Path
$installedLog = Join-Path $env:LOCALAPPDATA 'Popper/startup.log'
$installedLogHash = if (Test-Path -LiteralPath $installedLog) { (Get-FileHash -LiteralPath $installedLog).Hash } else { '' }
$dataRoot = Split-Path $executablePath
$logPath = if ($Portable) { Join-Path $dataRoot 'logs/startup.log' } else { $installedLog }
if ($Portable -and -not (Test-Path -LiteralPath (Join-Path (Split-Path $executablePath) 'portable.flag'))) {
    throw 'Portable smoke test requires portable.flag beside the executable'
}
$startedAtMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$process = Start-Process -FilePath $executablePath -PassThru
$deadline = [DateTime]::UtcNow.AddSeconds(20)
$ready = $false
try {
    while ([DateTime]::UtcNow -lt $deadline) {
        $process.Refresh()
        if ($process.HasExited) {
            throw "Popper exited during startup (exit $($process.ExitCode))"
        }
        if (Test-Path -LiteralPath $logPath) {
            $log = Get-Content -LiteralPath $logPath -Raw -Encoding utf8
            if ($log -match "(?m)^(\d+) process=$($process.Id) ready\r?$" -and [long]$Matches[1] -ge $startedAtMs) {
                # WebView creation is queued on Windows. A cold profile can
                # finish after the native app's ready event; allow the same
                # bounded startup window for its actual data to appear.
                $webviewRoot = Join-Path $dataRoot 'webview'
                if (-not $Portable -or ((Test-Path -LiteralPath $webviewRoot) -and
                    @(Get-ChildItem -LiteralPath $webviewRoot -Force).Count -gt 0)) {
                    $ready = $true
                    break
                }
            }
        }
        Start-Sleep -Milliseconds 200
    }
    if (-not $ready) {
        throw 'Popper did not complete startup within 20 seconds'
    }
    Start-Sleep -Milliseconds 500
    $process.Refresh()
    if ($process.HasExited) {
        throw "Popper exited immediately after startup (exit $($process.ExitCode))"
    }
    if ($Portable) {
        foreach ($path in @('config/settings.json', 'webview')) {
            if (-not (Test-Path -LiteralPath (Join-Path $dataRoot $path))) {
                throw "Portable startup did not create $path beside the executable"
            }
        }
        if (@(Get-ChildItem -LiteralPath (Join-Path $dataRoot 'webview') -Force).Count -eq 0) {
            throw 'Portable WebView did not use webview beside the executable'
        }
        if (Test-Path -LiteralPath (Join-Path $dataRoot 'data')) {
            throw 'Portable startup unexpectedly created the old data directory'
        }
        $currentInstalledLogHash = if (Test-Path -LiteralPath $installedLog) { (Get-FileHash -LiteralPath $installedLog).Hash } else { '' }
        if ($currentInstalledLogHash -ne $installedLogHash) {
            throw 'Portable startup wrote to the installed startup log'
        }
    }
    Write-Host "Popper release startup passed (PID $($process.Id))."
}
finally {
    # Only stop the process started by this check, never another user instance.
    $process.Refresh()
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        $null = $process.WaitForExit(5000)
    }
    $process.Dispose()
}
