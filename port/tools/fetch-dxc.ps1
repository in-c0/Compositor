# Fetches Microsoft's DirectX Shader Compiler (DXC) and puts dxcompiler.dll and dxil.dll next to
# the port's executables. wgpu compiles WGSL for DX12 through it; the older FXC compiler can't
# handle the engine's shaders. Pinned to one release, checked against its SHA-256.
#
#   pwsh port/tools/fetch-dxc.ps1 [-Into port/target/release] [-Into port/target/debug]

param([string[]]$Into = @("port/target/release", "port/target/debug"))

$ErrorActionPreference = "Stop"
$tag = "v1.9.2609"
$asset = "dxc_2026_09_29.zip"
$sha256 = "ad31b1fc8443175d204f77a611fdb3ef2ec42759bdc2f1167368de24a4a7e7f1"

$cache = Join-Path ([System.IO.Path]::GetTempPath()) "compositor-dxc-$tag"
$zip = Join-Path $cache $asset
if (-not (Test-Path $zip)) {
    New-Item -ItemType Directory -Force $cache | Out-Null
    Invoke-WebRequest "https://github.com/microsoft/DirectXShaderCompiler/releases/download/$tag/$asset" -OutFile $zip
}
$actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
if ($actual -ne $sha256) {
    Remove-Item $zip
    throw "$asset has SHA-256 $actual, expected $sha256"
}
$unpacked = Join-Path $cache "unpacked"
if (-not (Test-Path (Join-Path $unpacked "bin/x64/dxcompiler.dll"))) {
    Expand-Archive -Force $zip $unpacked
}
foreach ($dir in $Into) {
    New-Item -ItemType Directory -Force $dir | Out-Null
    foreach ($dll in "dxcompiler.dll", "dxil.dll") {
        Copy-Item -Force (Join-Path $unpacked "bin/x64/$dll") $dir
    }
    Write-Host "DXC $tag -> $dir"
}
