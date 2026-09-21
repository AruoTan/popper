$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'This build entry point is intended for a GitHub-hosted Windows runner.'
}

Set-Location (Join-Path $PSScriptRoot '../..')

# Discover the runner's SDK/MSVC installation; do not hard-code SDK versions or
# copy any environment settings from a developer's Windows machine.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if ($LASTEXITCODE -ne 0 -or -not $installation) {
    throw 'No Visual Studio C++ installation found'
}
Import-Module (Join-Path $installation 'Common7/Tools/Microsoft.VisualStudio.DevShell.dll')
Enter-VsDevShell -VsInstallPath $installation -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64'

# Prefer the explicitly installed NASM, not a bundled Strawberry Perl copy.
$env:PATH = (Join-Path $env:ProgramFiles 'NASM') + ';' + $env:PATH
Get-Command cl.exe, link.exe, rc.exe, cmake.exe, nasm.exe | Select-Object Name, Source
rustc --version
if ($LASTEXITCODE -ne 0) { throw 'Rust toolchain unavailable' }
nasm -v
if ($LASTEXITCODE -ne 0) { throw 'NASM unavailable' }

pnpm verify:windows
if ($LASTEXITCODE -ne 0) { throw 'Windows verification failed; packaging skipped' }

# Preserve the repository's packaging/PE/version/SHA-256 verification entry point.
pnpm package:windows
if ($LASTEXITCODE -ne 0) { throw 'Windows packaging or artifact verification failed' }

$version = (Get-Content package.json -Raw | ConvertFrom-Json).version
$bundleDirectory = 'src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis'
foreach ($filename in @("TextLens_${version}_x64-setup.exe", 'SHA256SUMS.txt')) {
    $artifact = Join-Path $bundleDirectory $filename
    if (-not (Test-Path -LiteralPath $artifact -PathType Leaf) -or (Get-Item -LiteralPath $artifact).Length -eq 0) {
        throw "Missing or empty build artifact: $artifact"
    }
}

@"
## TextLens $version — Windows x64

- Source commit: $env:GITHUB_SHA
- Windows verification and installer checks passed.
- Download the installer and SHA256SUMS.txt from this run's Artifacts section.
- Unsigned test installer; SmartScreen warnings are possible.
- Native selection behavior still requires manual desktop regression testing.
"@ | Out-File -FilePath $env:GITHUB_STEP_SUMMARY -Encoding utf8 -Append
