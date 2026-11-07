@echo off
setlocal
rem Build tools and downloaded dependencies stay local to this project.
pushd "%~dp0.." || exit /b 1
set "CARGO_HOME=%CD%\.tools\cargo"
set "RUSTUP_HOME=%CD%\.tools\rustup"
set "CARGO_TARGET_DIR=%CD%\target"
set "TEMP=%CD%\.tools\tmp"
set "TMP=%TEMP%"
if not exist "%TEMP%" mkdir "%TEMP%"
if errorlevel 1 exit /b 1

if not defined VSINSTALLDIR (
    for /f "usebackq tokens=*" %%i in (`powershell -NoProfile -Command "& (Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe') -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath"`) do set "GH_VS_ROOT=%%i"
    if not defined GH_VS_ROOT (
        echo Visual Studio C++ tools were not found. 1>&2
        exit /b 1
    )
)
if defined GH_VS_ROOT call "%GH_VS_ROOT%\Common7\Tools\VsDevCmd.bat" -arch=x64 -host_arch=x64 >nul
if errorlevel 1 exit /b 1
if exist "%VSINSTALLDIR%Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe" set "PATH=%VSINSTALLDIR%Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin;%PATH%"
cargo %*
set "GH_CARGO_EXIT=%ERRORLEVEL%"
popd
exit /b %GH_CARGO_EXIT%
