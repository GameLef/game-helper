#requires -Version 7.0
param(
    [string]$ProjectRoot = (Split-Path $PSScriptRoot -Parent),
    [string]$Executable = 'target\debug\game-helper.exe'
)
# Run with pwsh (UTF-8), after the native picker build is ready.
# This script owns its --probe windows and unique .tools sandbox. No playback.
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class PickerProbeNative {
 [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
 [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h,int x,int y,int w,int height,bool repaint);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern bool SetWindowText(IntPtr h,string text);
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref Point p);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out Rect r);
 [DllImport("user32.dll",SetLastError=true)] public static extern bool OpenClipboard(IntPtr owner);
 [DllImport("user32.dll",SetLastError=true)] public static extern bool CloseClipboard();
 [DllImport("user32.dll")] public static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);
}
// PowerShell/COM can switch native threads between calls. Clipboard ownership
// must remain on the same thread from OpenClipboard through CloseClipboard.
public sealed class PickerClipboardLease : IDisposable {
 [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
 private static extern IntPtr CreateWindowExW(uint exStyle, string className, string title, uint style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr parameter);
 [DllImport("user32.dll")] private static extern bool DestroyWindow(IntPtr window);
 private readonly System.Threading.ManualResetEventSlim ready = new(false);
 private readonly System.Threading.ManualResetEventSlim release = new(false);
 private readonly System.Threading.Thread worker;
 private bool opened;
 private bool closed;
 private bool disposed;
 public PickerClipboardLease() {
  worker = new System.Threading.Thread(() => {
   // NULL owners can be reopened as the same owner by other NULL callers;
   // use a private message-only window to model real exclusive contention.
   var owner = CreateWindowExW(0, "STATIC", "GH clipboard acceptance", 0, 0, 0, 0, 0, new IntPtr(-3), IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
   try {
    for (int attempt = 0; owner != IntPtr.Zero && attempt < 50 && !opened; attempt++) {
     opened = PickerProbeNative.OpenClipboard(owner);
     if (!opened) System.Threading.Thread.Sleep(20);
    }
    ready.Set();
    if (opened) {
     try { release.Wait(); }
     finally { closed = PickerProbeNative.CloseClipboard(); }
    }
   } finally { if (owner != IntPtr.Zero) DestroyWindow(owner); }
  });
  worker.IsBackground = true;
  worker.Start();
  if (!ready.Wait(3000) || !opened) { Dispose(); throw new Exception("Cannot acquire test clipboard lock"); }
 }
 public void Dispose() {
  if (disposed) return;
  disposed = true;
  release.Set();
  if (!worker.Join(3000)) throw new Exception("Clipboard lock worker did not finish");
  ready.Dispose(); release.Dispose();
  if (opened && !closed) throw new Exception("Cannot release test clipboard lock");
 }
}
'@
[PickerProbeNative]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
$ProjectRoot = [IO.Path]::GetFullPath($ProjectRoot)
$exe = if ([IO.Path]::IsPathRooted($Executable)) { $Executable } else { Join-Path $ProjectRoot $Executable }
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Build the native executable first: $exe" }
$token = [guid]::NewGuid().ToString('N')
$appRoot = Join-Path $ProjectRoot ('.tools\picker-smoke-' + $token)
$routes = Join-Path $appRoot 'routes'
[IO.Directory]::CreateDirectory($routes) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $appRoot 'ahk')) | Out-Null
[IO.File]::WriteAllText((Join-Path $appRoot 'ahk\daemon.ahk'), '; Inert picker smoke fixture. Never executed.', [Text.UTF8Encoding]::new($false))
$configText = '{"routesDir":"routes","autoStartDaemon":false,"autoReloadOnSave":false,"hotkeys":{"stopAll":"F12","bindings":[]}}'
[IO.File]::WriteAllText((Join-Path $appRoot 'config.json'), $configText, [Text.UTF8Encoding]::new($false))
$statePath = Join-Path $appRoot 'state\picker.json'
$pointName = '起点·中文名称'
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()
$pickerProcess = $null
$targetProcess = $null
$root = $null
$scope = [Windows.Automation.TreeScope]::Descendants
$clipboardBefore = $null
$restoreClipboard = $false
$stateLock = $null

function Wait-Until([string]$Description, [scriptblock]$Predicate, [int]$TimeoutMs = 8000) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    do {
        if (& $Predicate) { return }
        Start-Sleep -Milliseconds 100
    } while ($watch.ElapsedMilliseconds -lt $TimeoutMs)
    throw "Timed out: $Description"
}
function Wait-Window($Process) {
    Wait-Until 'owned native window' {
        $Process.Refresh()
        if ($Process.HasExited) { throw 'Owned test process exited before creating its window' }
        $Process.MainWindowHandle -ne [IntPtr]::Zero
    }
    # A debug executable can briefly report its console before the native HWND.
    Start-Sleep -Milliseconds 700
    $Process.Refresh()
    if ($Process.HasExited -or $Process.MainWindowHandle -eq [IntPtr]::Zero) { throw 'Owned process has no native window' }
}
function Node([string]$Id) {
    if ($root.Current.AutomationId -eq $Id) { return $root }
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, $Id)
    $root.FindFirst($scope, $condition)
}
function Need-Node([string]$Id) {
    Wait-Until "control $Id" { $null -ne (Node $Id) }
    Node $Id
}
function Foreground-Picker {
    [PickerProbeNative]::SetForegroundWindow($pickerProcess.MainWindowHandle) | Out-Null
    Wait-Until 'picker foreground' { [PickerProbeNative]::GetForegroundWindow() -eq $pickerProcess.MainWindowHandle }
}
function Invoke-Id([string]$Id) {
    Foreground-Picker
    Wait-Until "enabled control $Id" { $n = Node $Id; $null -ne $n -and $n.Current.IsEnabled }
    (Node $Id).GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 150
}
function Set-Value([string]$Id, [string]$Text, [int]$SettleMs = 100) {
    $node = Need-Node $Id
    $node.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($Text)
    Wait-Until "input $Id updates" { (Get-Value $Id) -ceq $Text }
    if ($SettleMs -gt 0) { Start-Sleep -Milliseconds $SettleMs }
}
function Get-Value([string]$Id) {
    $node = Node $Id
    if ($null -eq $node) { return $null }
    $node.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value
}
function Get-PointIds {
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Edit)
    foreach ($node in $root.FindAll($scope, $condition)) {
        if ($node.Current.AutomationId -match '^point-name-(\d+)$') { [int]$Matches[1] }
    }
}
function Wait-Points([int[]]$Ids) {
    $expected = (@($Ids | Sort-Object) -join ',')
    Wait-Until "point IDs [$expected]" { (@(Get-PointIds | Sort-Object) -join ',') -eq $expected }
}
function Assert-Points([int[]]$Ids) {
    $actual = @(Get-PointIds | Sort-Object) -join ','
    $expected = @($Ids | Sort-Object) -join ','
    if ($actual -ne $expected) { throw "Point IDs changed: expected [$expected], got [$actual]" }
}
function Press-Key([byte]$Key) {
    try { [PickerProbeNative]::keybd_event($Key, 0, 0, [UIntPtr]::Zero) }
    finally { [PickerProbeNative]::keybd_event($Key, 0, 2, [UIntPtr]::Zero) }
}
function Wait-Focus([string]$Id) {
    Wait-Until "keyboard focus on $Id" {
        $focused = [Windows.Automation.AutomationElement]::FocusedElement
        $null -ne $focused -and $focused.Current.AutomationId -eq $Id
    }
}
function Press-Tab([bool]$Backwards = $false) {
    try {
        if ($Backwards) { [PickerProbeNative]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero) }
        Press-Key 0x09
    } finally {
        [PickerProbeNative]::keybd_event(0x09, 0, 2, [UIntPtr]::Zero)
        [PickerProbeNative]::keybd_event(0x10, 0, 2, [UIntPtr]::Zero)
    }
}
function Escape-Picker {
    Foreground-Picker
    Press-Key 0x1B
    Start-Sleep -Milliseconds 150
}
function Target-Point {
    $targetProcess.Refresh()
    if ($targetProcess.HasExited) { throw 'Cannot position cursor on an exited probe' }
    $position = [PickerProbeNative+Point]::new()
    $position.X = 100; $position.Y = 100
    if (-not [PickerProbeNative]::ClientToScreen($targetProcess.MainWindowHandle, [ref]$position)) { throw 'Cannot locate dedicated probe client area' }
    $position
}
function Focus-Probe {
    [PickerProbeNative]::SetForegroundWindow($targetProcess.MainWindowHandle) | Out-Null
    Wait-Until 'dedicated probe foreground (picker unfocused)' { [PickerProbeNative]::GetForegroundWindow() -eq $targetProcess.MainWindowHandle }
    $position = Target-Point
    # Cursor movement is restricted to the owned --probe client area.
    if (-not [PickerProbeNative]::SetCursorPos($position.X, $position.Y)) { throw 'Cannot move cursor to dedicated probe' }
    Start-Sleep -Milliseconds 100
}
function Send-CaptureKeys {
    try {
        foreach ($key in @(0x11, 0x12, 0x50)) {
            [PickerProbeNative]::keybd_event($key, 0, 0, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 40
        }
    } finally {
        foreach ($key in @(0x50, 0x12, 0x11)) { [PickerProbeNative]::keybd_event($key, 0, 2, [UIntPtr]::Zero) }
    }
}
function Hotkey-Capture { Focus-Probe; Send-CaptureKeys }
function Enable-Hotkey {
    Invoke-Id 'hotkey'
    Wait-Until 'global hotkey enabled' { (Node 'hotkey').Current.Name -eq '释放 Ctrl+Alt+P' }
}
function Start-Probe([string]$Title) {
    $process = Start-Process -FilePath $exe -ArgumentList '--probe' -PassThru
    $owned.Add($process)
    Wait-Window $process
    if (-not [PickerProbeNative]::SetWindowText($process.MainWindowHandle, $Title)) { throw 'Cannot set owned probe title' }
    [PickerProbeNative]::MoveWindow($process.MainWindowHandle, 80, 80, 500, 400, $true) | Out-Null
    $process
}
function Stop-Probe {
    # Destroy only our probe; never send input or move the cursor onto other apps.
    $targetProcess.Kill()
    if (-not $targetProcess.WaitForExit(5000)) { throw 'Owned probe did not exit' }
}
function Start-Picker {
    $script:pickerProcess = Start-Process -FilePath $exe -ArgumentList @('--picker', '--app-root', ('"' + $appRoot + '"')) -PassThru
    $owned.Add($pickerProcess)
    Wait-Window $pickerProcess
    $script:root = [Windows.Automation.AutomationElement]::FromHandle($pickerProcess.MainWindowHandle)
    Wait-Until 'native picker title' { $root.Current.Name -eq '取点工具' }
    foreach ($id in @('picker-root', 'picker-targets', 'picker-points', 'coordinates', 'picker-status', 'picker-search', 'target-summary', 'screen-coordinate', 'client-coordinate', 'normalized-coordinate')) { $null = Need-Node $id }
    foreach ($id in @('picker-stem', 'picker-name', 'save-picker', 'export-picker')) {
        if ($null -ne (Node $id)) { throw "Obsolete route save/export control remains: $id" }
    }
}
function Close-Picker {
    if (-not $pickerProcess.CloseMainWindow()) { throw 'Cannot request owned picker close' }
    if (-not $pickerProcess.WaitForExit(8000)) { throw 'Normal close did not flush and exit automatically' }
}
function Select-Probe([string]$Title) {
    Set-Value 'picker-search' ''
    Invoke-Id 'refresh-windows'
    $targetId = 'target-' + $targetProcess.MainWindowHandle.ToInt64()
    Wait-Until 'dedicated native target in list' { $null -ne (Node $targetId) }
    Set-Value 'picker-search' ('no-match-' + $token)
    Wait-Until 'search filters unrelated target' { $null -eq (Node $targetId) }
    Set-Value 'picker-search' $Title
    Wait-Until 'search finds dedicated target' { $null -ne (Node $targetId) }
    Invoke-Id $targetId
    # GPUI may expose an offscreen list row whose Invoke is a no-op.
    Start-Sleep -Milliseconds 350
    $summary = Node 'target-summary'
    if ($null -eq $summary -or -not $summary.Current.Name.Contains($Title)) {
        Invoke-Id 'crosshair'
        $null = Need-Node 'picker-countdown'
        Focus-Probe
        Wait-Until 'crosshair chooses dedicated probe' {
            $summary = Node 'target-summary'
            $null -ne $summary -and $summary.Current.Name.Contains($Title)
        } 6000
    }
    Set-Value 'picker-search' ('no-match-' + $token)
    if (-not (Need-Node 'target-summary').Current.Name.Contains($Title)) { throw 'Filtering cleared the selected target' }
    Set-Value 'picker-search' ''
}
function Switch-ProbeImmediately([string]$Title) {
    # Prepare the row before rapid edits, then invoke without the search/refresh delay.
    $targetId = 'target-' + $targetProcess.MainWindowHandle.ToInt64()
    Foreground-Picker
    (Need-Node $targetId).GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Wait-Until 'immediate target switch' { (Node 'target-summary').Current.Name.Contains($Title) }
}
function Capture-Delayed([int[]]$ExpectedIds) {
    Invoke-Id 'capture'
    $null = Need-Node 'picker-countdown'
    Focus-Probe
    Wait-Points $ExpectedIds
    Wait-Until 'capture countdown disappears' { $null -eq (Node 'picker-countdown') }
}
function Wait-Confirmation([string]$Action) {
    $null = Need-Node 'picker-confirmation'
    $null = Need-Node $Action
}
function Assert-NoConfirmation { Wait-Until 'confirmation dismissed' { $null -eq (Node 'picker-confirmation') } }
function Assert-Keys($Object, [string[]]$Keys, [string]$Description) {
    if ($null -eq $Object -or (@($Object.PSObject.Properties.Name | Sort-Object) -join ',') -cne (@($Keys | Sort-Object) -join ',')) { throw "Unexpected JSON fields: $Description" }
}
function Assert-Target($Target) {
    Assert-Keys $Target @('process', 'windowClass', 'title') 'target'
    foreach ($key in @('process', 'windowClass', 'title')) {
        if ($Target.$key -isnot [string]) { throw "Non-string target field: $key" }
    }
    if (-not $Target.process -and -not $Target.windowClass) { throw 'Target has no process or windowClass' }
}
function Normalize-Process([string]$ProcessName) {
    [regex]::Replace($ProcessName, '[A-Z]', { param($m) $m.Value.ToLowerInvariant() })
}
function Same-Target($Left, $Right) {
    $null -ne $Left -and $null -ne $Right -and
        (Normalize-Process $Left.process) -ceq (Normalize-Process $Right.process) -and
        $Left.windowClass -ceq $Right.windowClass -and $Left.title -ceq $Right.title
}
function Read-State {
    $state = [IO.File]::ReadAllText($statePath) | ConvertFrom-Json
    Assert-Keys $state @('format', 'lastTarget', 'targets') 'picker state'
    if ($state.format -cne 'picker/v1' -or $state.targets -isnot [array]) { throw 'Invalid picker/v1 state' }
    if ($null -ne $state.lastTarget) { Assert-Target $state.lastTarget }
    foreach ($group in $state.targets) {
        Assert-Keys $group @('target', 'points') 'target group'
        Assert-Target $group.target
        if ($group.points -isnot [array]) { throw 'Points must be a JSON array' }
        $ids = @()
        foreach ($point in $group.points) {
            Assert-Keys $point @('id', 'name', 'position') 'captured point'
            if ($point.id -isnot [ValueType] -or $point.id -is [bool] -or $point.id -lt 1 -or [Math]::Floor([double]$point.id) -ne $point.id -or $ids -contains $point.id -or $point.name -isnot [string]) { throw 'Invalid stable point ID/name' }
            $ids += $point.id
            if ($point.position -isnot [array] -or $point.position.Count -ne 2) { throw 'position must contain exactly two coordinates' }
            foreach ($coordinate in $point.position) {
                if ($coordinate -isnot [ValueType] -or $coordinate -is [bool] -or -not [double]::IsFinite([double]$coordinate) -or $coordinate -lt 0 -or $coordinate -gt 1) { throw 'Invalid normalized coordinate' }
            }
        }
    }
    $state
}
function Get-Group($State, $Target) {
    $groups = @($State.targets | Where-Object { Same-Target $_.target $Target })
    if ($groups.Count -ne 1) { throw 'Expected exactly one group for the full target identity' }
    $groups[0]
}
function Wait-Saved($Target, [int[]]$Ids, [int]$NamedId = 0, [string]$Name = '') {
    $expected = @($Ids | Sort-Object) -join ','
    Wait-Until "automatic picker state save [$expected] / $Name" {
        try {
            $state = Read-State
            $group = Get-Group $state $Target
            $matchesName = $NamedId -eq 0 -or @($group.points | Where-Object { $_.id -eq $NamedId -and $_.name -ceq $Name }).Count -eq 1
            (Same-Target $state.lastTarget $Target) -and $matchesName -and (@($group.points.id | Sort-Object) -join ',') -eq $expected
        } catch { $false }
    }
}
function Assert-Contract($Group, [int]$PointId, [string]$Name, [double[]]$Expected) {
    $point = @($Group.points | Where-Object { $_.id -eq $PointId })
    if ($point.Count -ne 1 -or $point[0].name -cne $Name) { throw 'Picker state lost stable point ID or Unicode name' }
    for ($i = 0; $i -lt 2; $i++) {
        if ([Math]::Abs($point[0].position[$i] - $Expected[$i]) -gt 0.000001) { throw 'Unfocused capture lost normalized client precision' }
    }
}
function Write-CopyDiagnostics([string]$Control) {
    Write-Warning "Copy failure diagnostics: $Control"
    try { Write-Warning ('picker-status: ' + (Node 'picker-status').Current.Name) } catch { Write-Warning "picker-status unavailable: $_" }
    try { Write-Warning ('clipboard: ' + (ConvertTo-Json -InputObject ([string](Get-Clipboard -Raw)) -Compress)) } catch { Write-Warning "clipboard unavailable: $_" }
    try { Write-Warning ('picker bounds: ' + $root.Current.BoundingRectangle) } catch { Write-Warning "picker bounds unavailable: $_" }
    $ids = @($Control)
    if ($Control -match '^copy-(?:coordinate|point)-(\d+)$') {
        $pointId = $Matches[1]
        $ids = @("copy-coordinate-$pointId", "copy-point-$pointId", "point-name-$pointId")
    }
    foreach ($id in $ids) {
        try {
            $node = Node $id
            if ($null -eq $node) { Write-Warning "${id}: missing"; continue }
            Write-Warning ("{0}: enabled={1}; offscreen={2}; bounds={3}; name={4}" -f $id, $node.Current.IsEnabled, $node.Current.IsOffscreen, $node.Current.BoundingRectangle, $node.Current.Name)
        } catch { Write-Warning "${id}: diagnostics unavailable: $_" }
    }
}
function Assert-Copy([string]$Control, [double[]]$Expected, [int]$ClipboardHoldMs = 0) {
    $clipboardLease = $null
    try {
        $sentinel = 'picker-clipboard-' + [guid]::NewGuid().ToString('N')
        Set-Clipboard -Value $sentinel
        if ($ClipboardHoldMs -gt 0) {
            # Resolve UIA and focus before acquiring the clipboard; invoke while it is locked.
            Foreground-Picker
            Wait-Until "enabled control $Control for clipboard contention" { $n = Node $Control; $null -ne $n -and $n.Current.IsEnabled }
            $invoke = (Node $Control).GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern)
            $clipboardLease = [PickerClipboardLease]::new()
            try {
                $invoke.Invoke()
                Start-Sleep -Milliseconds $ClipboardHoldMs
            } finally {
                $clipboardLease.Dispose()
                $clipboardLease = $null
            }
        } else {
            Invoke-Id $Control
        }
        Wait-Until "coordinate JSON copied by $Control" {
            try { ([string](Get-Clipboard -Raw)) -cne $sentinel } catch { $false }
        }
        $text = [string](Get-Clipboard -Raw)
        if ($text -cnotmatch '^\[([01]\.\d{6}), ([01]\.\d{6})\]$') { throw "Clipboard is not a six-decimal coordinate pair: $text" }
        $pair = ConvertFrom-Json -InputObject $text -NoEnumerate
        if ($pair -isnot [array] -or $pair.Count -ne 2) { throw 'Clipboard JSON must be a coordinate array' }
        for ($i = 0; $i -lt 2; $i++) {
            if ([Math]::Abs($pair[$i] - $Expected[$i]) -gt 0.000001) { throw "Clipboard precision mismatch from $Control" }
        }
        $text
    } catch {
        # Release before diagnostics so Get-Clipboard can inspect the actual result.
        if ($null -ne $clipboardLease) { $clipboardLease.Dispose(); $clipboardLease = $null }
        Write-CopyDiagnostics $Control
        throw
    } finally {
        if ($null -ne $clipboardLease) { $clipboardLease.Dispose() }
    }
}
function Assert-BothCopies([int]$Id, [double[]]$Expected) {
    $coordinate = Assert-Copy "copy-coordinate-$Id" $Expected
    $button = Assert-Copy "copy-point-$Id" $Expected
    if ($coordinate -cne $button) { throw 'Coordinate click and copy button produced different JSON' }
}
function Group-Json($Target) {
    $group = Get-Group (Read-State) $Target
    # Canonicalize only ASCII process characters; class/title remain exact.
    $group.target.process = Normalize-Process $group.target.process
    $group | ConvertTo-Json -Depth 20 -Compress
}
function Assert-GroupUnchanged($Target, [string]$ExpectedJson) {
    if ((Group-Json $Target) -cne $ExpectedJson) { throw 'Editing/deleting/clearing one target changed another target group' }
}
function Assert-FileBytes([byte[]]$Expected) {
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($statePath)) -cne [Convert]::ToBase64String($Expected)) { throw 'Picker overwrote the protected external/corrupt state file' }
}
function Wait-PersistenceError([string]$Label = '重试') {
    Wait-Until 'persistence error with retry action' { $n = Node 'retry-persistence'; $null -ne $n -and $n.Current.IsEnabled -and $n.Current.Name -eq $Label }
}
function Rename-Rapidly([int]$Id, [string]$FinalName) {
    # No settle sleeps: mutations overlap debounce/in-flight saves before switching/closing.
    foreach ($name in @('快速改名·一', '快速改名·二', '快速改名·三', $FinalName)) { Set-Value "point-name-$Id" $name 0 }
}

try {
    Write-Output "Isolated picker sandbox: $appRoot"
    try { $clipboardBefore = Get-Clipboard -Raw; $restoreClipboard = $true } catch { }
    $titleA = 'GH Picker Acceptance ' + $token
    $titleB = $titleA.ToLowerInvariant() # Exact title matching must distinguish case.
    $titleC = 'GH Picker Replacement ' + $token
    $targetProcess = Start-Probe $titleA
    Start-Picker
    Select-Probe $titleA
    Enable-Hotkey
    Hotkey-Capture
    Wait-Points @(1)
    Set-Value 'point-name-1' $pointName
    $client = [PickerProbeNative+Rect]::new()
    if (-not [PickerProbeNative]::GetClientRect($targetProcess.MainWindowHandle, [ref]$client)) { throw 'Cannot read dedicated probe client bounds' }
    $expected = @((100.0 / ($client.Right - 1)), (100.0 / ($client.Bottom - 1)))
    Wait-Until 'first Unicode point automatically persisted' {
        try { @((Read-State).targets | Where-Object { $_.target.title -ceq $titleA -and $_.points.Count -eq 1 -and $_.points[0].name -ceq $pointName }).Count -eq 1 } catch { $false }
    }
    $identityA = (Read-State).lastTarget
    if ($identityA.title -cne $titleA -or $identityA.process -notmatch '^[\x00-\x7f]+$') { throw 'Saved target is not the owned ASCII-process probe' }
    Wait-Saved $identityA @(1) 1 $pointName
    Assert-Contract (Get-Group (Read-State) $identityA) 1 $pointName $expected
    Assert-BothCopies 1 $expected
    # One invoke must survive temporary OpenClipboard contention without a second click.
    $null = Assert-Copy 'copy-point-1' $expected 125

    Invoke-Id 'capture'
    $null = Need-Node 'picker-countdown'
    Invoke-Id 'cancel-countdown'
    Wait-Until 'button cancels countdown' { $null -eq (Node 'picker-countdown') }
    Focus-Probe
    Start-Sleep -Milliseconds 2600
    Assert-Points @(1)
    Invoke-Id 'capture'
    $null = Need-Node 'picker-countdown'
    Escape-Picker
    Wait-Until 'Escape cancels countdown' { $null -eq (Node 'picker-countdown') }
    Focus-Probe
    Start-Sleep -Milliseconds 2600
    Assert-Points @(1)
    Invoke-Id 'crosshair'
    $null = Need-Node 'picker-countdown'
    Invoke-Id 'cancel-countdown'
    Wait-Until 'crosshair countdown cancelled' { $null -eq (Node 'picker-countdown') }
    Focus-Probe
    Start-Sleep -Milliseconds 3400
    Assert-Points @(1)
    if (-not (Need-Node 'target-summary').Current.Name.Contains($titleA)) { throw 'Cancelled crosshair changed selected target after expiry' }
    Capture-Delayed @(1, 2)
    Capture-Delayed @(1, 2, 3)
    Invoke-Id 'clear-picker'
    Wait-Confirmation 'confirm-clear'
    Wait-Focus 'cancel-confirmation'
    Press-Tab; Wait-Focus 'confirm-clear'
    Press-Tab; Wait-Focus 'cancel-confirmation'
    Press-Tab $true; Wait-Focus 'confirm-clear'
    Press-Tab $true; Wait-Focus 'cancel-confirmation'
    Hotkey-Capture
    Start-Sleep -Milliseconds 500
    Assert-Points @(1, 2, 3)
    Wait-Confirmation 'confirm-clear'
    Invoke-Id 'cancel-confirmation'
    Assert-NoConfirmation
    Invoke-Id 'clear-picker'
    Wait-Confirmation 'confirm-clear'
    Escape-Picker
    Assert-NoConfirmation
    Assert-Points @(1, 2, 3)

    # Same process/class with a title differing only by ASCII case gets a new list.
    [PickerProbeNative]::SetWindowText($targetProcess.MainWindowHandle, $titleB) | Out-Null
    Select-Probe $titleB
    Wait-Points @()
    Capture-Delayed @(1)
    Set-Value 'point-name-1' '另一个标题·保留点'
    $identityB = [pscustomobject]@{ process = $identityA.process; windowClass = $identityA.windowClass; title = $titleB }
    Wait-Saved $identityB @(1) 1 '另一个标题·保留点'
    $groupB = Group-Json $identityB
    [PickerProbeNative]::SetWindowText($targetProcess.MainWindowHandle, $titleA) | Out-Null
    Select-Probe $titleA
    Wait-Points @(1, 2, 3)
    if ((Get-Value 'point-name-1') -cne $pointName) { throw 'Switching titles lost the original Unicode name' }
    Invoke-Id 'remove-point-1'
    Wait-Points @(2, 3)
    Wait-Saved $identityA @(2, 3)
    Assert-GroupUnchanged $identityB $groupB
    Invoke-Id 'clear-picker'
    Wait-Confirmation 'confirm-clear'
    Foreground-Picker
    Wait-Focus 'cancel-confirmation'
    Press-Tab; Wait-Focus 'confirm-clear'
    Press-Key 0x0D
    Assert-NoConfirmation
    Wait-Points @()
    Wait-Saved $identityA @()
    Assert-GroupUnchanged $identityB $groupB
    Capture-Delayed @(1)

    # Prepare B's current HWND row before edits, then switch immediately after the last change.
    [PickerProbeNative]::SetWindowText($targetProcess.MainWindowHandle, $titleB) | Out-Null
    Invoke-Id 'refresh-windows'
    Set-Value 'picker-search' $titleB
    $null = Need-Node ('target-' + $targetProcess.MainWindowHandle.ToInt64())
    $switchName = '快速改名·切换后最终名称'
    Rename-Rapidly 1 $switchName
    Switch-ProbeImmediately $titleB
    Wait-Points @(1)
    Wait-Saved $identityB @(1) 1 '另一个标题·保留点'
    Assert-Contract (Get-Group (Read-State) $identityA) 1 $switchName $expected
    Start-Sleep -Milliseconds 700
    Assert-Contract (Get-Group (Read-State) $identityA) 1 $switchName $expected
    Assert-GroupUnchanged $identityB $groupB
    [PickerProbeNative]::SetWindowText($targetProcess.MainWindowHandle, $titleA) | Out-Null
    Select-Probe $titleA
    $closeName = '快速改名·关闭后最终名称'
    Rename-Rapidly 1 $closeName
    Close-Picker
    Assert-Contract (Get-Group (Read-State) $identityA) 1 $closeName $expected
    Assert-GroupUnchanged $identityB $groupB

    # Seed uppercase process plus a class-case decoy while closed, then reopen offline.
    Stop-Probe
    $state = Read-State
    foreach ($group in $state.targets) { $group.target.process = $group.target.process.ToUpperInvariant() }
    $state.lastTarget.process = $state.lastTarget.process.ToUpperInvariant()
    $classVariant = $identityA.windowClass.ToUpperInvariant()
    if ($classVariant -ceq $identityA.windowClass) { $classVariant = $identityA.windowClass.ToLowerInvariant() }
    if ($classVariant -ceq $identityA.windowClass) { throw 'Probe class must contain an ASCII letter for the exact-class case test' }
    $decoyTarget = [pscustomobject]@{ process = $identityA.process.ToUpperInvariant(); windowClass = $classVariant; title = $titleA }
    $state.targets += [pscustomobject]@{ target = $decoyTarget; points = @([pscustomobject]@{ id = 71; name = '类名大小写隔离'; position = @(0.123456, 0.654321) }) }
    [IO.File]::WriteAllText($statePath, ($state | ConvertTo-Json -Depth 20), [Text.UTF8Encoding]::new($false))
    $decoyJson = Group-Json $decoyTarget
    Start-Picker
    Wait-Points @(1)
    if ((Get-Value 'point-name-1') -cne $closeName -or -not (Node 'target-summary').Current.Name.Contains($titleA)) { throw 'Offline restart did not restore last target/name' }
    if ((Need-Node 'capture').Current.IsEnabled) { throw 'Offline restoration incorrectly reattached a live capture target' }
    Assert-BothCopies 1 $expected
    Enable-Hotkey
    Foreground-Picker
    Send-CaptureKeys
    Start-Sleep -Milliseconds 400
    Assert-Points @(1)
    $targetProcess = Start-Probe $titleA
    Select-Probe $titleA
    Wait-Points @(1)
    if ((Get-Value 'point-name-1') -cne $closeName) { throw 'Same-title probe restart did not restore its list' }
    Hotkey-Capture
    Wait-Points @(1, 2)
    Wait-Saved $identityA @(1, 2)
    Assert-GroupUnchanged $decoyTarget $decoyJson
    Assert-GroupUnchanged $identityB $groupB

    # The delayed capture and the global hotkey both reject an invalidated owned target.
    Invoke-Id 'capture'
    $null = Need-Node 'picker-countdown'
    Stop-Probe
    Foreground-Picker
    Wait-Until 'invalid target countdown completes' { $null -eq (Node 'picker-countdown') } 5000
    Send-CaptureKeys
    Wait-Until 'lost target reported' { ((Node 'coordinates').Current.Name + ' ' + (Node 'picker-status').Current.Name) -match '失效|不可用|失败' }
    Start-Sleep -Milliseconds 400
    Assert-Points @(1, 2)
    $targetProcess = Start-Probe $titleC
    Select-Probe $titleC
    Wait-Points @()
    Capture-Delayed @(1)
    Set-Value 'point-name-1' '新标题·自动保存'
    $identityC = [pscustomobject]@{ process = $identityA.process; windowClass = $identityA.windowClass; title = $titleC }
    Wait-Saved $identityC @(1) 1 '新标题·自动保存'
    Assert-GroupUnchanged $identityB $groupB
    Assert-GroupUnchanged $decoyTarget $decoyJson

    # External changes must survive retries and close cancellation byte for byte.
    $baselineBytes = [IO.File]::ReadAllBytes($statePath)
    $externalBytes = [Text.UTF8Encoding]::new($false).GetBytes(([IO.File]::ReadAllText($statePath) + "`n `n"))
    [IO.File]::WriteAllBytes($statePath, $externalBytes)
    Set-Value 'point-name-1' '冲突后·内存保留'
    Wait-PersistenceError
    Assert-FileBytes $externalBytes
    Assert-BothCopies 1 $expected
    Invoke-Id 'retry-persistence'
    Wait-PersistenceError
    Assert-FileBytes $externalBytes
    $pickerProcess.CloseMainWindow() | Out-Null
    Wait-Confirmation 'retry-close'
    $null = Need-Node 'discard-close'
    Wait-Focus 'cancel-confirmation'
    Foreground-Picker
    Press-Tab; Wait-Focus 'retry-close'
    Press-Tab; Wait-Focus 'discard-close'
    Press-Tab; Wait-Focus 'cancel-confirmation'
    Press-Tab $true; Wait-Focus 'discard-close'
    Press-Tab $true; Wait-Focus 'retry-close'
    Press-Tab $true; Wait-Focus 'cancel-confirmation'
    Escape-Picker
    Assert-NoConfirmation
    if ((Get-Value 'point-name-1') -cne '冲突后·内存保留') { throw 'Escape from failed close lost in-memory edits' }
    $pickerProcess.CloseMainWindow() | Out-Null
    Wait-Confirmation 'retry-close'
    Invoke-Id 'cancel-confirmation'
    Assert-NoConfirmation
    Set-Value 'point-name-1' '继续编辑·重试后保存'
    Wait-PersistenceError
    Assert-FileBytes $externalBytes
    # Reinstating the exact snapshot resolves the conflict without accepting external changes.
    [IO.File]::WriteAllBytes($statePath, $baselineBytes)
    Invoke-Id 'retry-persistence'
    Wait-Saved $identityC @(1) 1 '继续编辑·重试后保存'
    Wait-Until 'write retry clears error' { $null -eq (Node 'retry-persistence') }

    # A genuine sharing failure also keeps edits and allows retry-close once released.
    $stateLock = [IO.File]::Open($statePath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
    Set-Value 'point-name-1' '写入失败·关闭重试'
    Wait-PersistenceError
    $pickerProcess.CloseMainWindow() | Out-Null
    Wait-Confirmation 'retry-close'
    Invoke-Id 'retry-close'
    Wait-Confirmation 'retry-close'
    $pickerProcess.Refresh()
    if ($pickerProcess.HasExited) { throw 'Failed retry-close exited despite the locked state file' }
    $stateLock.Dispose(); $stateLock = $null
    Invoke-Id 'retry-close'
    if (-not $pickerProcess.WaitForExit(8000)) { throw 'Retry-close did not flush and exit after the sharing failure was repaired' }
    Assert-Contract (Get-Group (Read-State) $identityC) 1 '写入失败·关闭重试' $expected

    Start-Picker
    Wait-Points @(1)
    if ((Get-Value 'point-name-1') -cne '写入失败·关闭重试') { throw 'Retry-close did not persist the final Unicode name' }
    Assert-BothCopies 1 $expected
    # Discard is explicit, leaves the external file untouched, and drops only unsaved edits.
    $externalBytes = [Text.UTF8Encoding]::new($false).GetBytes(([IO.File]::ReadAllText($statePath) + "`n  `n"))
    [IO.File]::WriteAllBytes($statePath, $externalBytes)
    Set-Value 'point-name-1' '明确放弃·不应落盘'
    Wait-PersistenceError
    $pickerProcess.CloseMainWindow() | Out-Null
    Wait-Confirmation 'discard-close'
    Invoke-Id 'discard-close'
    if (-not $pickerProcess.WaitForExit(8000)) { throw 'Explicit discard-close did not exit' }
    Assert-FileBytes $externalBytes
    Assert-Contract (Get-Group (Read-State) $identityC) 1 '写入失败·关闭重试' $expected

    # Corrupt reads preserve bytes, including on normal close. Retry rereads a repaired file.
    $goodBytes = [IO.File]::ReadAllBytes($statePath)
    $corruptBytes = [Text.UTF8Encoding]::new($false).GetBytes('{"format":"picker/v1","targets":[损坏-' + $token)
    [IO.File]::WriteAllBytes($statePath, $corruptBytes)
    Start-Picker
    Wait-PersistenceError '重新读取'
    Assert-Points @()
    foreach ($id in @('capture', 'hotkey', 'crosshair')) { if ((Need-Node $id).Current.IsEnabled) { throw "Read failure did not disable $id" } }
    Assert-FileBytes $corruptBytes
    Close-Picker
    Assert-FileBytes $corruptBytes
    Start-Picker
    Wait-PersistenceError '重新读取'
    Invoke-Id 'retry-persistence'
    Wait-PersistenceError '重新读取'
    Assert-FileBytes $corruptBytes
    [IO.File]::WriteAllBytes($statePath, $goodBytes)
    Invoke-Id 'retry-persistence'
    Wait-Until 'read retry succeeds' { $null -eq (Node 'retry-persistence') -and $null -ne (Node 'point-name-1') }
    Wait-Points @(1)
    if ((Get-Value 'point-name-1') -cne '写入失败·关闭重试' -or (Need-Node 'capture').Current.IsEnabled) { throw 'Repaired read did not restore offline state' }
    Assert-FileBytes $goodBytes
    Assert-BothCopies 1 $expected
    Select-Probe $titleC
    Wait-Points @(1)
    Wait-Saved $identityC @(1) 1 '写入失败·关闭重试'
    Close-Picker
    Assert-GroupUnchanged $identityB $groupB
    Assert-GroupUnchanged $decoyTarget $decoyJson
    if ([IO.File]::ReadAllText((Join-Path $appRoot 'config.json')) -cne $configText) { throw 'Picker unexpectedly changed sandbox daemon configuration' }
    if (@(Get-ChildItem -LiteralPath $routes -Recurse -File).Count -ne 0) { throw 'Picker unexpectedly created route/export/cache files' }
    Write-Output 'Passed: isolated probe selection/search; automatic picker/v1 state with Chinese names and positions; both coordinate-copy entries as six-decimal JSON with 125ms clipboard contention recovery; exact title/class and ASCII-insensitive process restoration; title switching, offline/restarted target recovery; stable IDs, scoped delete/clear; countdown button/Escape, unfocused hotkey, invalid target and modal keyboard; rapid edits with immediate switch/close; file conflict preservation, write retry, failed-close retry/continue/discard, corrupt-file reread and automatic close flush. No playback requested. Actual route-manager paste is covered by smoke-picker-coordinates.ps1.'
} finally {
    if ($null -ne $stateLock) { $stateLock.Dispose() }
    foreach ($key in @(0x50, 0x12, 0x11, 0x1B, 0x09, 0x10, 0x0D)) { [PickerProbeNative]::keybd_event($key, 0, 2, [UIntPtr]::Zero) }
    foreach ($process in $owned) {
        $process.Refresh()
        if (-not $process.HasExited) {
            $process.CloseMainWindow() | Out-Null
            if (-not $process.WaitForExit(1000)) { $process.Kill(); $process.WaitForExit(5000) | Out-Null }
        }
        $process.Dispose()
    }
    if ($restoreClipboard) {
        try { if ($null -eq $clipboardBefore) { Set-Clipboard -Value '' } else { Set-Clipboard -Value $clipboardBefore } } catch { Write-Warning 'Could not restore clipboard text' }
    }
    Write-Output "Sandbox retained for inspection: $appRoot"
}
