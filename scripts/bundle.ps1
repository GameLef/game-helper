param([switch]$IncludeUserData)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$exe=Join-Path $root 'target\release\game-helper.exe'
if(-not (Test-Path -LiteralPath $exe -PathType Leaf)){throw 'Release executable missing. Run scripts\cargo.cmd build --release --locked first.'}

# Verify all vendored resources before creating any release directory. Pins are
# independent of metadata, so changing a resource and its metadata cannot pass.
$runtime=Join-Path $root 'ahk\runtime'
$metadataPath=Join-Path $runtime 'metadata.json'
if(-not (Test-Path -LiteralPath $metadataPath -PathType Leaf)){throw 'Bundled AHK metadata.json is missing.'}
$metadata=Get-Content -LiteralPath $metadataPath -Raw -Encoding UTF8 | ConvertFrom-Json
$archiveUrl='https://github.com/AutoHotkey/AutoHotkey/releases/download/v2.0.31/AutoHotkey_2.0.31.zip'
$archiveHash='CA1A595ACA4528755D8D4CD7FED23D497A37BD9A0B6892072612D68DEE1BD00A'
$sourceCommit='9cebeeb1823a07ca03983ade5164a4f533753519'
$sourceUrl='https://codeload.github.com/AutoHotkey/AutoHotkey/zip/'+$sourceCommit
$expected=@{
 'AutoHotkey64.exe'='5073EE85DA091653C7C6CE32CACDA3A0B4BFB51649823C10015BA0BD71ACB73C'
 'license.txt'='B27C1A7C92686E47F8740850AD24877A50BE23FD3DBD44EDEE50AC1223135E38'
 'source.zip'='C917929A8E19681A166B518603DCF846A3C05800DBBBE7292C1C343A520A2303'
}
if($metadata.schemaVersion -ne 1 -or $metadata.name -cne 'AutoHotkey' -or $metadata.version -cne '2.0.31' -or $metadata.architecture -cne 'x64' -or $metadata.variant -cne 'standard' -or $metadata.hashAlgorithm -cne 'SHA256') {
 throw 'Bundled AHK metadata identity/version/architecture is invalid.'
}
if($metadata.binaryArchive.url -cne $archiveUrl -or $metadata.binaryArchive.sha256 -ne $archiveHash -or $metadata.source.url -cne $sourceUrl -or $metadata.source.commit -cne $sourceCommit -or $metadata.source.sha256 -ne $expected['source.zip'] -or $metadata.license.path -cne 'license.txt') {
 throw 'Bundled AHK provenance does not match the pinned official release.'
}
if(@($metadata.files).Count -ne $expected.Count){throw 'Bundled AHK metadata file list is incomplete.'}
foreach($name in $expected.Keys) {
 $resource=Join-Path $runtime $name
 if(-not (Test-Path -LiteralPath $resource -PathType Leaf)){throw ('Bundled AHK resource missing: '+$name)}
 $entry=@($metadata.files | Where-Object {$_.path -ceq $name})
 if($entry.Count -ne 1 -or $entry[0].sha256 -ne $expected[$name]){throw ('Bundled AHK metadata hash mismatch: '+$name)}
 if((Get-FileHash -LiteralPath $resource -Algorithm SHA256).Hash -ne $expected[$name]){throw ('Bundled AHK resource hash mismatch: '+$name)}
}

# Prepare a portable copy without modifying the user's source configuration.
$utf8=New-Object Text.UTF8Encoding $false
if($IncludeUserData) {
 $portableConfig=Get-Content -LiteralPath (Join-Path $root 'config.json') -Raw -Encoding UTF8 | ConvertFrom-Json
 if($null -eq $portableConfig -or $portableConfig -isnot [PSCustomObject]){throw 'config.json must contain a JSON object.'}
 $portableConfig.PSObject.Properties.Remove('ahkPath')
 $configText=ConvertTo-Json -InputObject $portableConfig -Depth 100
} else {
 $configText='{"routesDir":"routes","hotkeys":{"stopAll":"F12","bindings":[]},"autoStartDaemon":true,"autoReloadOnSave":true}'
}
$stamp=Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$destination=Join-Path $root ('dist\game-helper-native-preview-'+$stamp)
New-Item -ItemType Directory -Path (Join-Path $destination 'routes') -Force | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $destination 'game-helper.exe')
Copy-Item -LiteralPath (Join-Path $root 'ahk') -Destination (Join-Path $destination 'ahk') -Recurse
Copy-Item -LiteralPath (Join-Path $root 'docs\rust-migration.md') -Destination (Join-Path $destination 'MIGRATION.md')
[IO.File]::WriteAllText((Join-Path $destination 'config.json'),$configText,$utf8)
if($IncludeUserData) {
 Get-ChildItem -LiteralPath (Join-Path $root 'routes') -File | Where-Object {$_.Extension -in @('.yaml','.yml')} | Copy-Item -Destination (Join-Path $destination 'routes')
}
[IO.File]::WriteAllText((Join-Path $destination 'start.bat'),"@echo off`r`nstart `"`" `"%~dp0game-helper.exe`" --app-root `"%~dp0.`" %*`r`n",$utf8)
[IO.File]::WriteAllText((Join-Path $destination 'picker.bat'),"@echo off`r`nstart `"`" `"%~dp0game-helper.exe`" --app-root `"%~dp0.`" --picker`r`n",$utf8)
[IO.File]::WriteAllText((Join-Path $destination 'README.txt'),"Game Helper native preview`r`nOpen game-helper.exe for the manager, or picker.bat for point capture.`r`nKeep this complete portable directory together; the executable requires its config and ahk resources.`r`nThe client requires no Python, Rust, Tauri or WebView runtime.`r`nWindows must provide the Microsoft Visual C++ x64 runtime (VCRUNTIME140.dll).`r`nAutoHotkey 2.0.31 x64 is included in ahk/runtime; no AutoHotkey installation or network access is required.`r`nPlayback uses the AHK service only. New packages start the service with the manager; included user data preserves the user's auto-start setting.`r`nThe bundled interpreter has priority; config.json ahkPath and system discovery are compatibility fallbacks when it is absent.`r`nAHK's complete license, corresponding source.zip, and pinned provenance/hashes are included in ahk/runtime.`r`nSave imported or bundled YAML routes in the manager before using hotkeys to create their JSON caches.`r`nThis is a preview; see MIGRATION.md for tested and pending acceptance items.`r`nNo user routes or bindings are included unless -IncludeUserData is explicitly selected; machine-specific ahkPath is omitted from that copy.`r`n",$utf8)
Get-ChildItem -LiteralPath $destination -Recurse -File | ForEach-Object {
 [PSCustomObject]@{Path=$_.FullName.Substring($destination.Length+1);SHA256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash}
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $destination 'checksums.json') -Encoding UTF8
$archive=$destination+'.zip'
Compress-Archive -LiteralPath $destination -DestinationPath $archive
Write-Output ('Bundle: '+$destination)
Write-Output ('Archive: '+$archive)
