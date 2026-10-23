param(
    [Parameter(Mandatory = $true)]
    [string]$Executable,
    [switch]$Portable,
    [ValidateRange(1, 300)]
    [int]$StartupTimeoutSeconds = 20
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
$deadline = [DateTime]::UtcNow.AddSeconds($StartupTimeoutSeconds)
$ready = $false
$nativeReady = $false
$checkpoint = 'logger not observed'
# Only these fixed labels may be printed. Raw logs can contain local paths or
# error details and remain on the runner rather than being uploaded to CI.
$checkpoints = @(
    'building application', 'initializing UI thread', 'loading settings',
    'initializing HTTP clients', 'initializing selection monitor',
    'initializing screen capture', 'creating settings window', 'creating toolbar',
    'creating tray', 'initializing quick ask shortcut',
    'initializing screen capture shortcuts', 'starting selection listener',
    'showing startup notice', 'setup complete', 'ready'
)
try {
    while ([DateTime]::UtcNow -lt $deadline) {
        $process.Refresh()
        if ($process.HasExited) {
            throw "Popper exited during startup (exit $($process.ExitCode))"
        }
        if (Test-Path -LiteralPath $logPath) {
            $log = Get-Content -LiteralPath $logPath -Raw -Encoding utf8
            $entries = @([regex]::Matches([string]$log, "(?m)^(\d+) process=$($process.Id) (.*)\r?$") |
                Where-Object { [long]$_.Groups[1].Value -ge $startedAtMs })
            foreach ($entry in $entries) {
                $message = $entry.Groups[2].Value.Trim()
                if ($message -in $checkpoints) { $checkpoint = $message }
            }
            if (@($entries | Where-Object { $_.Groups[2].Value.StartsWith('failure:') }).Count -gt 0) {
                throw 'Popper reported a native startup failure; inspect startup.log on the runner'
            }
            $nativeReady = @($entries | Where-Object { $_.Groups[2].Value.Trim() -eq 'ready' }).Count -gt 0
            if ($nativeReady) {
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
        $waitingFor = if ($nativeReady) { 'portable WebView profile creation' } else { 'native ready event' }
        throw "Popper did not complete startup within $StartupTimeoutSeconds seconds (waiting for $waitingFor; last checkpoint: $checkpoint)"
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
catch {
    # Emit the fixed checkpoint even for early exits, without copying or
    # printing the raw diagnostic log or any user configuration.
    Write-Host "Startup state: last checkpoint=$checkpoint; native ready=$nativeReady; portable=$Portable"
    throw
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
