#Requires AutoHotkey v2.0
#SingleInstance Force
; 常驻执行服务。Rust 管理器通过 Win32 消息控制本进程。
#Include lib\JSON.ahk
#Include lib\executor.ahk

global DAEMON_BaseDir := A_ScriptDir "\.."
global DAEMON_WinTitle := "game_helper_daemon_v2"
global DAEMON_Hotkeys := Map()       ; canonical identity -> {key, action, route, startAt}
global DAEMON_Routes := Map()
global DAEMON_Paused := false
global DAEMON_StopKey := ""
global DAEMON_Cursors := Map()
global DAEMON_RunPending := false
global DAEMON_CaptureOwner := 0
global DAEMON_CaptureOwnerPid := 0
global DAEMON_CaptureDeadline := 0
global DAEMON_CapturePause := false

EXE_Setup(DAEMON_BaseDir)
EXE_Log("DAEMON boot pid=" ProcessExist())

OnMessage(DllCall("RegisterWindowMessage", "str", "GameHelperDaemon_Ping", "uint"), DAEMON_OnPing)
OnMessage(DllCall("RegisterWindowMessage", "str", "GameHelperDaemon_Reload", "uint"), DAEMON_OnReload)
OnMessage(DllCall("RegisterWindowMessage", "str", "GameHelperDaemon_Stop", "uint"), DAEMON_OnStop)
OnMessage(DllCall("RegisterWindowMessage", "str", "GameHelperDaemon_CaptureBegin", "uint"), DAEMON_OnCaptureBegin)
OnMessage(DllCall("RegisterWindowMessage", "str", "GameHelperDaemon_CaptureRenew", "uint"), DAEMON_OnCaptureRenew)
OnMessage(DllCall("RegisterWindowMessage", "str", "GameHelperDaemon_CaptureEnd", "uint"), DAEMON_OnCaptureEnd)
OnMessage(0x004A, DAEMON_OnCopyData)

A_TrayMenu.Delete()
A_TrayMenu.Add("暂停/恢复热键", (*) => DAEMON_TogglePause())
A_TrayMenu.Add("重载配置", (*) => DAEMON_LoadAll())
A_TrayMenu.Add()
A_TrayMenu.Add("退出", (*) => DAEMON_GracefulExit())
A_TrayMenu.Default := "暂停/恢复热键"
TraySetIcon("shell32.dll", 44)
DetectHiddenWindows(true)
WinSetTitle(DAEMON_WinTitle, "ahk_id " A_ScriptHwnd)
EXE_Log("DAEMON msgwin hwnd=" A_ScriptHwnd)
DAEMON_LoadAll()
TrayTip("game_helper v2 daemon", "热键引擎已启动", 1)
Persistent()

; 人工写法兼容 Rust hotkey_to_ahk。配置通常已由 Rust 转为 AHK 写法。
DAEMON_HotkeyToAhk(input) {
    text := Trim(input)
    if (StrLen(text) <= 1 || RegExMatch(text, "^[\^!#+~$*<>]"))
        return text
    ; Preserve the AHK release suffix for plain key names too.
    suffix := ""
    if RegExMatch(text, "i)\s+up$") {
        text := Trim(RegExReplace(text, "i)\s+up$"))
        suffix := " up"
    }
    parts := StrSplit(StrReplace(text, "-", "+"), "+")
    prefix := "", key := ""
    for part in parts {
        part := Trim(part)
        switch StrLower(part) {
            case "ctrl", "control": prefix .= "^"
            case "alt": prefix .= "!"
            case "shift": prefix .= "+"
            case "win", "windows", "super": prefix .= "#"
            case "lctrl", "lcontrol": prefix .= "<^"
            case "rctrl", "rcontrol": prefix .= ">^"
            case "lalt": prefix .= "<!"
            case "ralt": prefix .= ">!"
            case "lshift": prefix .= "<+"
            case "rshift": prefix .= ">+"
            case "lwin": prefix .= "<#"
            case "rwin": prefix .= ">#"
            default:
                if (part = "" || key != "")
                    throw ValueError("热键只能包含一个主键")
                key := part
        }
    }
    if (key = "")
        throw ValueError("热键缺少主键")
    return prefix key suffix
}

; Matches Rust hotkey_canonical: modifier order ^ ! + #, then none/< />;
; ~ and $ change registration behavior, not key identity. * remains distinct.
DAEMON_Canonical(input) {
    text := StrLower(DAEMON_HotkeyToAhk(input))
    mods := Map(), wildcard := false, pos := 1
    while (pos < StrLen(text)) {
        ch := SubStr(text, pos, 1)
        if (ch = "~" || ch = "$") {
            pos++
            continue
        }
        if (ch = "*") {
            wildcard := true
            pos++
            continue
        }
        side := ""
        if (ch = "<" || ch = ">") {
            side := ch
            pos++
            ch := SubStr(text, pos, 1)
        }
        if !InStr("^!+#", ch) || ch = ""
            break
        mods[side ch] := true
        pos++
    }
    key := Trim(RegExReplace(SubStr(text, pos), "\s+", " ")), suffix := ""
    if RegExMatch(key, "i)\s+up$") {
        key := Trim(RegExReplace(key, "i)\s+up$"))
        suffix := " up"
    }
    switch key {
        case "esc": key := "escape"
        case "return": key := "enter"
        case "del": key := "delete"
        case "ins": key := "insert"
        case "bs": key := "backspace"
    }
    if (key = "")
        throw ValueError("热键缺少主键")
    result := wildcard ? "*" : ""
    for symbol in ["^", "!", "+", "#"] {
        for side in ["", "<", ">"] {
            if mods.Has(side symbol)
                result .= side symbol
        }
    }
    return result key suffix
}

DAEMON_ValidStem(name) {
    return Type(name) = "String" && name != "" && name != "." && name != ".."
        && !RegExMatch(name, '[\\/:*?"<>|\x00-\x1f]') && !RegExMatch(name, "[. ]$")
}

DAEMON_IdentityParts(identity) {
    pos := 1, mods := Map(), wildcard := false
    while pos < StrLen(identity) {
        ch := SubStr(identity, pos, 1)
        if ch = "*" {
            wildcard := true
            pos++
            continue
        }
        side := ""
        if ch = "<" || ch = ">" {
            side := ch
            pos++
            ch := SubStr(identity, pos, 1)
        }
        if ch = "" || !InStr("^!+#", ch)
            break
        mask := mods.Has(ch) ? mods[ch] : 0
        mods[ch] := mask | (side = "<" ? 2 : side = ">" ? 4 : 1)
        pos++
    }
    return { key: RegExReplace(SubStr(identity, pos), " up$"), mods: mods, wildcard: wildcard }
}

DAEMON_ModifierMatches(mask, wildcard, state) {
    if !mask
        return wildcard || state = 0
    return (!(mask & 1) || state != 0) && (!(mask & 2) || (state & 1)) && (!(mask & 4) || (state & 2))
}

DAEMON_StopConflicts(identity, stopIdentity) {
    route := DAEMON_IdentityParts(identity), stop := DAEMON_IdentityParts(stopIdentity)
    if route.key != stop.key
        return false
    for symbol in ["^", "!", "+", "#"] {
        rm := route.mods.Has(symbol) ? route.mods[symbol] : 0
        sm := stop.mods.Has(symbol) ? stop.mods[symbol] : 0
        overlaps := false
        for state in [0, 1, 2, 3] {
            if DAEMON_ModifierMatches(rm, route.wildcard, state) && DAEMON_ModifierMatches(sm, stop.wildcard, state) {
                overlaps := true
                break
            }
        }
        if !overlaps
            return false
    }
    return true
}

DAEMON_BuildBindings(cfg, stopKey) {
    winners := Map(), stopIdentity := DAEMON_Canonical(stopKey)
    if !cfg.Has("hotkeys") || !cfg["hotkeys"].Has("bindings")
        return winners
    bindings := cfg["hotkeys"]["bindings"]
    index := bindings.Length
    ; Reverse scan: last enabled run_route binding with the same identity wins.
    while (index > 0) {
        b := bindings[index--]
        try {
            if (b.Has("enabled") && !b["enabled"])
                continue
            action := b.Has("action") ? b["action"] : "run_route"
            if (action != "run_route")
                continue
            key := DAEMON_HotkeyToAhk(b["hotkey"])
            identity := DAEMON_Canonical(key)
            if (winners.Has(identity) || DAEMON_StopConflicts(identity, stopIdentity))
                continue
            route := b["route"]
            startAt := b.Has("startAt") ? Integer(b["startAt"]) : 1
            if !DAEMON_ValidStem(route) || startAt < 1
                throw ValueError("路线名称或起始点无效")
            winners[identity] := { key: key, action: action, route: route, startAt: startAt }
        } catch as e {
            EXE_Log("DAEMON ignored invalid binding: " e.Message)
        }
    }
    return winners
}

DAEMON_DisableSet(bindings, stopKey := "") {
    for _, hk in bindings {
        try Hotkey(hk.key, "Off")
    }
    if (stopKey != "") {
        try Hotkey(stopKey, "Off")
    }
}

DAEMON_RegisterSet(bindings, stopKey) {
    ; Stop is never suppressed by either tray pause or a capture lease.
    Hotkey(stopKey, EXE_StopAllHandler, "On")
    for _, hk in bindings
        Hotkey(hk.key, DAEMON_OnHotkey.Bind(hk.key, hk.route, hk.startAt),
            DAEMON_Paused || DAEMON_CaptureOwner ? "Off" : "On")
}

DAEMON_LoadAll() {
    global DAEMON_Hotkeys, DAEMON_StopKey
    if EXE_Running || DAEMON_RunPending
        return false
    try {
        cfg := JSON.parse(FileRead(DAEMON_BaseDir "\config.json", "UTF-8"))
        stopKey := cfg.Has("hotkeys") && cfg["hotkeys"].Has("stopAll") ? cfg["hotkeys"]["stopAll"] : "F12"
        stopKey := DAEMON_HotkeyToAhk(stopKey)
        next := DAEMON_BuildBindings(cfg, stopKey)
    } catch as e {
        EXE_Log("DAEMON reload rejected: " e.Message)
        return false
    }
    Critical("On")
    previous := DAEMON_Hotkeys, previousStop := DAEMON_StopKey
    DAEMON_DisableSet(previous, previousStop)
    try {
        DAEMON_RegisterSet(next, stopKey)
    } catch as e {
        DAEMON_DisableSet(next, stopKey)
        ; On first boot, keep the validated new stop key even if a route key fails.
        try DAEMON_RegisterSet(previous, previousStop != "" ? previousStop : stopKey)
        EXE_Log("DAEMON registration failed; previous hotkeys restored: " e.Message)
        Critical("Off")
        return false
    }
    DAEMON_Hotkeys := next
    DAEMON_StopKey := stopKey
    EXE_Log("DAEMON registered " DAEMON_Hotkeys.Count " route hotkey(s); stopAll=" stopKey)
    DAEMON_Routes.Clear()
    cacheDir := DAEMON_BaseDir "\routes\_cache"
    if DirExist(cacheDir) {
        loop files cacheDir "\*.json" {
            name := RegExReplace(A_LoopFileName, "i)\.json$")
            DAEMON_LoadRoute(name, true)
        }
    }
    ; Capture is temporary: neither begin/end nor reload while capturing resets
    ; progress. Normal explicit reload retains the existing reset behavior.
    if !DAEMON_CaptureOwner
        DAEMON_Cursors.Clear()
    DAEMON_WriteCursorState()
    Critical("Off")
    EXE_Log("DAEMON preloaded " DAEMON_Routes.Count " route(s)")
    TrayTip("game_helper v2 daemon", "已加载 " DAEMON_Hotkeys.Count " 个热键 / " DAEMON_Routes.Count " 条路线", 1)
    return true
}

DAEMON_LoadRoute(name, force := false) {
    if !DAEMON_ValidStem(name)
        return false
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

DAEMON_OnHotkey(key, routeName, startAt, extra*) {
    global EXE_HotkeyTick, DAEMON_RunPending
    Critical("On")
    DAEMON_CheckCapture()
    if DAEMON_Paused || DAEMON_CaptureOwner || EXE_Running || DAEMON_RunPending {
        Critical("Off")
        return
    }
    DAEMON_RunPending := true
    Critical("Off")
    try {
        EXE_HotkeyTick := A_TickCount
        if !DAEMON_LoadRoute(routeName) && !DAEMON_Routes.Has(routeName) {
            EXE_Log("HOTKEY " key " ERROR: 路线未缓存 " routeName)
            ToolTip("game_helper: 路线未缓存 " routeName)
            SetTimer(() => ToolTip(), -2000)
            return
        }
        data := DAEMON_Routes[routeName].data
        total := data["points"].Length
        if (total < 1 || startAt > total)
            return
        cur := DAEMON_Cursors.Has(routeName) ? DAEMON_Cursors[routeName] : startAt
        if (cur > total)
            cur := startAt
        ToolTip("game_helper: " routeName " 第 " cur "/" total " 点")
        SetTimer(() => ToolTip(), -800)
        EXE_Log("HOTKEY " key " -> step " routeName " point " cur "/" total)
        if EXE_RunRoute(data, cur, cur) {
            DAEMON_Cursors[routeName] := cur + 1
            EXE_Log("CURSOR " routeName " -> " (cur + 1))
            DAEMON_WriteCursorState()
        } else {
            EXE_Log("CURSOR " routeName " 停留在 " cur "（执行未成功，不推进）")
        }
    } finally {
        DAEMON_RunPending := false
    }
}

DAEMON_WriteCursorState() {
    stateDir := DAEMON_BaseDir "\state"
    DirCreate(stateDir)
    out := Map("routes", Map())
    for _, hk in DAEMON_Hotkeys {
        rn := hk.route, total := 0
        try total := DAEMON_Routes[rn].data["points"].Length
        lastDone := DAEMON_Cursors.Has(rn) ? DAEMON_Cursors[rn] - 1 : 0
        if (lastDone > total)
            lastDone := 0
        out["routes"][rn] := Map("lastDone", lastDone, "total", total, "startAt", hk.startAt)
    }
    path := stateDir "\cursor.json", tmp := path ".tmp"
    try FileDelete(tmp)
    FileAppend(JSON.stringify(out), tmp, "UTF-8")
    FileMove(tmp, path, 1)
}

DAEMON_OnPing(wParam, lParam, msg, hwnd) {
    return (DAEMON_Hotkeys.Count << 1) | (EXE_Running || DAEMON_RunPending ? 1 : 0)
}

DAEMON_OnReload(wParam, lParam, msg, hwnd) {
    EXE_Log("MSG reload from " wParam)
    return DAEMON_LoadAll() ? 1 : 0
}

DAEMON_OnStop(wParam, lParam, msg, hwnd) {
    EXE_Log("MSG stop from " wParam)
    SetTimer(DAEMON_GracefulExit, -1)
    return 1
}

; A capture lease only suppresses route hotkeys. The original tray pause state,
; enabled flags, and cursors are preserved; stop remains live. Three seconds of
; silence, owner destruction/PID change, or foreground loss releases the lease.
DAEMON_Now() => DllCall("GetTickCount64", "uint64")

DAEMON_OwnerValid(owner, expectedPid := 0) {
    if !owner || !DllCall("IsWindow", "ptr", owner) || DllCall("GetForegroundWindow", "ptr") != owner
        return false
    pid := 0
    DllCall("GetWindowThreadProcessId", "ptr", owner, "uint*", &pid)
    return pid != 0 && (!expectedPid || pid = expectedPid)
}

DAEMON_SetRouteKeys() {
    for _, hk in DAEMON_Hotkeys
        Hotkey(hk.key, DAEMON_Paused || DAEMON_CaptureOwner ? "Off" : "On")
}

DAEMON_ReleaseCapture(reason) {
    global DAEMON_CaptureOwner, DAEMON_CaptureOwnerPid, DAEMON_CaptureDeadline, DAEMON_Paused
    if !DAEMON_CaptureOwner
        return
    DAEMON_CaptureOwner := 0
    DAEMON_CaptureOwnerPid := 0
    DAEMON_CaptureDeadline := 0
    DAEMON_Paused := DAEMON_CapturePause
    SetTimer(DAEMON_CaptureWatch, 0)
    DAEMON_SetRouteKeys()
    EXE_Log("CAPTURE ended: " reason "; paused=" DAEMON_Paused)
}

DAEMON_CheckCapture() {
    if DAEMON_CaptureOwner && (DAEMON_Now() >= DAEMON_CaptureDeadline || !DAEMON_OwnerValid(DAEMON_CaptureOwner, DAEMON_CaptureOwnerPid))
        DAEMON_ReleaseCapture("lease expired or owner lost focus")
}

DAEMON_CaptureWatch() {
    Critical("On")
    DAEMON_CheckCapture()
}

DAEMON_OnCaptureBegin(owner, lParam, msg, hwnd) {
    global DAEMON_CaptureOwner, DAEMON_CaptureOwnerPid, DAEMON_CaptureDeadline, DAEMON_CapturePause
    Critical("On")
    DAEMON_CheckCapture()
    if lParam || EXE_Running || DAEMON_RunPending || !DAEMON_OwnerValid(owner)
        return 0
    if DAEMON_CaptureOwner && DAEMON_CaptureOwner != owner
        return 0
    if !DAEMON_CaptureOwner {
        DAEMON_CapturePause := DAEMON_Paused
        DAEMON_CaptureOwner := owner
        pid := 0
        DllCall("GetWindowThreadProcessId", "ptr", owner, "uint*", &pid)
        DAEMON_CaptureOwnerPid := pid
        DAEMON_SetRouteKeys()
        EXE_Log("CAPTURE began owner=" owner "; paused=" DAEMON_CapturePause)
    }
    DAEMON_CaptureDeadline := DAEMON_Now() + 3000
    SetTimer(DAEMON_CaptureWatch, 100)
    return 1
}

DAEMON_OnCaptureRenew(owner, lParam, msg, hwnd) {
    global DAEMON_CaptureDeadline
    Critical("On")
    DAEMON_CheckCapture()
    if lParam || !DAEMON_CaptureOwner || DAEMON_CaptureOwner != owner
        return 0
    DAEMON_CaptureDeadline := DAEMON_Now() + 3000
    return 1
}

DAEMON_OnCaptureEnd(owner, lParam, msg, hwnd) {
    Critical("On")
    DAEMON_CheckCapture()
    if lParam || !owner || (DAEMON_CaptureOwner && DAEMON_CaptureOwner != owner)
        return 0
    DAEMON_ReleaseCapture("owner ended")
    return 1
}

; WM_COPYDATA dwData=1: UTF-8 NUL-terminated {route,startAt,only}.
DAEMON_OnCopyData(wParam, lParam, msg, hwnd) {
    global EXE_HotkeyTick, DAEMON_RunPending
    Critical("On")
    DAEMON_CheckCapture()
    if DAEMON_CaptureOwner || EXE_Running || DAEMON_RunPending
        return 0
    cds := NumGet(lParam, 2 * A_PtrSize, "ptr")
    cb := NumGet(lParam, A_PtrSize, "uint")
    dw := NumGet(lParam, 0, "uptr")
    if dw != 1 || !cds || cb < 2 || cb > 65536 || NumGet(cds, cb - 1, "uchar") != 0
        return 0
    try {
        req := JSON.parse(StrGet(cds, cb - 1, "UTF-8"))
        routeName := req["route"]
        startAt := req.Has("startAt") ? Integer(req["startAt"]) : 1
        only := req.Has("only") ? Integer(req["only"]) : 0
        if !DAEMON_LoadRoute(routeName) && !DAEMON_Routes.Has(routeName)
            return 0
        total := DAEMON_Routes[routeName].data["points"].Length
        if startAt < 1 || startAt > total || only < 0 || only > total
            return 0
        EXE_HotkeyTick := A_TickCount
        DAEMON_RunPending := true
        EXE_Log("MSG run " routeName " startAt=" startAt " only=" only)
        SetTimer(DAEMON_DeferredRun.Bind(routeName, startAt, only), -1)
        return 1
    } catch as e {
        EXE_Log("MSG run rejected: " e.Message)
        return 0
    }
}

DAEMON_DeferredRun(rn, s, o) {
    global DAEMON_RunPending
    try {
        if !DAEMON_CaptureOwner && DAEMON_Routes.Has(rn)
            EXE_RunRoute(DAEMON_Routes[rn].data, s, o)
    } finally {
        DAEMON_RunPending := false
    }
}

DAEMON_TogglePause() {
    global DAEMON_Paused
    Critical("On")
    DAEMON_CheckCapture()
    if DAEMON_CaptureOwner
        return
    DAEMON_Paused := !DAEMON_Paused
    DAEMON_SetRouteKeys()
    state := DAEMON_Paused ? "已暂停" : "已恢复"
    EXE_Log("DAEMON hotkeys " state)
    TrayTip("game_helper v2 daemon", "热键 " state, 1)
}

DAEMON_GracefulExit() {
    global EXE_StopFlag
    if EXE_Running {
        EXE_StopFlag := true
        ; Allow the suspended playback thread's finally blocks to release input.
        SetTimer(DAEMON_GracefulExit, -50)
        return
    }
    EXE_Log("DAEMON exit (graceful)")
    ExitApp()
}
