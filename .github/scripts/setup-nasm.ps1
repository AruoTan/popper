$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$version = '3.02'
$expectedHash = '161d0bfaff53c2f9e9f3e69fd0672323ebabafd1268976a5cec11be92a19aee7'
$archive = Join-Path $env:RUNNER_TEMP "nasm-$version-win64.zip"
$destination = Join-Path $env:RUNNER_TEMP "popper-nasm-$version"
$nasmDirectory = Join-Path $destination "nasm-$version"
$nasmExecutable = Join-Path $nasmDirectory 'nasm.exe'

Invoke-WebRequest -Uri "https://www.nasm.us/pub/nasm/releasebuilds/$version/win64/nasm-$version-win64.zip" `
    -OutFile $archive -TimeoutSec 120 -MaximumRetryCount 3 -RetryIntervalSec 3
$actualHash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash
if ($actualHash -ne $expectedHash) {
    throw "NASM archive SHA-256 mismatch: expected $expectedHash, got $actualHash"
}

Expand-Archive -LiteralPath $archive -DestinationPath $destination -Force
if (-not (Test-Path -LiteralPath $nasmExecutable -PathType Leaf)) {
    throw "NASM executable missing from verified archive: $nasmExecutable"
}
$versionOutput = & $nasmExecutable -v
if ($LASTEXITCODE -ne 0) { throw "NASM verification failed (exit $LASTEXITCODE)" }
if ($versionOutput -notmatch ('^NASM version ' + [regex]::Escape($version) + '(?:\s|$)')) {
    throw "Unexpected NASM version: $versionOutput"
}
Write-Host $versionOutput

$env:POPPER_NASM_DIRECTORY = $nasmDirectory
$env:PATH = $nasmDirectory + [IO.Path]::PathSeparator + $env:PATH
$nasmDirectory | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
"POPPER_NASM_DIRECTORY=$nasmDirectory" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
