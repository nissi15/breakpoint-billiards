<#
    Breakpoint Billiards installer.

    Run it from a release download (next to breakpoint-billiards.exe) or from
    a cloned repository, in which case it builds the game first (that needs
    Rust from https://rustup.rs). Installs for the current user only, so no
    administrator rights are needed.

        powershell -ExecutionPolicy Bypass -File install.ps1
#>
param(
    [switch]$NoDesktopShortcut
)
$ErrorActionPreference = 'Stop'
$AppName = 'Breakpoint Billiards'
$ExeName = 'breakpoint-billiards.exe'
$Root = $PSScriptRoot
$Dest = Join-Path $env:LOCALAPPDATA 'Programs\Breakpoint Billiards'

function Step($text) { Write-Host "  > $text" -ForegroundColor Yellow }

Write-Host ''
Write-Host '  BREAKPOINT BILLIARDS' -ForegroundColor White
Write-Host '  --------------------' -ForegroundColor DarkYellow

# 1. Find the game: a ready-made exe beside this script, or build it.
$Source = Join-Path $Root $ExeName
if (-not (Test-Path $Source)) {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Host ''
        Write-Host '  Rust is needed to build the game from source.' -ForegroundColor Red
        Write-Host '  Install it from https://rustup.rs and run this again,'
        Write-Host '  or download the ready-made game from the Releases page.'
        exit 1
    }
    Step 'Building the game (the first build takes a few minutes)...'
    Push-Location $Root
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw 'The build failed.' }
    } finally {
        Pop-Location
    }
    $Source = Join-Path $Root "target\release\$ExeName"
}

# 2. Copy it into the user's programs folder.
Step "Installing to $Dest"
$Exe = Join-Path $Dest $ExeName
$Running = Get-Process breakpoint-billiards -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe }
if ($Running) {
    $Running | Stop-Process -Force
    $Running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
}
New-Item -ItemType Directory -Force -Path $Dest | Out-Null
Copy-Item -LiteralPath $Source -Destination $Exe -Force
foreach ($f in 'uninstall.ps1', 'README.md', 'LICENSE') {
    $p = Join-Path $Root $f
    if (Test-Path $p) { Copy-Item -LiteralPath $p -Destination $Dest -Force }
}

# 3. Start menu and desktop shortcuts.
Step 'Adding shortcuts'
$Shell = New-Object -ComObject WScript.Shell
$Places = @(Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs')
if (-not $NoDesktopShortcut) { $Places += [Environment]::GetFolderPath('Desktop') }
foreach ($Place in $Places) {
    $Link = $Shell.CreateShortcut((Join-Path $Place "$AppName.lnk"))
    $Link.TargetPath = $Exe
    $Link.WorkingDirectory = $Dest
    $Link.IconLocation = "$Exe,0"
    $Link.Description = 'Realistic 3D pool: 8-ball, 9-ball and online rooms'
    $Link.Save()
}

# 4. List it in Settings > Apps so it can be removed like any other game.
Step 'Registering with Windows'
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
    URLInfoAbout    = 'https://github.com/nissi15/breakpoint-billiards'
}
foreach ($k in $Values.Keys) { Set-ItemProperty -Path $Key -Name $k -Value $Values[$k] }
Set-ItemProperty -Path $Key -Name NoModify -Value 1 -Type DWord
Set-ItemProperty -Path $Key -Name NoRepair -Value 1 -Type DWord
Set-ItemProperty -Path $Key -Name EstimatedSize -Value ([int]((Get-Item $Exe).Length / 1KB)) -Type DWord

Write-Host ''
Write-Host '  Installed. Start it from the Start menu or your desktop.' -ForegroundColor Green
Write-Host ''
