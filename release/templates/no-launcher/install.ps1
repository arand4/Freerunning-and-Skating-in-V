# Faith Runner V installer (no launcher): what the launcher's Install button does, as a script.
# Started by "Install Faith Runner.bat". Options:
#   -Gta "<GTA V folder>"  -Mirrors "<Mirror's Edge folder, or its Xbox 360 disc image (.iso)>"  -Uninstall  -Quiet
# (-Quiet: no pause, no folder windows)
param([string]$Gta = '', [string]$Mirrors = '', [switch]$Uninstall, [switch]$Quiet)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$files = Join-Path $here 'files'
$GTA_VERSION = '1.0.3889.0'
$ME_COPY = 'FaithRunnerV\MirrorsEdge'
# What FaithRunnerXbox.exe writes inside it from an Xbox 360 disc image.
$XBOX_DUMP = 'TdGame\CookedXenon\TdGame.xxx'
# Every file Faith Runner V reads from Mirror's Edge's TdGame\CookedPC (about 65 MB).
$ME_FILES = @(
  'Characters\CH_TKY_Crim_Fixer_1P.upk', 'Animations\AS_C1P_Unarmed.upk', 'Animations\AS_C1P_OneHanded_Common.upk',
  'Animations\AS_F3P_Unarmed.upk', 'Characters\CH_TKY_Crim_Fixer.upk', 'Characters\CH_Faith_Cinematic.upk',
  'Audio\A_Bodyfalls.upk', 'Audio\A_Character_Disarm.upk', 'Audio\A_Character_Effects.upk', 'Audio\A_Character_Female_01.upk',
  'Audio\A_Character_Melee.upk', 'Audio\A_Material_Footstep.upk', 'Audio\A_Material_Handstep.upk',
  'Audio\A_Props_Interactive.upk', 'Audio\A_WP_Handling.upk', 'Audio\int\A_VO_AI_CUE.upk')

function Say($t) { Write-Host $t }
function Done($ok, $t) {
  if ($ok) { Write-Host ''; Write-Host $t -ForegroundColor Green } else { Write-Host ''; Write-Host $t -ForegroundColor Red }
  if (-not $Quiet) { Write-Host ''; Read-Host 'Press Enter to close' | Out-Null }
  exit ([int](-not $ok))
}
function Reg($path, $name) {
  try { (Get-ItemProperty -Path $path -Name $name -ErrorAction Stop).$name } catch { $null }
}
function SteamLibraries {
  $steam = Reg 'HKCU:\Software\Valve\Steam' 'SteamPath'
  if (-not $steam) { return @() }
  $steam = $steam.Replace('/', '\')
  $libs = @($steam)
  $vdf = Join-Path $steam 'steamapps\libraryfolders.vdf'
  if (Test-Path $vdf) {
    foreach ($l in Get-Content $vdf) {
      if ($l.Trim() -match '^"path"\s+"(.+)"$') { $p = $Matches[1].Replace('\\', '\'); if ($libs -notcontains $p) { $libs += $p } }
    }
  }
  $libs
}
function MeCooked($dir) {
  if (-not $dir) { return $null }
  foreach ($c in (Join-Path $dir 'TdGame\CookedPC'), (Join-Path $dir 'CookedPC'), $dir) {
    if ((Test-Path (Join-Path $c 'Characters\CH_TKY_Crim_Fixer_1P.upk')) -and (Test-Path (Join-Path $c 'Animations\AS_C1P_Unarmed.upk'))) { return $c }
  }
  $null
}
function FindGta {
  foreach ($k in @(('HKLM:\SOFTWARE\WOW6432Node\Rockstar Games\Grand Theft Auto V', 'InstallFolder'),
                   ('HKLM:\SOFTWARE\WOW6432Node\Rockstar Games\GTAV', 'InstallFolderSteam'),
                   ('HKLM:\SOFTWARE\WOW6432Node\Rockstar Games\GTAV', 'InstallFolder'),
                   ('HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 271590', 'InstallLocation'))) {
    $p = Reg $k[0] $k[1]
    if ($p -and (Test-Path (Join-Path $p 'GTA5.exe'))) { return $p.TrimEnd('\') }
  }
  foreach ($l in SteamLibraries) {
    $d = Join-Path $l 'steamapps\common\Grand Theft Auto V'
    if (Test-Path (Join-Path $d 'GTA5.exe')) { return $d }
  }
  ''
}
function FindMirrors {
  foreach ($k in @(("HKLM:\SOFTWARE\WOW6432Node\EA Games\Mirror's Edge", 'Install Dir'),
                   ("HKLM:\SOFTWARE\EA Games\Mirror's Edge", 'Install Dir'),
                   ('HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 17410', 'InstallLocation'))) {
    $p = Reg $k[0] $k[1]
    if ($p -and (MeCooked $p)) { return $p.TrimEnd('\') }
  }
  foreach ($l in SteamLibraries) {
    $d = Join-Path $l 'steamapps\common\mirrors edge'
    if (MeCooked $d) { return $d }
  }
  ''
}
function IsIso($p) { $p -and (Test-Path $p -PathType Leaf) -and ([IO.Path]::GetExtension($p) -ieq '.iso') }
# Mirror's Edge from a file picker: its MirrorsEdge.exe (the PC game) or an Xbox 360 disc image.
function PickMirrors {
  if ($Quiet) { return '' }
  Add-Type -AssemblyName System.Windows.Forms
  $f = New-Object System.Windows.Forms.OpenFileDialog
  $f.Title = "Mirror's Edge: pick MirrorsEdge.exe (PC game, in its Binaries folder) or your Xbox 360 disc image (.iso)"
  $f.Filter = "Mirror's Edge (MirrorsEdge.exe, *.iso)|MirrorsEdge.exe;*.iso|All files|*.*"
  if ($f.ShowDialog() -ne 'OK') { return '' }
  $p = $f.FileName
  if (IsIso $p) { return $p }
  # MirrorsEdge.exe is in <game>\Binaries: the game folder is above it.
  $d = Split-Path $p
  if ((Split-Path $d -Leaf) -ieq 'Binaries') { $d = Split-Path $d }
  $d
}
function PickFolder($text) {
  if ($Quiet) { return '' }
  Add-Type -AssemblyName System.Windows.Forms
  $f = New-Object System.Windows.Forms.FolderBrowserDialog
  $f.Description = $text
  $f.ShowNewFolderButton = $false
  if ($f.ShowDialog() -eq 'OK') { $f.SelectedPath } else { '' }
}
# ASI loaders: DLLs in the GTA folder that contain "*.asi" (plain or UTF-16). Returns @(64-bit names, 32-bit names).
function AsiLoaders($dir) {
  $x64 = @(); $x86 = @()
  $plain = [Text.Encoding]::ASCII.GetBytes('*.asi')
  $wide = [Text.Encoding]::Unicode.GetBytes('*.asi')
  foreach ($f in Get-ChildItem $dir -Filter *.dll -File) {
    if ($f.Length -gt 8MB) { continue }
    $b = [IO.File]::ReadAllBytes($f.FullName)
    $low = [Text.Encoding]::GetEncoding(28591).GetString($b).ToLowerInvariant()
    if ($low.Contains('*.asi') -or $low.Contains([Text.Encoding]::GetEncoding(28591).GetString($wide))) {
      $pe = [BitConverter]::ToInt32($b, 0x3C)
      if ($pe -gt 0 -and $pe + 6 -le $b.Length -and [BitConverter]::ToUInt16($b, $pe + 4) -eq 0x8664) { $x64 += $f.Name } else { $x86 += $f.Name }
    }
  }
  @(, $x64) + @(, $x86)
}
function Writable($dir) {
  $t = Join-Path $dir ('frv-write-test-' + [guid]::NewGuid().ToString('N') + '.tmp')
  try { [IO.File]::WriteAllText($t, 'x'); Remove-Item $t -Force; $true } catch { $false }
}
function Recycle($p) {
  Add-Type -AssemblyName Microsoft.VisualBasic
  if (Test-Path $p -PathType Container) { [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($p, 'OnlyErrorDialogs', 'SendToRecycleBin') }
  elseif (Test-Path $p) { [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($p, 'OnlyErrorDialogs', 'SendToRecycleBin') }
}

# --- GTA V folder
if (-not $Gta) { $Gta = FindGta }
if (-not $Gta -or -not (Test-Path (Join-Path $Gta 'GTA5.exe'))) {
  Say "Couldn't find GTA V by itself. Pick your GTA V folder (the one with GTA5.exe)."
  $Gta = PickFolder 'Pick your GTA V folder (the one with GTA5.exe)'
}
if (-not $Gta -or -not (Test-Path (Join-Path $Gta 'GTA5.exe'))) { Done $false "No GTA V folder picked (it must hold GTA5.exe). Nothing was changed." }
$Gta = $Gta.TrimEnd('\')
Say "GTA V:          $Gta"
if (Get-Process -Name GTA5 -ErrorAction SilentlyContinue) { Done $false 'GTA V is running. Close it, then run this again.' }

if (-not (Writable $Gta)) {
  if ($Quiet) { Done $false "Windows won't let this write into the GTA V folder. Run it as administrator." }
  Say 'The GTA V folder needs administrator rights to change: asking Windows for them...'
  $a = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-Gta', "`"$Gta`"")
  if ($Mirrors) { $a += @('-Mirrors', "`"$Mirrors`"") }
  if ($Uninstall) { $a += '-Uninstall' }
  try { Start-Process powershell.exe -Verb RunAs -ArgumentList $a | Out-Null; exit 0 } catch { Done $false 'Administrator rights were refused. Nothing was changed.' }
}

# --- Uninstall
if ($Uninstall) {
  foreach ($f in 'FaithRunnerV.asi', 'FaithRunnerV.ini', 'FaithRunnerV.log', 'FaithRunnerV') {
    $p = Join-Path $Gta $f
    if (Test-Path $p) { Recycle $p; Say "  to the Recycle Bin: $f" }
  }
  Done $true 'Faith Runner removed (its files are in the Recycle Bin). ScriptHookV was left alone.'
}

# --- Checks (the same as the launcher's)
$ver = (Get-Item (Join-Path $Gta 'GTA5.exe')).VersionInfo
$v = '{0}.{1}.{2}.{3}' -f $ver.FileMajorPart, $ver.FileMinorPart, $ver.FileBuildPart, $ver.FilePrivatePart
Say "GTA5.exe:       version $v"
if ($v -ne $GTA_VERSION) { Done $false "This GTA5.exe is version $v. Faith Runner needs GTA V Legacy $GTA_VERSION (not Enhanced). Nothing was changed." }
if (-not (Test-Path (Join-Path $Gta 'ScriptHookV.dll'))) { Done $false "ScriptHookV isn't installed in the GTA V folder (ScriptHookV.dll and dinput8.dll). Install it first: dev-c.com/gtav/scripthookv. Nothing was changed." }
$loaders = AsiLoaders $Gta
if ($loaders[0].Count -eq 0 -and $loaders[1].Count -gt 0) { Done $false ("{0} in the GTA V folder is an ASI loader for 32-bit games (like GTA IV), so GTA V skips it and loads no mods. Replace it with ScriptHookV's own dinput8.dll, from its download's bin folder." -f ($loaders[1] -join ', ')) }
if ($loaders[0].Count -eq 0) { Done $false "There's no ASI loader in the GTA V folder, so GTA would load no mods. ScriptHookV's is dinput8.dll, in its download's bin folder next to ScriptHookV.dll." }
Say ("ASI loader:     {0}" -f ($loaders[0] -join ', '))

# --- Mirror's Edge (not needed again once its files were copied)
$copied = (MeCooked (Join-Path $Gta $ME_COPY)) -or (Test-Path (Join-Path $Gta "$ME_COPY\$XBOX_DUMP"))
if (-not $Mirrors) { $Mirrors = FindMirrors }
if (-not (MeCooked $Mirrors) -and -not (IsIso $Mirrors) -and -not $copied) {
  Say "Couldn't find Mirror's Edge by itself. Pick its MirrorsEdge.exe (PC), or your Mirror's Edge Xbox 360 disc image (.iso)."
  $Mirrors = PickMirrors
}
$iso = IsIso $Mirrors
$src = if ($iso) { $null } else { MeCooked $Mirrors }
if (-not $src -and -not $iso -and -not $copied) { Done $false "No Mirror's Edge with Faith's files in it was picked. Nothing was changed." }
if ($iso) { Say "Mirror's Edge:  Xbox 360 disc image $Mirrors" } elseif ($src) { Say "Mirror's Edge:  $Mirrors" } else { Say "Mirror's Edge:  not found, using the copy made before" }

# --- Install
if (-not (Test-Path (Join-Path $files 'FaithRunnerV.asi'))) { Done $false "files\FaithRunnerV.asi is missing from this download. Unzip the whole download again." }
Say ''
Say 'Copying FaithRunnerV.asi'
Copy-Item (Join-Path $files 'FaithRunnerV.asi') (Join-Path $Gta 'FaithRunnerV.asi') -Force
if ($iso) {
  # FaithRunnerXbox.exe converts Faith's body, animations, textures and sounds from the disc (a few seconds).
  $tool = Join-Path $files 'FaithRunnerXbox.exe'
  if (-not (Test-Path $tool)) { Done $false "files\FaithRunnerXbox.exe is missing from this download. Unzip the whole download again." }
  Say "Reading Faith's files from the Xbox 360 disc"
  & $tool $Mirrors (Join-Path $Gta $ME_COPY)
  if ($LASTEXITCODE -ne 0) { Done $false "Reading the Mirror's Edge disc failed (see above). Faith Runner was copied, but has no Faith files yet." }
}
if ($src) {
  $dst = Join-Path $Gta "$ME_COPY\TdGame\CookedPC"
  $n = 0
  foreach ($f in $ME_FILES) {
    $from = Join-Path $src $f
    if (-not (Test-Path $from)) { continue }
    $to = Join-Path $dst $f
    if (-not (Test-Path $to) -or (Get-Item $to).Length -ne (Get-Item $from).Length) {
      Say "Copying Mirror's Edge file  $f"
      New-Item -ItemType Directory -Force (Split-Path $to) | Out-Null
      Copy-Item $from $to -Force
      $n++
    }
  }
  if (-not (MeCooked (Join-Path $Gta $ME_COPY))) { Done $false "Copying Faith's files from Mirror's Edge failed." }
  if ($n -eq 0) { Say "Mirror's Edge files: already there" }
}
# An ini that's already there keeps the player's settings: only the Mirror's Edge line changes.
$ini = Join-Path $Gta 'FaithRunnerV.ini'
$from = if (Test-Path $ini) { $ini } else { Join-Path $files 'FaithRunnerV.ini' }
if (Test-Path $ini) { Say 'Keeping your FaithRunnerV.ini settings' } else { Say 'Copying FaithRunnerV.ini' }
$set = "MirrorsEdge = $ME_COPY"
$found = $false
$lines = foreach ($l in [IO.File]::ReadAllLines($from)) {
  if ($l.TrimStart().StartsWith('MirrorsEdge') -and $l.Contains('=')) { $found = $true; $set } else { $l }
}
if (-not $found) { $lines = @($lines) + $set }
[IO.File]::WriteAllText($ini, (($lines -join "`r`n") + "`r`n"), (New-Object Text.UTF8Encoding $false))

Done $true 'Faith Runner is installed. In story mode, tap F8 (or click both sticks) to switch it on, and hold it for half a second to open its settings menu. Mirror''s Edge can be uninstalled now if you like.'
