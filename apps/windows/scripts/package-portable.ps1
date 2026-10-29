param([string]$ReleaseDirectory)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$version = & node (Join-Path $projectRoot 'scripts/version.mjs')
if ($LASTEXITCODE -ne 0) { throw 'Cannot derive Popper version from Git tags' }
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid Popper version' }
if (-not $ReleaseDirectory) {
    $targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $projectRoot 'src-tauri/target' }
    if (-not [System.IO.Path]::IsPathRooted($targetRoot)) { $targetRoot = Join-Path $projectRoot $targetRoot }
    $ReleaseDirectory = Join-Path $targetRoot 'x86_64-pc-windows-msvc/release'
}
$releaseRoot = (Resolve-Path -LiteralPath $ReleaseDirectory).Path
$executable = Join-Path $releaseRoot 'popper.exe'
if (-not (Test-Path -LiteralPath $executable -PathType Leaf) -or (Get-Item -LiteralPath $executable).Length -eq 0) {
    throw "Missing release executable: $executable"
}
$output = Join-Path $releaseRoot 'bundle/portable'
$null = New-Item -ItemType Directory -Path $output -Force
$archive = Join-Path $output "Popper_${version}_x64-portable.zip"
$staging = Join-Path ([System.IO.Path]::GetTempPath()) ('popper-package-' + [Guid]::NewGuid().ToString('N'))
try {
    $null = New-Item -ItemType Directory -Path $staging
    # Only explicitly listed distribution files are packaged. Never include
    # release directory contents, smoke-test data, DLL build tools or secrets.
    Copy-Item -LiteralPath $executable -Destination (Join-Path $staging 'popper.exe')
    foreach ($name in @('THIRD_PARTY_NOTICES.md', 'LICENSE.selection-hook', 'LICENSE.capture', 'LICENSE')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot $name) -Destination (Join-Path $staging $name)
    }
    foreach ($name in @('README.txt', 'Import-InstalledSettings.ps1')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot "apps/windows/portable/$name") -Destination (Join-Path $staging $name)
    }
    [System.IO.File]::WriteAllText((Join-Path $staging 'portable.flag'), '')
    Compress-Archive -Path (Join-Path $staging '*') -DestinationPath $archive -CompressionLevel Optimal -Force
}
finally {
    if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
}
Write-Host "Portable ZIP: $archive"
