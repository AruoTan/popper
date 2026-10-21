param(
    [string]$DataDirectory = $PSScriptRoot,
    [string]$SourceDirectory = (Join-Path $env:APPDATA 'com.local.popper')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (Get-Process -Name popper -ErrorAction SilentlyContinue) {
    throw 'Exit Popper (including the installed version) before importing settings.'
}
$source = (Resolve-Path -LiteralPath $SourceDirectory).Path
$settings = Join-Path $source 'settings.json'
if (-not (Test-Path -LiteralPath $settings -PathType Leaf)) {
    throw "Installed settings were not found: $settings"
}
$null = Get-Content -LiteralPath $settings -Raw -Encoding utf8 | ConvertFrom-Json
$encrypted = Test-Path -LiteralPath (Join-Path $source 'api-keys.enc.json') -PathType Leaf
$key = Test-Path -LiteralPath (Join-Path $source '.api-keys.key') -PathType Leaf
if ($encrypted -and -not $key) {
    throw 'The installed API key file is missing its encryption key. No settings were imported.'
}

$null = New-Item -ItemType Directory -Path $DataDirectory -Force
$dataRoot = (Resolve-Path -LiteralPath $DataDirectory).Path
$destination = Join-Path $dataRoot 'config'
if (Test-Path -LiteralPath $destination) {
    throw "Destination already exists: $destination. Import before the first portable launch, or back up and move the existing config directory first."
}
$staging = Join-Path $dataRoot ('.import-' + [Guid]::NewGuid().ToString('N'))
try {
    $null = New-Item -ItemType Directory -Path $staging
    foreach ($name in @('settings.json', 'api-keys.enc.json', '.api-keys.key')) {
        $file = Join-Path $source $name
        if (Test-Path -LiteralPath $file -PathType Leaf) {
            Copy-Item -LiteralPath $file -Destination (Join-Path $staging $name)
        }
    }
    # Publish the complete directory together; Directory.Move refuses to
    # overwrite a profile created by another launch during the import.
    [System.IO.Directory]::Move($staging, $destination)
}
finally {
    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
}
Write-Host "Imported settings and local secrets to $destination. The installed profile was kept."
