param([switch]$IncludeUserData)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$exe=Join-Path $root 'target\release\game-helper.exe'
if(-not (Test-Path -LiteralPath $exe)){throw 'Release executable missing. Run scripts\cargo.cmd build --release --locked first.'}
$stamp=Get-Date -Format 'yyyyMMdd-HHmmss'
$destination=Join-Path $root ('dist\game-helper-native-preview-'+$stamp)
New-Item -ItemType Directory -Path (Join-Path $destination 'routes') -Force | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $destination 'game-helper.exe')
Copy-Item -LiteralPath (Join-Path $root 'ahk') -Destination (Join-Path $destination 'ahk') -Recurse
Copy-Item -LiteralPath (Join-Path $root 'docs\rust-migration.md') -Destination (Join-Path $destination 'MIGRATION.md')
$utf8=New-Object Text.UTF8Encoding $false
if($IncludeUserData) {
 Copy-Item -LiteralPath (Join-Path $root 'config.json') -Destination (Join-Path $destination 'config.json')
 Get-ChildItem -LiteralPath (Join-Path $root 'routes') -File | Where-Object {$_.Extension -in @('.yaml','.yml')} | Copy-Item -Destination (Join-Path $destination 'routes')
} else {
 [IO.File]::WriteAllText((Join-Path $destination 'config.json'),'{"routesDir":"routes","hotkeys":{"stopAll":"F12","bindings":[]},"autoStartDaemon":false,"autoReloadOnSave":true}',$utf8)
}
[IO.File]::WriteAllText((Join-Path $destination 'start.bat'),"@echo off`r`nstart `"`" `"%~dp0game-helper.exe`" --app-root `"%~dp0.`" %*`r`n",$utf8)
[IO.File]::WriteAllText((Join-Path $destination 'picker.bat'),"@echo off`r`nstart `"`" `"%~dp0game-helper.exe`" --app-root `"%~dp0.`" --picker`r`n",$utf8)
[IO.File]::WriteAllText((Join-Path $destination 'README.txt'),"Game Helper native preview`r`nOpen game-helper.exe for the manager, or picker.bat for point capture.`r`nThe client requires no Python, Rust, Tauri or WebView runtime.`r`nWindows must provide the Microsoft Visual C++ x64 runtime (VCRUNTIME140.dll).`r`nRoute playback requires an installed AutoHotkey v2. Set config.json ahkPath if detection fails.`r`nThis is a preview; see MIGRATION.md for tested and pending acceptance items.`r`nNo user routes or bindings are included unless -IncludeUserData is explicitly selected.`r`n",$utf8)
Get-ChildItem -LiteralPath $destination -Recurse -File | ForEach-Object {
 [PSCustomObject]@{Path=$_.FullName.Substring($destination.Length+1);SHA256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash}
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $destination 'checksums.json') -Encoding UTF8
$archive=$destination+'.zip'
Compress-Archive -LiteralPath $destination -DestinationPath $archive
Write-Output ('Bundle: '+$destination)
Write-Output ('Archive: '+$archive)
