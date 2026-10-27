#Requires AutoHotkey v2.0
; ============================================================
; executor.ahk - 共享执行库（runner.ahk / daemon.ahk 共用）
; 职责：窗口定位、归一化坐标回放、急停、日志
; 坐标系：0~1 归一化、相对目标窗口客户区
; ============================================================

global EXE_StopFlag := false        ; 急停标志
global EXE_Running := false         ; 是否正在执行路线（防重入）
global EXE_LogFile := ""            ; 当前日志文件
global EXE_HotkeyTick := 0          ; 热键触发时刻（用于延迟统计）

; ---------- 基础设置 ----------
EXE_Setup(baseDir) {
    global EXE_LogFile
    ; Per-Monitor DPI Aware，避免高 DPI 缩放下坐标错位
    try DllCall("Shcore\SetProcessDpiAwareness", "uint", 2)
    try DllCall("User32\SetProcessDPIAware")
    CoordMode("Mouse", "Screen")
    SendMode("Input")
    SetStoreCapsLockMode(false)

    logDir := baseDir "\logs"
    DirCreate(logDir)
    EXE_LogFile := logDir "\run_" FormatTime(A_Now, "yyyyMMdd_HHmmss") ".log"
}

; ---------- 日志 ----------
EXE_Log(msg) {
    if (EXE_LogFile = "")
        return
    FileAppend(FormatTime(A_Now, "yyyy-MM-dd HH:mm:ss") " | " msg "`n", EXE_LogFile, "UTF-8")
}

; ---------- 可中断等待：急停时提前返回 ----------
EXE_Sleep(ms) {
    end := A_TickCount + ms
    while (A_TickCount < end) {
        if (EXE_StopFlag)
            return
        Sleep(Min(50, end - A_TickCount))
    }
}

; ---------- 按目标标识查找窗口 ----------
; target: Map/Obj，含 process / windowClass / title（至少一个非空）
; 返回 hwnd；找不到返回 0
EXE_FindTargetWindow(target) {
    list := 0
    proc := EXE_GetStr(target, "process")
    cls := EXE_GetStr(target, "windowClass")
    title := EXE_GetStr(target, "title")
    if (proc != "") {
        list := WinGetList("ahk_exe " proc)
    } else if (cls != "") {
        list := WinGetList("ahk_class " cls)
    } else if (title != "") {
        SetTitleMatchMode(2)
        list := WinGetList(title)
    } else {
        return 0
    }
    ; 取第一个可见窗口
    for hwnd in list {
        if (WinExist("ahk_id " hwnd) && DllCall("IsWindowVisible", "ptr", hwnd))
            return hwnd
    }
    return 0
}

; 安全读取字符串字段（Map 或 Obj 兼容）
EXE_GetStr(obj, key) {
    try {
        if (obj is Map)
            return obj.Has(key) && obj[key] != "" ? String(obj[key]) : ""
        return obj.HasOwnProp(key) && obj.%key% != "" ? String(obj.%key%) : ""
    }
    return ""
}

; ---------- 归一化坐标 → 屏幕物理坐标 ----------
; client: {x, y, w, h}（客户区屏幕坐标）
EXE_ToScreen(normPair, client, jitter := 0) {
    px := Round(normPair[1] * (client.w - 1))
    py := Round(normPair[2] * (client.h - 1))
    if (jitter > 0) {
        px += Random(-jitter, jitter)
        py += Random(-jitter, jitter)
    }
    return { x: client.x + px, y: client.y + py }
}

; ---------- 主入口：执行路线 ----------
; route: 解析自 _cache JSON 的对象
; startAt: 从第几个点位开始（1-based，0=全部）
; only: 仅执行第几个点位（1-based，0=不启用）
; 返回 true/false
EXE_RunRoute(route, startAt := 1, only := 0) {
    global EXE_Running, EXE_StopFlag
    if (EXE_Running) {
        EXE_Log("IGNORE: 已有路线在执行中，忽略本次触发")
        return false
    }
    EXE_Running := true
    EXE_StopFlag := false
    firstAction := true
    ok := false
    try {
        target := route["target"]
        name := EXE_GetStr(route, "name")
        hwnd := EXE_FindTargetWindow(target)
        if (!hwnd) {
            EXE_Log("ERROR: 找不到目标窗口 route=" name)
            ToolTip("game_helper: 找不到目标窗口（" name "）")
            SetTimer(() => ToolTip(), -3000)
            return false
        }
        ; 激活目标窗口并等待就绪
        WinActivate("ahk_id " hwnd)
        if !WinWaitActive("ahk_id " hwnd, , 3) {
            EXE_Log("ERROR: 目标窗口无法前置 hwnd=" hwnd)
            return false
        }
        WinGetClientPos(&cx, &cy, &cw, &ch, "ahk_id " hwnd)
        client := { x: cx, y: cy, w: cw, h: ch }
        EXE_Log("START route=" name " hwnd=" hwnd " client=" client.w "x" client.h " startAt=" startAt " only=" only)

        points := route["points"]
        total := points.Length
        idx := 0
        for pt in points {
            idx++
            if (only > 0 && only != idx)
                continue
            if (only = 0 && startAt > 1 && idx < startAt)
                continue
            if (EXE_StopFlag) {
                EXE_Log("STOP at point#" idx "（急停触发）")
                break
            }
            ptName := EXE_GetStr(pt, "name")
            EXE_Log("POINT #" idx " id=" pt["id"] " name=" ptName)
            ; 点位 pos 本身不点击，仅作为定位锚；操作序列才是动作
            ops := pt.Has("operations") ? pt["operations"] : []
            j := 0
            for op in ops {
                j++
                if (EXE_StopFlag) {
                    EXE_Log("STOP at point#" idx " op#" j "（急停触发）")
                    break 2
                }
                t0 := A_TickCount
                EXE_ExecuteOp(op, client)
                if (firstAction && EXE_HotkeyTick > 0) {
                    EXE_Log("LATENCY hotkey->first action = " (t0 - EXE_HotkeyTick) " ms")
                    firstAction := false
                }
            }
        }
        EXE_Log("DONE " (EXE_StopFlag ? "(aborted)" : "(ok)"))
        ok := !EXE_StopFlag
    } catch as e {
        EXE_Log("EXCEPTION: " e.Message " @ " e.What " line " e.Line)
        ok := false
    } finally {
        EXE_Running := false
        EXE_StopFlag := false
    }
    return ok
}

; ---------- 单个操作执行 ----------
EXE_ExecuteOp(op, client) {
    static typeName := Map(
        "click", "点击", "key", "按键", "press", "按住",
        "wheel", "滚轮", "drag", "拖拽", "wait", "等待"
    )
    t := op["type"]
    jitter := op.Has("jitter") ? Integer(op["jitter"]) : 0

    switch t {
        case "click":
            pos := EXE_ToScreen(op["pos"], client, jitter)
            btn := op.Has("button") ? op["button"] : "left"
            clicks := op.Has("clicks") ? Integer(op["clicks"]) : 1
            MouseClick(btn, pos.x, pos.y, clicks)
            EXE_Log("  op click " btn " @ " pos.x "," pos.y " x" clicks)
        case "key":
            k := op["key"]
            if (op.Has("up") && op["up"] = true)
                Send("{" k " up}")
            else
                Send("{" k "}")
            EXE_Log("  op key {" k "}")
        case "press":
            k := op["key"]
            ms := Integer(op["ms"])
            Send("{" k " down}")
            EXE_Sleep(ms)
            Send("{" k " up}")
            EXE_Log("  op press {" k "} " ms "ms")
        case "wheel":
            d := Integer(op["delta"])
            n := Abs(d)
            which := d > 0 ? "WheelUp" : "WheelDown"
            MouseClick(which,,, n)
            EXE_Log("  op wheel " d)
        case "drag":
            from := EXE_ToScreen(op["from"], client, jitter)
            to := EXE_ToScreen(op["to"], client, jitter)
            dur := op.Has("duration") ? Integer(op["duration"]) : 200
            MouseMove(from.x, from.y)
            Click("Down")
            ; 插值移动，模拟拖拽轨迹
            steps := Max(10, Round(dur / 15))
            dx := (to.x - from.x) / steps, dy := (to.y - from.y) / steps
            Loop steps {
                if (EXE_StopFlag)
                    break
                MouseMove(from.x + dx * A_Index, from.y + dy * A_Index, 0)
                EXE_Sleep(Round(dur / steps))
            }
            Click("Up")
            EXE_Log("  op drag " from.x "," from.y " -> " to.x "," to.y)
        case "wait":
            ms := Integer(op["ms"])
            EXE_Log("  op wait " ms "ms")
            EXE_Sleep(ms)
        default:
            EXE_Log("  op UNKNOWN type: " t)
    }
}

; ---------- 急停热键回调 ----------
EXE_StopAllHandler(*) {
    global EXE_StopFlag
    EXE_StopFlag := true
    EXE_Log("HOTKEY stop-all triggered (F12/自定义)")
    ToolTip("game_helper: 已触发急停")
    SetTimer(() => ToolTip(), -1500)
}
