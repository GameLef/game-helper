param([Parameter(Mandatory=$true)][string]$BundleDir)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=New-Object Text.UTF8Encoding $false
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$bundle=[IO.Path]::GetFullPath($BundleDir)
if(-not $bundle.StartsWith((Join-Path $root 'dist')+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'Bundle must be inside project dist'}
$manifest=Get-Content -LiteralPath (Join-Path $bundle 'checksums.json') -Raw -Encoding UTF8 | ConvertFrom-Json
foreach($entry in $manifest){if((Get-FileHash -LiteralPath (Join-Path $bundle $entry.Path) -Algorithm SHA256).Hash -ne $entry.SHA256){throw ('Checksum mismatch: '+$entry.Path)}}
$destination=Join-Path $root ('.tools\便携 测试-'+[guid]::NewGuid().ToString('N'))
Copy-Item -LiteralPath $bundle -Destination $destination -Recurse
$processes=@()
try {
 foreach($mode in @('manager','picker')) {
  $options=@{FilePath=(Join-Path $destination 'game-helper.exe');WorkingDirectory=(Join-Path $root '.tools\tmp');PassThru=$true}
  if($mode -eq 'picker'){$options.ArgumentList='--picker'}
  $p=Start-Process @options
  $processes+=$p
  for($i=0;$i -lt 50 -and $p.MainWindowHandle -eq [IntPtr]::Zero;$i++){Start-Sleep -Milliseconds 100;$p.Refresh()}
  if($p.HasExited -or $p.MainWindowHandle -eq [IntPtr]::Zero){throw "$mode did not open a native window"}
  $activation=New-Object -ComObject WScript.Shell
  $activation.AppActivate($p.Id)|Out-Null
  Start-Sleep -Milliseconds 200
  $element=[Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
  $condition=New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty),([Windows.Automation.ControlType]::Edit)
  $edits=$null
  for($i=0;$i -lt 40;$i++){$edits=$element.FindAll([Windows.Automation.TreeScope]::Descendants,$condition);if($edits.Count -ge 2){break};Start-Sleep -Milliseconds 100}
  if($edits.Count -lt 2){$nodes=$element.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition);$details=@($nodes|ForEach-Object {$_.Current.Name});throw "$mode controls missing; handle=$($p.MainWindowHandle); nodes=$details"}
  Write-Output ("Observed: $mode opened from Chinese/space portable path with a different working directory.")
 }
} finally {
 foreach($p in $processes){if(-not $p.HasExited){$p.CloseMainWindow()|Out-Null; if(-not $p.WaitForExit(5000)){throw 'Portable preview did not close cleanly'}}}
}
Write-Output 'Bundle SHA-256 manifest matched; manager and picker exited cleanly.'
