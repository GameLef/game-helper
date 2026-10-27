#Requires AutoHotkey v2.0
#SingleInstance Off
; ============================================================
; runner.ahk - 一次性 CLI 执行器（调试 / 管理器按钮保底）
; 用法: AutoHotkey64.exe runner.ahk <路线.json> [--start N] [--only N] [--stop KEY]
;   --start N  从第 N 个点位开始（1-based）
;   --only N   仅执行第 N 个点位
;   --stop KEY 急停热键（默认 F12）
; 退出码: 0 成功 / 1 失败
; ============================================================
#Include lib\JSON.ahk
#Include lib\executor.ahk

baseDir := A_ScriptDir "\.."

; ---------- 解析参数 ----------
routePath := "", startAt := 1, only := 0, stopKey := "F12"
i := 1
while (i <= A_Args.Length) {
    a := A_Args[i]
    switch a {
        case "--start":  i++, startAt := Integer(A_Args[i])
        case "--only":   i++, only := Integer(A_Args[i])
        case "--stop":   i++, stopKey := A_Args[i]
        default:
            if (a != "" && SubStr(a, 1, 2) != "--")
                routePath := a
    }
    i++
}

if (routePath = "") {
    FileAppend("用法: runner.ahk <路线.json> [--start N] [--only N] [--stop KEY]`n", "*", "UTF-8")
    ExitApp(1)
}

EXE_Setup(baseDir)
Hotkey(stopKey, EXE_StopAllHandler, "On")

EXE_Log("RUNNER route=" routePath " startAt=" startAt " only=" only)
route := JSON.parse(FileRead(routePath, "UTF-8"))

EXE_HotkeyTick := A_TickCount
ok := EXE_RunRoute(route, startAt, only)
EXE_Log("RUNNER exit code=" (ok ? 0 : 1))
ExitApp(ok ? 0 : 1)
