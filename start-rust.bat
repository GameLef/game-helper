@echo off
setlocal
set "GH_EXE=%~dp0target\release\game-helper.exe"
if not exist "%GH_EXE%" set "GH_EXE=%~dp0target\debug\game-helper.exe"
if not exist "%GH_EXE%" (
    echo Native executable not found. Run scripts\cargo.cmd build -p game-helper-desktop --locked
    exit /b 1
)
start "" "%GH_EXE%" --app-root "%~dp0." %*
