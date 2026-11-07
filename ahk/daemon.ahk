#Requires AutoHotkey v2.0
#SingleInstance Force
; ============================================================
; daemon.ahk - 常驻执行引擎（主路径）
; 职责：
;   1. 读 ../config.json，动态注册原生热键（含急停键）
;   2. 启动/重载时预载 ../routes/_cache/*.json
;   3. 隐藏消息窗口 + RegisterWindowMessage 协议（Ping/Reload/Stop/CopyData-Run）
;   4. 托盘菜单：暂停热键 / 重载配置 / 退出
; Rust 管理器通过 Win32 消息控制本进程
; ============================================================
#Include lib\JSON.ahk
#Include lib\executor.ahk

global DAEMON_BaseDir := A_ScriptDir "\.."
global DAEMON_WinTitle := "game_helper_daemon_v2"   ; 隐藏窗口标题（Rust 用 FindWindowW 定位）
global DAEMON_MsgPing := 0, DAEMON_MsgReload := 0, DAEMON_MsgStop := 0
global DAEMON_Hotkeys := Map()       ; key -> {action, route, startAt}
global DAEMON_Routes := Map()        ; routeName -> {mtime, data}
global DAEMON_Paused := false
global DAEMON_Cursors := Map()       ; routeName -> 下一个待执行点位（1-based，单步模式游标）

EXE_Setup(DAEMON_BaseDir)
EXE_Log("DAEMON boot pid=" ProcessExist())

; ---------- 注册窗口消息 ----------
DAEMON_MsgPing := DllCall("RegisterWindowMessage", "str", "GameHelperDaemon_Ping", "uint")
DAEMON_MsgReload := DllCall("RegisterWindowMessage", "str", "GameHelperDaemon_Reload", "uint")
DAEMON_MsgStop := DllCall("RegisterWindowMessage", "str", "GameHelperDaemon_Stop", "uint")
OnMessage(DAEMON_MsgPing, DAEMON_OnPing)
OnMessage(DAEMON_MsgReload, DAEMON_OnReload)
OnMessage(DAEMON_MsgStop, DAEMON_OnStop)
OnMessage(0x004A, DAEMON_OnCopyData)   ; WM_COPYDATA

; ---------- 托盘（同时确保脚本主窗口已创建）----------
A_TrayMenu.Delete()
A_TrayMenu.Add("暂停/恢复热键", (*) => DAEMON_TogglePause())
A_TrayMenu.Add("重载配置", (*) => DAEMON_LoadAll())
A_TrayMenu.Add()
A_TrayMenu.Add("退出", (*) => DAEMON_GracefulExit())
A_TrayMenu.Default := "暂停/恢复热键"
TraySetIcon("shell32.dll", 44)   ; 齿轮

; ---------- 消息窗口：复用脚本主窗口 ----------
; 注意：本机环境下 Gui() 初始化会静默阻塞，改用 A_ScriptHwnd 主窗口收消息。
; 改名后 Rust 侧通过 FindWindowW 定位。
DetectHiddenWindows(true)
WinSetTitle(DAEMON_WinTitle, "ahk_id " A_ScriptHwnd)
EXE_Log("DAEMON msgwin hwnd=" A_ScriptHwnd)

; ---------- 初始加载 ----------
DAEMON_LoadAll()

TrayTip("game_helper v2 daemon", "热键引擎已启动", 1)
Persistent()

; ============================================================
; 加载：config + 热键注册 + 路线预载
; ============================================================
DAEMON_LoadAll() {
    ; 注销旧热键
    for key in DAEMON_Hotkeys
        Hotkey(key, 0)
    DAEMON_Hotkeys.Clear()

    cfgPath := DAEMON_BaseDir "\config.json"
    if !FileExist(cfgPath) {
        EXE_Log("DAEMON ERROR: 缺少 config.json")
        return
    }
    cfg := JSON.parse(FileRead(cfgPath, "UTF-8"))
    stopKey := cfg["hotkeys"].Has("stopAll") ? cfg["hotkeys"]["stopAll"] : "F12"
    Hotkey(stopKey, EXE_StopAllHandler, "On")
    EXE_Log("DAEMON stopAll=" stopKey)

    count := 0
    if (cfg["hotkeys"].Has("bindings")) {
        for b in cfg["hotkeys"]["bindings"] {
            key := b["hotkey"], action := b["action"]
            if (action != "run_route")
                continue
            routeName := b["route"]
            startAt := b.Has("startAt") ? Integer(b["startAt"]) : 1
            Hotkey(key, DAEMON_OnHotkey.Bind(key, routeName, startAt), "On")
            DAEMON_Hotkeys[key] := { action: action, route: routeName, startAt: startAt }
            count++
        }
    }
    EXE_Log("DAEMON registered " count " hotkey(s), " DAEMON_Hotkeys.Count " total")

    ; 预载所有已转换路线
    DAEMON_Routes.Clear()
    cacheDir := DAEMON_BaseDir "\routes\_cache"
    if DirExist(cacheDir) {
        loop files cacheDir "\*.json" {
            name := StrReplace(A_LoopFileName, ".json", "")
            DAEMON_LoadRoute(name, true)
        }
    }
    EXE_Log("DAEMON preloaded " DAEMON_Routes.Count " route(s)")
    ; 配置重载后游标全部复位（点数/起始点可能已变化），并同步状态文件
    DAEMON_Cursors.Clear()
    DAEMON_WriteCursorState()
    TrayTip("game_helper v2 daemon", "已加载 " DAEMON_Hotkeys.Count " 个热键 / " DAEMON_Routes.Count " 条路线", 1)
}

DAEMON_LoadRoute(name, force := false) {
    path := DAEMON_BaseDir "\routes\_cache\" name ".json"
    if !FileExist(path)
        return false
    mtime := FileGetTime(path, "M")
    if (!force && DAEMON_Routes.Has(name) && DAEMON_Routes[name].mtime = mtime)
        return true
    try {
        data := JSON.parse(FileRead(path, "UTF-8"))
        DAEMON_Routes[name] := { mtime: mtime, data: data }
        return true
    } catch as e {
        EXE_Log("DAEMON route parse failed: " name " -> " e.Message)
        return false
    }
}

; ============================================================
; 热键触发：单步执行（每按一次只跑一个点位，走完回绕到 startAt 循环）
; ============================================================
; extra*：AHK v2.0.19+ 热键触发时会给回调传附加参数，必须吸收掉，
; 否则 BoundFunc(3参) + 附加参数 会报 "Too many parameters" 并弹出错误框
DAEMON_OnHotkey(key, routeName, startAt, extra*) {
    global EXE_HotkeyTick, DAEMON_Cursors
    EXE_HotkeyTick := A_TickCount
    if (DAEMON_Paused) {
        EXE_Log("HOTKEY " key " ignored (paused)")
        return
    }
    if (EXE_Running) {
        EXE_Log("HOTKEY " key " ignored (busy)")
        return
    }
    ; mtime 变化则重载（保存后未重载的兜底）
    if !DAEMON_LoadRoute(routeName) && !DAEMON_Routes.Has(routeName) {
        EXE_Log("HOTKEY " key " ERROR: 路线未缓存 " routeName "（请先在管理器保存/绑定）")
        ToolTip("game_helper: 路线未缓存 " routeName)
        SetTimer(() => ToolTip(), -2000)
        return
    }
    data := DAEMON_Routes[routeName].data
    total := data["points"].Length
    if (total < 1) {
        EXE_Log("HOTKEY " key " ERROR: 路线没有点位 " routeName)
        return
    }
    ; 游标：无记录从 startAt 开始；超出总点数则回绕（走完一轮从头循环）
    cur := DAEMON_Cursors.Has(routeName) ? DAEMON_Cursors[routeName] : startAt
    if (cur > total)
        cur := startAt
    ToolTip("game_helper: " routeName " 第 " cur "/" total " 点")
    SetTimer(() => ToolTip(), -800)
    EXE_Log("HOTKEY " key " -> step " routeName " point " cur "/" total)
    ok := EXE_RunRoute(data, cur, cur)   ; startAt=only=cur：只执行这一个点位
    if (ok) {
        DAEMON_Cursors[routeName] := cur + 1
        EXE_Log("CURSOR " routeName " -> " (cur + 1) "（" cur " 完成）")
        DAEMON_WriteCursorState()
    } else {
        EXE_Log("CURSOR " routeName " 停留在 " cur "（执行未成功，不推进）")
    }
}

; ---------- 游标状态文件：v2/state/cursor.json（manager 轮询显示用） ----------
DAEMON_WriteCursorState() {
    global DAEMON_Cursors
    stateDir := DAEMON_BaseDir "\state"
    DirCreate(stateDir)
    out := Map("routes", Map())
    for _, hk in DAEMON_Hotkeys {
        if (hk.action != "run_route")
            continue
        rn := hk.route
        total := 0
        try total := DAEMON_Routes[rn].data["points"].Length
        startAt := hk.startAt
        lastDone := 0
        if (DAEMON_Cursors.Has(rn))
            lastDone := DAEMON_Cursors[rn] - 1   ; 游标=下一个待执行，已完成=游标-1
        if (lastDone > total)
            lastDone := 0
        out["routes"][rn] := Map("lastDone", lastDone, "total", total, "startAt", startAt)
    }
    path := stateDir "\cursor.json"
    tmp := path ".tmp"
    try {
        FileDelete(tmp)
    }
    FileAppend(JSON.stringify(out), tmp, "UTF-8")
    FileMove(tmp, path, 1)   ; 原子覆盖，避免 manager 读到半截
}

; ============================================================
; 消息协议处理
; ============================================================
DAEMON_OnPing(wParam, lParam, msg, hwnd) {
    ; 返回值 = (热键数 << 1) | (执行中 ? 1 : 0)，SendMessage 可拿到
    return (DAEMON_Hotkeys.Count << 1) | (EXE_Running ? 1 : 0)
}

DAEMON_OnReload(wParam, lParam, msg, hwnd) {
    EXE_Log("MSG reload from " wParam)
    DAEMON_LoadAll()
    return 1
}

DAEMON_OnStop(wParam, lParam, msg, hwnd) {
    EXE_Log("MSG stop from " wParam)
    SetTimer(DAEMON_GracefulExit, -1)
    return 1
}

; WM_COPYDATA: dwData=1 表示运行请求，lpData = JSON {"route":"..","startAt":1,"only":0}
DAEMON_OnCopyData(wParam, lParam, msg, hwnd) {
    global EXE_HotkeyTick
    cds := NumGet(lParam, 2 * A_PtrSize, "ptr")        ; lpData
    cb := NumGet(lParam, A_PtrSize, "uint")            ; cbData
    dw := NumGet(lParam, 0, "uptr")                    ; dwData
    if (dw != 1)
        return 0
    payload := StrGet(cds, cb, "UTF-8")
    try {
        req := JSON.parse(payload)
        EXE_HotkeyTick := A_TickCount
        routeName := req["route"]
        startAt := req.Has("startAt") ? Integer(req["startAt"]) : 1
        only := req.Has("only") ? Integer(req["only"]) : 0
        if !DAEMON_LoadRoute(routeName) && !DAEMON_Routes.Has(routeName) {
            EXE_Log("MSG run: 路线未缓存 " routeName)
            return 0
        }
        EXE_Log("MSG run " routeName " startAt=" startAt " only=" only)
        SetTimer(DAEMON_DeferredRun.Bind(routeName, startAt, only), -1)
        return 1
    } catch as e {
        EXE_Log("MSG run parse error: " e.Message)
        return 0
    }
}

; 延迟执行：让 WM_COPYDATA 立即返回，避免管理器 SendMessage 阻塞到路线跑完
DAEMON_DeferredRun(rn, s, o) {
    if DAEMON_Routes.Has(rn)
        EXE_RunRoute(DAEMON_Routes[rn].data, s, o)
}

; ============================================================
; 托盘与退出
; ============================================================
DAEMON_TogglePause() {
    global DAEMON_Paused
    DAEMON_Paused := !DAEMON_Paused
    for key in DAEMON_Hotkeys
        Hotkey(key, , DAEMON_Paused ? "Off" : "On")
    state := DAEMON_Paused ? "已暂停" : "已恢复"
    EXE_Log("DAEMON hotkeys " state)
    TrayTip("game_helper v2 daemon", "热键 " state, 1)
}

DAEMON_GracefulExit() {
    EXE_Log("DAEMON exit (graceful), running=" EXE_Running)
    if (EXE_Running) {
        EXE_StopFlag := true
        EXE_Log("DAEMON: 已请求当前路线中断")
    }
    ExitApp()
}
