param([Parameter(Mandatory = $true)][int]$TargetProcessId)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding $false
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$process = Get-Process -Id $TargetProcessId
$root = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
$editCondition = New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty), ([Windows.Automation.ControlType]::Edit)
$edits = $root.FindAll([Windows.Automation.TreeScope]::Descendants, $editCondition)
if ($edits.Count -lt 4) { throw 'Expected main input, two table inputs, and editor.' }
$input = $edits[0].GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)
$original = $input.Current.Value
$probeText = -join [char[]]@(0x539F, 0x751F, 0x4E2D, 0x6587, 0x8F93, 0x5165)
$input.SetValue($probeText)
Start-Sleep -Milliseconds 200
if ($input.Current.Value -ne $probeText) { throw 'Unicode value round trip failed.' }
$echoName = -join [char[]]@(0x56DE, 0x663E, 0x8F93, 0x5165)
$nameCondition = New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::NameProperty), $echoName
$echo = $root.FindFirst([Windows.Automation.TreeScope]::Descendants, $nameCondition)
if ($null -eq $echo) { throw 'Echo button not found.' }
$echo.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
Start-Sleep -Milliseconds 200
if ($input.Current.Value -ne $probeText) { throw 'Unrelated redraw lost the input value.' }
Write-Output 'Unicode value retained after native button invocation.'
$buttonCondition = New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty), ([Windows.Automation.ControlType]::Button)
$buttons = $root.FindAll([Windows.Automation.TreeScope]::Descendants, $buttonCondition)
$hotkeyButton = $null
foreach ($button in $buttons) { if ($button.Current.Name.Contains('Ctrl+Alt+P')) { $hotkeyButton = $button; break } }
if ($null -eq $hotkeyButton) { throw 'Hotkey probe button not found.' }
$hotkeyButton.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
Start-Sleep -Milliseconds 500
$stopName = -join [char[]]@(0x505C,0x6B62,0x5168,0x5C40,0x91C7,0x70B9,0x6D4B,0x8BD5)
$stopCondition = New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::NameProperty), $stopName
$stop = $root.FindFirst([Windows.Automation.TreeScope]::Descendants, $stopCondition)
if ($null -eq $stop) { throw 'Hotkey registration did not remain active; check conflict status.' }
Write-Output 'Hotkey registration reached active UI state.'
$stop.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
Start-Sleep -Milliseconds 200
$input.SetValue($original)
Write-Output 'Hotkey probe stopped; input restored. OS IME composition and physical key capture still require separate checks.'
