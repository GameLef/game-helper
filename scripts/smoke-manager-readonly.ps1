param([Parameter(Mandatory=$true)][int]$TargetProcessId)
# Attach to an already running isolated acceptance manager. No windows are launched.
# This script restores the search field and any draft panel it opens.
# It never creates a draft, changes route values, or requests playback.
# Keep UTF-8 with BOM for Windows PowerShell 5.1.
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=New-Object Text.UTF8Encoding $false
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$p=Get-Process -Id $TargetProcessId
if($p.MainWindowHandle -eq [IntPtr]::Zero){throw 'Target process has no visible main window'}
$root=[Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
$scope=[Windows.Automation.TreeScope]::Descendants
function Nodes($type) {
    $condition=New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty),$type
    $root.FindAll($scope,$condition)
}
function Button($name) {
    @(Nodes ([Windows.Automation.ControlType]::Button) | Where-Object {$_.Current.Name -eq $name}) | Select-Object -First 1
}
function Has-Text($name) {
    $condition=New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::NameProperty),$name
    $null -ne $root.FindFirst($scope,$condition)
}
function Click($name) {
    $button=Button $name
    if($null -eq $button){throw "Button not found: $name"}
    $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 150
}
function Smoke-Routes {
    @(Nodes ([Windows.Automation.ControlType]::Button) | Where-Object {$_.Current.Name.StartsWith('Smoke ')})
}
function Wait-Until($predicate,$description) {
    for($wait=0;$wait -lt 50;$wait++) {
        if(& $predicate){return}
        Start-Sleep -Milliseconds 100
    }
    throw $description
}
function Set-Search($text) {
    # Find a fresh pattern because filtering can rebuild the native subtree.
    $searchEdit=@(Nodes ([Windows.Automation.ControlType]::Edit) | Where-Object {$_.Current.Name -eq '搜索名称或文件名'}) | Select-Object -First 1
    if($null -eq $searchEdit){throw 'Route search input not found'}
    $searchEdit.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($text)
    Start-Sleep -Milliseconds 150
}
$search=@(Nodes ([Windows.Automation.ControlType]::Edit) | Where-Object {$_.Current.Name -eq '搜索名称或文件名'}) | Select-Object -First 1
if($null -eq $search){throw 'Route search input not found'}
$original=$search.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value
try {
    Set-Search ''
    Wait-Until {@(Smoke-Routes).Count -gt 0} 'Isolated Smoke route not found'
    $routeName=(@(Smoke-Routes) | Select-Object -First 1).Current.Name.Substring('Smoke '.Length)
    Set-Search 'Smoke'
    Wait-Until {@(Smoke-Routes).Count -gt 0} 'Search did not match the route filename stem'
    if(-not [string]::IsNullOrWhiteSpace($routeName)) {
        Set-Search $routeName
        Wait-Until {@(Smoke-Routes).Count -gt 0} 'Search did not match the route display name'
    }
    $missing='smoke-no-match-'+[Guid]::NewGuid().ToString('N')
    Set-Search $missing
    Wait-Until {@(Smoke-Routes).Count -eq 0 -and (Has-Text '没有匹配的路线')} 'Nonmatching search did not show the empty search result'
    Set-Search ''
    Wait-Until {@(Smoke-Routes).Count -gt 0 -and -not (Has-Text '没有匹配的路线')} 'Clearing search did not restore the Smoke route'
    Write-Output 'Observed: route search matches filename and display name, shows the no-results message for a nonexistent query, and restores Smoke after clearing.'
} finally {
    Set-Search $original
}
$openedDraftPanel=$false
try {
    if($null -eq (Button '创建草稿')) {
        Click '新建路线'
        $openedDraftPanel=$true
    }
    Wait-Until {$null -ne (Button '创建草稿')} 'New route panel did not expose the create-draft button'
    Write-Output 'Observed: new route panel exposes the create-draft button without submitting a draft.'
} finally {
    if($openedDraftPanel) {
        Click '新建路线'
        Wait-Until {$null -eq (Button '创建草稿')} 'New route panel did not collapse'
    }
}
Write-Output 'Search and draft panel restored. No route values, files, playback, or window lifecycle actions were changed.'
