# Builds the portable Windows release: dist\Relix-<version>-windows-x64-portable.zip
# Needs vendor\mpv set up (see README). Run from anywhere: powershell -File packaging\portable.ps1
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

cargo build --release -p relix-app -p relix-server
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$version = (Select-String -Path "crates\relix-app\Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$name = "Relix-$version-windows-x64-portable"
$out = Join-Path $root "dist\$name"
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force $out | Out-Null

Copy-Item "target\release\relix-app.exe", "target\release\relix-server.exe", "target\release\libmpv-2.dll" $out
Copy-Item "packaging\portable-readme.txt" "$out\README.txt"
# The marker that keeps data next to the app.
New-Item -ItemType File "$out\portable" | Out-Null

$zip = Join-Path $root "dist\$name.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path $out -DestinationPath $zip
Get-Item $zip | Select-Object Name, @{ n = "MB"; e = { [math]::Round($_.Length / 1MB, 1) } }
