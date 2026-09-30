# Builds the Windows app in release mode and puts compositor.exe in a folder with the two DXC
# DLLs it loads at run time (dxcompiler.dll and dxil.dll). Copy that folder anywhere to run it.
#
#   pwsh port/tools/package-app.ps1 [-Out port/target/dist/compositor]

param([string]$Out = "port/target/dist/compositor")

$ErrorActionPreference = "Stop"
cargo build --release --manifest-path port/Cargo.toml -p app
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$release = "port/target/release"
if (-not (Test-Path (Join-Path $release "dxcompiler.dll")) -or -not (Test-Path (Join-Path $release "dxil.dll"))) {
    & (Join-Path $PSScriptRoot "fetch-dxc.ps1") -Into $release
}

New-Item -ItemType Directory -Force $Out | Out-Null
foreach ($file in "compositor.exe", "dxcompiler.dll", "dxil.dll") {
    Copy-Item -Force (Join-Path $release $file) $Out
}
Write-Host "Compositor -> $Out"
