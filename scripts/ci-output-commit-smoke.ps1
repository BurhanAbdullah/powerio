$ErrorActionPreference = "Stop"

# cargo reaches the MSVC toolset through the cc crate, but a bare cl.exe is
# not on PATH until the developer shell is entered.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vsPath = & $vswhere -latest -products * `
  -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
  -property installationPath
if (-not $vsPath) { throw 'no Visual Studio C++ toolset on this runner' }
Import-Module "$vsPath\Common7\Tools\Microsoft.VisualStudio.DevShell.dll"
Enter-VsDevShell -VsInstallPath $vsPath -SkipAutomaticLocation `
  -DevCmdArguments '-arch=x64 -host_arch=x64'
$importLib = Get-ChildItem target/release -Filter '*powerio_capi*.dll.lib' |
  Select-Object -First 1 -ExpandProperty FullName
if (-not $importLib) { throw 'powerio-capi import library is missing' }
cl.exe /nologo /std:c11 /DPIO_DIST /Ipowerio-capi/include `
  powerio-capi/examples/smoke.c `
  $importLib `
  /Fe:pio_smoke.exe
Copy-Item target/release/powerio_capi.dll .
./pio_smoke.exe tests/data/case9.m
