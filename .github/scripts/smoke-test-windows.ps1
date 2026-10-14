param(
    [Parameter(Mandatory = $true)]
    [string]$Executable
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# Library tests do not exercise native application startup. Check the actual
# release executable and keep its diagnostics local to the Windows runner.
$executablePath = (Resolve-Path -LiteralPath $Executable).Path
$logPath = Join-Path $env:LOCALAPPDATA 'Popper/startup.log'
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
                $ready = $true
                break
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
