$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$appRoot=Join-Path $root '.tools\acceptance sandbox'
$p=Start-Process -FilePath (Join-Path $root 'target\debug\game-helper.exe') -ArgumentList @('--app-root',('"'+$appRoot+'"')) -PassThru
try {
 for($i=0;$i -lt 50 -and $p.MainWindowHandle -eq [IntPtr]::Zero;$i++){Start-Sleep -Milliseconds 100;$p.Refresh()}
 & (Join-Path $PSScriptRoot 'smoke-manager.ps1') -TargetProcessId $p.Id
} finally {
 if(-not $p.HasExited){$p.CloseMainWindow()|Out-Null}
}
