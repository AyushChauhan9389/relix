# Puts libmpv into vendor\mpv for Windows builds: the shinchiro dev build plus an MSVC import library.
$ErrorActionPreference = "Stop"
# shortcut: pinned shinchiro build; their old releases get pruned, so bump this when the download 404s.
$url = "https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20261021/mpv-dev-x86_64-v3-20261021-git-e471496c56.7z"

New-Item -ItemType Directory -Force vendor\mpv | Out-Null
Invoke-WebRequest $url -OutFile mpv-dev.7z
7z x -y -ovendor\mpv mpv-dev.7z | Out-Null
Remove-Item mpv-dev.7z

$vs = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -property installationPath
$msvc = Get-ChildItem "$vs\VC\Tools\MSVC" | Sort-Object Name | Select-Object -Last 1
$bin = "$($msvc.FullName)\bin\Hostx64\x64"
$exports = & "$bin\dumpbin.exe" -nologo -exports vendor\mpv\libmpv-2.dll |
    Select-String '\s(mpv_\w+)\s*$' | ForEach-Object { $_.Matches[0].Groups[1].Value }
if ($exports.Count -lt 40) { throw "found only $($exports.Count) mpv exports in libmpv-2.dll" }
@("LIBRARY libmpv-2", "EXPORTS") + $exports | Set-Content vendor\mpv\mpv.def
& "$bin\lib.exe" -nologo -def:vendor\mpv\mpv.def -name:libmpv-2.dll -out:vendor\mpv\mpv.lib -machine:x64
if ($LASTEXITCODE -ne 0) { throw "lib.exe failed" }
