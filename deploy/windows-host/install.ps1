# Runs ON the deploy host (Windows PowerShell 5.1), from C:\aa-web after deploy.sh copied the files.
# Registers aa-web as a startup task, opens the port, (re)starts it. Safe to run again.
param(
    [int]$Port = 8080,
    [string]$Pak = 'C:\aa-web\game.aapk',
    [string]$Login = '127.0.0.1:1237',
    [string]$Game = '127.0.0.1:1239'
)
$ErrorActionPreference = 'Stop'
$dir = 'C:\aa-web'
$exe = Join-Path $dir 'aa-web.exe'
$cmdArgs = "--listen 0.0.0.0:$Port --pak `"$Pak`" --web `"$dir\web`" --net login=$Login --net game=$Game"

# Rotate the new exe in (deploy.sh uploads aa-web.new.exe so a running server is not in the way).
$task = 'aa-web'
if (Get-ScheduledTask -TaskName $task -ErrorAction SilentlyContinue) {
    Stop-ScheduledTask -TaskName $task -ErrorAction SilentlyContinue
}
Get-Process aa-web -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | Stop-Process -Force
if (Test-Path "$dir\aa-web.new.exe") { Move-Item -Force "$dir\aa-web.new.exe" $exe }
if (Test-Path "$dir\game.new.aapk") { Move-Item -Force "$dir\game.new.aapk" $Pak }

$action = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument "/c `"`"$exe`" $cmdArgs >> `"$dir\aa-web.log`" 2>&1`"" -WorkingDirectory $dir
$trigger = New-ScheduledTaskTrigger -AtStartup
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) -StartWhenAvailable
$principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
Register-ScheduledTask -TaskName $task -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Force | Out-Null

if (-not (Get-NetFirewallRule -DisplayName "aa-web $Port" -ErrorAction SilentlyContinue)) {
    New-NetFirewallRule -DisplayName "aa-web $Port" -Direction Inbound -Protocol TCP -LocalPort $Port -Action Allow | Out-Null
}
Start-ScheduledTask -TaskName $task
Start-Sleep -Seconds 3
try {
    $m = Invoke-RestMethod "http://127.0.0.1:$Port/pak/manifest.json"
    "aa-web up on port ${Port}: pak $($m.version), $($m.files) files"
} catch {
    "aa-web did not answer on port ${Port}; see $dir\aa-web.log"
}
