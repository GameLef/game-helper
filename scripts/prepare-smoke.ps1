$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$fixture=Join-Path $root '.tools\acceptance sandbox'
New-Item -ItemType Directory -Path (Join-Path $fixture 'routes'),(Join-Path $fixture 'ahk') -Force | Out-Null
$utf8=New-Object Text.UTF8Encoding $false
[IO.File]::WriteAllText((Join-Path $fixture 'config.json'),'{"routesDir":"routes","hotkeys":{"stopAll":"F12","bindings":[]},"autoStartDaemon":false,"autoReloadOnSave":false}',$utf8)
$source=@'
# Isolated fixture. Playback is never requested by UI smoke scripts.
format: route/v1
name: Smoke
note: isolated GUI acceptance
target:
  process: notepad.exe
points:
  - id: 42
    name: first
    pos: [0.25, 0.5]
    operations:
      - type: wait
        ms: 1
  - id: 7
    name: second
    pos: [0.75, 0.5]
    operations: []
'@
[IO.File]::WriteAllText((Join-Path $fixture 'routes\Smoke.yaml'),$source,$utf8)
Copy-Item -Path (Join-Path $root 'ahk\*') -Destination (Join-Path $fixture 'ahk') -Recurse -Force
Write-Output ('Prepared isolated smoke fixture: '+$fixture)
