<#
    Removes Breakpoint Billiards installed by install.ps1.
    Saved settings are kept unless you pass -RemoveSettings.
#>
param(
    [switch]$RemoveSettings
)
$ErrorActionPreference = 'SilentlyContinue'
$AppName = 'Breakpoint Billiards'
$Dest = Join-Path $env:LOCALAPPDATA 'Programs\Breakpoint Billiards'

$Running = Get-Process breakpoint-billiards | Where-Object { $_.Path -like "$Dest*" }
$Running | Stop-Process -Force
$Running | Wait-Process -Timeout 10
Remove-Item -LiteralPath (Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\$AppName.lnk") -Force
Remove-Item -LiteralPath (Join-Path ([Environment]::GetFolderPath('Desktop')) "$AppName.lnk") -Force
Remove-Item -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\BreakpointBilliards' -Recurse -Force
# Windows can hold the exe for a moment after the game closes; retry briefly.
for ($i = 0; $i -lt 20 -and (Test-Path -LiteralPath $Dest); $i++) {
    Remove-Item -LiteralPath $Dest -Recurse -Force
    if (Test-Path -LiteralPath $Dest) { Start-Sleep -Milliseconds 250 }
}
if ($RemoveSettings) {
    Remove-Item -LiteralPath (Join-Path $env:APPDATA 'BreakpointBilliards') -Recurse -Force
}
Write-Host "$AppName has been removed." -ForegroundColor Green
