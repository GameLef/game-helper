@echo off
rem ============================================
rem  game_helper v2 脚本管理器启动脚本
rem  双击即可启动；已运行时会提示并跳过
rem  若目标游戏以管理员运行，请右键以管理员身份运行本脚本
rem ============================================

rem 切到脚本所在目录，保证相对路径可用
cd /d "%~dp0"

rem 检查是否已有 manager.py 在运行，避免重复拉起
powershell -NoProfile -Command "if (Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -match 'manager\.py' }) { exit 0 } else { exit 1 }" >nul 2>nul
if not errorlevel 1 (
    echo 脚本管理器已在运行中，本次不重复启动。
    timeout /t 3 >nul
    exit /b
)

rem 优先用 pythonw（无控制台黑窗口），找不到则回退 python
where pythonw >nul 2>nul
if %errorlevel%==0 (
    start "" pythonw "python\manager.py"
) else (
    start "" python "python\manager.py"
)

echo 脚本管理器已启动。
timeout /t 2 >nul
