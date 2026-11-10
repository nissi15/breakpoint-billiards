# Breakpoint Billiards installer. No administrator rights needed.
#
# Easiest: paste this one line into PowerShell. It downloads the game and
# adds it to your Start menu and desktop:
#
#     irm https://raw.githubusercontent.com/nissi15/breakpoint-billiards/main/install.ps1 | iex
#
# It also works from a download (run it next to breakpoint-billiards.exe) and
# from a cloned repository (it builds the game, which needs https://rustup.rs).
# To remove the game later: Settings > Apps > Breakpoint Billiards > Uninstall.

$ErrorActionPreference = 'Stop'
$AppName = 'Breakpoint Billiards'
$ExeName = 'breakpoint-billiards.exe'
$Repo = 'nissi15/breakpoint-billiards'
$Root = $PSScriptRoot
$Dest = Join-Path $env:LOCALAPPDATA 'Programs\Breakpoint Billiards'
$Exe = Join-Path $Dest $ExeName

function Step($text) { Write-Host "  > $text" -ForegroundColor Yellow }

Write-Host ''
Write-Host '  BREAKPOINT BILLIARDS' -ForegroundColor White
Write-Host '  --------------------' -ForegroundColor DarkYellow

# Close a running copy so its file can be replaced.
$Running = Get-Process breakpoint-billiards -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe }
if ($Running) {
    $Running | Stop-Process -Force
    $Running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
}
New-Item -ItemType Directory -Force -Path $Dest | Out-Null

# 1. Get the game: beside this script, built from source, or downloaded.
$Beside = if ($Root) { Join-Path $Root $ExeName } else { $null }
if ($Beside -and (Test-Path $Beside)) {
    Step 'Installing the game from this folder'
    Copy-Item -LiteralPath $Beside -Destination $Exe -Force
} elseif ($Root -and (Test-Path (Join-Path $Root 'Cargo.toml')) -and (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Step 'Building the game (the first build takes a few minutes)...'
    Push-Location $Root
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw 'The build failed.' }
    } finally {
        Pop-Location
    }
    Copy-Item -LiteralPath (Join-Path $Root "target\release\$ExeName") -Destination $Exe -Force
} else {
    Step 'Downloading the latest version...'
    $Url = if ($env:BPB_DOWNLOAD_URL) { $env:BPB_DOWNLOAD_URL } else { "https://github.com/$Repo/releases/latest/download/$ExeName" }
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $ProgressPreference = 'SilentlyContinue'
    Invoke-WebRequest -Uri $Url -OutFile $Exe -UseBasicParsing
}
# A file that came out of a downloaded zip carries a "from the internet" mark
# that makes Windows warn before running it. The player chose to install, so
# clear it.
Unblock-File -LiteralPath $Exe -ErrorAction SilentlyContinue

# 2. An uninstaller that lives with the game.
$Uninstall = @'
$ErrorActionPreference = 'SilentlyContinue'
$Dest = Join-Path $env:LOCALAPPDATA 'Programs\Breakpoint Billiards'
$Running = Get-Process breakpoint-billiards | Where-Object { $_.Path -like "$Dest*" }
$Running | Stop-Process -Force
$Running | Wait-Process -Timeout 10
Remove-Item -LiteralPath (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Breakpoint Billiards.lnk') -Force
Remove-Item -LiteralPath (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Breakpoint Billiards.lnk') -Force
Remove-Item -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\BreakpointBilliards' -Recurse -Force
Remove-Item -LiteralPath (Join-Path $env:APPDATA 'BreakpointBilliards') -Recurse -Force
Set-Location $env:TEMP
# Windows can hold the game file for a moment after it closes; retry briefly.
for ($i = 0; $i -lt 20 -and (Test-Path -LiteralPath $Dest); $i++) {
    Remove-Item -LiteralPath $Dest -Recurse -Force
    if (Test-Path -LiteralPath $Dest) { Start-Sleep -Milliseconds 250 }
}
Write-Host 'Breakpoint Billiards has been removed.' -ForegroundColor Green
'@
Set-Content -LiteralPath (Join-Path $Dest 'uninstall.ps1') -Value $Uninstall -Encoding UTF8

# 3. Start menu and desktop shortcuts.
Step 'Adding shortcuts'
$Shell = New-Object -ComObject WScript.Shell
$Places = @((Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'))
if (-not $env:BPB_NO_DESKTOP) { $Places += [Environment]::GetFolderPath('Desktop') }
foreach ($Place in $Places) {
    $Link = $Shell.CreateShortcut((Join-Path $Place "$AppName.lnk"))
    $Link.TargetPath = $Exe
    $Link.WorkingDirectory = $Dest
    $Link.IconLocation = "$Exe,0"
    $Link.Description = 'Realistic 3D pool: 8-ball, 9-ball and online rooms'
    $Link.Save()
}

# 4. List it in Settings > Apps so it can be removed like any other game.
$Version = (Get-Item $Exe).VersionInfo.ProductVersion
if (-not $Version) { $Version = '1.0.0' }
$Key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\BreakpointBilliards'
New-Item -Path $Key -Force | Out-Null
$Values = [ordered]@{
    DisplayName     = $AppName
    DisplayVersion  = $Version
    DisplayIcon     = "$Exe,0"
    Publisher       = 'Breakpoint Billiards'
    InstallLocation = $Dest
    UninstallString = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$Dest\uninstall.ps1`""
    URLInfoAbout    = "https://github.com/$Repo"
}
foreach ($k in $Values.Keys) { Set-ItemProperty -Path $Key -Name $k -Value $Values[$k] }
Set-ItemProperty -Path $Key -Name NoModify -Value 1 -Type DWord
Set-ItemProperty -Path $Key -Name NoRepair -Value 1 -Type DWord
Set-ItemProperty -Path $Key -Name EstimatedSize -Value ([int]((Get-Item $Exe).Length / 1KB)) -Type DWord

Write-Host ''
Write-Host '  Installed. Start it from the Start menu or your desktop.' -ForegroundColor Green
Write-Host ''
