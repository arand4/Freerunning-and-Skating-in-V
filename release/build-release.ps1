# Builds the release zip: the launcher and the parkour part (Faith Runner V). SK8V is NOT included:
# the launcher downloads becaring's own release from his GitHub and runs his setup tools.
#   powershell -ExecutionPolicy Bypass -File release\build-release.ps1 -Version 0.1.0-beta
# Needs Rust (rustup.rs) and the Visual Studio C++ build tools. The name is in launcher.json,
# README.txt, NOTICE.txt and the folder/zip name, so renaming is a rebuild.
# Output: release\out\<name>-<version>\ and release\out\<name>-<version>.zip (an older one goes to the Recycle Bin).
# Upload the zip to a GitHub release: the launcher installs and updates from the newest one.
param(
  [string]$Name = 'Freerunning and Skating in V',
  [string]$Version = '0.1.0-beta',
  [string]$ParkourRepo = 'arand4/Freerunning-and-Skating-in-V',
  [switch]$NoCompile
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot
$slug = ($Name -replace '[^A-Za-z0-9]+', '-').Trim('-')
$out = Join-Path $PSScriptRoot "out\$slug-$Version"
$zip = "$out.zip"
$exe = Join-Path $repo 'launcher\target\release\Launcher.exe'
$asi = Join-Path $repo 'faithrunner-v\target\release\FaithRunnerV.dll'

if (-not $NoCompile) {
  # Keep this PC's folder names out of the built files.
  $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
  $rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' }
  $env:CARGO_ENCODED_RUSTFLAGS = (@("--remap-path-prefix=$cargoHome=cargo", "--remap-path-prefix=$rustupHome=rustup",
    "--remap-path-prefix=$repo=src", "--remap-path-prefix=$env:USERPROFILE=user") -join [char]0x1f)
  foreach ($crate in 'launcher', 'faithrunner-v') {
    Push-Location (Join-Path $repo $crate)
    try { cargo build --release; if ($LASTEXITCODE) { throw "$crate did not build" } } finally { Pop-Location }
  }
}

Add-Type -AssemblyName Microsoft.VisualBasic, System.IO.Compression.FileSystem
function Recycle($p) {
  if (Test-Path $p -PathType Container) { [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($p, 'OnlyErrorDialogs', 'SendToRecycleBin') }
  elseif (Test-Path $p) { [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($p, 'OnlyErrorDialogs', 'SendToRecycleBin') }
}
foreach ($p in $exe, $asi) { if (-not (Test-Path $p)) { throw "missing: $p (build it first)" } }
Recycle $out; Recycle $zip
New-Item -ItemType Directory -Force $out | Out-Null

# The launcher. The parkour card streams media/parkour.mp4 from $ParkourRepo; the SK8V card shows SK8V's own picture.
Copy-Item $exe "$out\Launcher.exe"
[IO.File]::WriteAllText("$out\launcher.json", (@{ name = $Name; version = $Version; parkourRepo = $ParkourRepo; sk8vRepo = 'becaring/SK8V' } | ConvertTo-Json), [Text.Encoding]::ASCII)
New-Item -ItemType Directory -Force "$out\images" | Out-Null
Copy-Item (Join-Path $repo 'media\sk8v.webp') "$out\images\skate.webp"

# The parkour part (Faith Runner V); it holds SK8V-derived code, so SK8V's licence and notice go with it.
$pk = New-Item -ItemType Directory -Force "$out\parkour"
Copy-Item $asi "$pk\FaithRunnerV.asi"
Copy-Item (Join-Path $repo 'licenses\faith-runner-LICENSE.txt') "$pk\LICENSE-faith-runner.txt"
Copy-Item (Join-Path $repo 'licenses\SK8V-LICENSE.txt') "$pk\LICENSE-SK8V.txt"
Copy-Item (Join-Path $repo 'licenses\SK8V-NOTICE.txt') "$pk\NOTICE-SK8V.txt"
Copy-Item (Join-Path $repo 'faithrunner-v\patches\faith-runner-gta-changes.patch') $pk
$ini = Get-Content (Join-Path $repo 'faithrunner-v\FaithRunnerV.ini') | ForEach-Object {
  if ($_ -match '^; Faith Runner V settings') { "; Faith Runner V settings ($Name $Version). Lines starting with ; are notes. Most can be changed in game: hold F8." }
  elseif ($_ -match '^HeadHide\s*=') { 'HeadHide = shrink' }   # keeps Faith's camera movement
  elseif ($_ -match '^MirrorsEdge\s*=') { 'MirrorsEdge = ' }    # the launcher fills this in
  elseif ($_ -match "^; Your Mirror's Edge folder") { "; Where Faith's body, animations and sounds are read from: the launcher copies the 16 files it needs" }
  elseif ($_ -match '^; nothing from the game is copied') { "; (65 MB) into FaithRunnerV\MirrorsEdge. Empty: that copy, else Mirror's Edge's usual folders. Or your Mirror's Edge folder." }
  elseif ($_ -match '^MenuKey\s*=') { 'MenuKey = 0' }
  elseif ($_ -match '^; 0x84 = F21') { '; For example 0x84 = F21. 0 = none.' }
  else { $_ }
}
[IO.File]::WriteAllLines("$pk\FaithRunnerV.ini", [string[]]$ini)

foreach ($f in 'README.txt', 'NOTICE.txt') {
  $t = [IO.File]::ReadAllText("$PSScriptRoot\templates\$f").Replace('@NAME@', $Name).Replace('@VERSION@', $Version)
  [IO.File]::WriteAllText("$out\$f", $t, [Text.Encoding]::ASCII)
}

$sums = Get-ChildItem $out -Recurse -File | Sort-Object FullName | ForEach-Object {
  '{0}  {1}' -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.FullName.Substring($out.Length + 1).Replace('\', '/')
}
[IO.File]::WriteAllLines("$out\SHA256SUMS.txt", [string[]]$sums)
[IO.Compression.ZipFile]::CreateFromDirectory($out, $zip, 'Optimal', $true)
'{0}  ({1:N1} MB)' -f $zip, ((Get-Item $zip).Length / 1MB)
