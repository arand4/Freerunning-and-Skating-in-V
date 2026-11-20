@NAME@ @VERSION@ - Faith Runner only, no launcher
Mirror's Edge parkour in GTA V Legacy Story Mode.

This is a BETA: things will break. Please report what you find, with the log (see below).

WHAT IT IS
  Faith Runner (parkour), by Tanner (github.com/tnrjns): Mirror's Edge movement (run, wallrun,
  wallclimb, vault, ledge grab, pull-up, slide, roll, ladders) on GTA's real buildings, cars and
  props. Faith's first-person arms and legs or your GTA character's own, and her sounds, come from
  your Mirror's Edge: the installer copies the 16 files it needs (about 65 MB) into
  GTA V\FaithRunnerV\MirrorsEdge, so Mirror's Edge can be uninstalled afterwards. Or from your
  Mirror's Edge Xbox 360 disc image (.iso): it converts what it needs in a few seconds (67 MB).
  This download has no launcher: a small script installs it. It doesn't check for updates and
  doesn't include SK8V (skating). For those, get the full download with the launcher.

REQUIREMENTS
  - Windows 10 or 11 x64, GTA V Legacy 1.0.3889.0 (Story Mode, BattlEye off). Not Enhanced, not Online.
  - ScriptHookV v3889.0 with its ASI loader (dinput8.dll): https://www.dev-c.com/gtav/scripthookv/
  - Mirror's Edge (2008) for PC (Steam, EA app or disc), installed while you install Faith Runner,
    or your own Mirror's Edge Xbox 360 disc image (.iso).
  - Keyboard and mouse or a controller.

INSTALL
  1. Unzip this folder anywhere (not inside the GTA folder) and close GTA V.
     Install ScriptHookV if you haven't: from its download's bin folder, copy BOTH ScriptHookV.dll
     and dinput8.dll (the ASI loader) into the GTA V folder. Without dinput8.dll no mods load.
  2. Double-click "Install Faith Runner.bat". It finds GTA V and Mirror's Edge by itself when it
     can, and asks when it can't: for Mirror's Edge, pick its MirrorsEdge.exe (PC) or your Xbox
     360 disc image (.iso). It checks your game version and ScriptHookV,
     copies the mod and Faith's files, and says when it's done. If the GTA V folder needs
     administrator rights, Windows asks for them.
  3. Start GTA V with -nobattleye (see below). In story mode, tap F8.
  Installing again (a newer version of this download) keeps your FaithRunnerV.ini settings.

  From a command prompt instead (GTA closed):
    "Install Faith Runner.bat" -Gta "<GTA V folder>" -Mirrors "<Mirror's Edge folder, or its .iso>"
  Leave out -Gta or -Mirrors to let it find the folder itself. Add -Quiet to skip the windows and
  the final "Press Enter".

  By hand instead: copy files\FaithRunnerV.asi and files\FaithRunnerV.ini into the GTA V folder
  and put your Mirror's Edge folder after "MirrorsEdge =" in the ini. Faith Runner then reads
  Faith's files from Mirror's Edge each time GTA starts, so keep Mirror's Edge installed.

STARTING GTA
  BattlEye has to be off, or it blocks ScriptHookV and Faith Runner. Make a shortcut:
  1. In your GTA V folder, right-click PlayGTAV.exe > Send to > Desktop (create shortcut).
  2. Right-click the new shortcut > Properties. In Target, after the closing quote, add a space and
     -nobattleye, so it ends like ...\PlayGTAV.exe" -nobattleye. Press OK.
  On Steam, start the game with this shortcut, not Steam's Play button: that installs any waiting
  update first, and a game update stops ScriptHookV and Faith Runner until they get one of their own.

STAYING ON 1.0.3889.0
  Turn updates off in BOTH places: the store you bought it from (Steam or Epic) AND the Rockstar
  Games Launcher, because either one can update the game by itself.
  - Steam: Library > right-click GTA V Legacy > Properties > Updates > Automatic updates: Only
    update this game when I launch it. Then start the game with the shortcut above.
  - Epic Games: Library > GTA V > ... > Manage > turn Auto-update off.
  - Rockstar Games Launcher: Settings > My installed games > Grand Theft Auto V > turn Enable
    automatic updates off.
  - Don't use Steam's "Verify integrity of game files": it brings in the newest version too.
  The full download's README (and its launcher's Guide button) explains how to back up and undo a
  game update.

CONTROLS
  F8, or both sticks clicked together       parkour on / off
  Hold F8 (or both sticks) half a second    parkour settings menu (body, sounds, look speed...)
  Move                                      W A S D  /  left stick
  Look                                      mouse  /  right stick
  Jump, wallrun, climb, grab                Space  /  A or LB
  Crouch, slide, roll, let go               Shift, Ctrl or C  /  B or LT
  Quick turn (180)                          Q  /  Y

KNOWN LIMITS (beta)
  - First person only.
  - Water doesn't end parkour mode yet: press F8 before you jump in.
  - Pedestrians have no collision yet: you run through people.
  - Missions are untested: switch parkour off with F8 if one acts up.
  - Only ladders are found in the world so far: no swing poles, balance beams, door barges,
    barbed wire or soft landings yet.
  - A fall from about 20 m ends parkour with GTA's ragdoll (LethalFall in FaithRunnerV.ini).

LOG
  FaithRunnerV.log in the GTA V folder.

UNINSTALL (GTA closed)
  Double-click "Uninstall Faith Runner.bat": FaithRunnerV.asi, FaithRunnerV.ini, the log and the
  FaithRunnerV folder with Faith's files go to the Recycle Bin. ScriptHookV is left alone.

SUPPORT
  Free, and always will be. A donation is never required but always appreciated:
  https://ko-fi.com/arand4

CREDITS AND LICENCES
  See NOTICE.txt and the licence files in the files folder. In short: Faith Runner by Tanner (MIT),
  parts derived from SK8V by becaring (Apache-2.0), live GTA collision reading by Sol4ra, and
  for Xbox 360 discs SK8V's XMA decoder (LGPL, a port of FFmpeg's) in files\FaithRunnerXbox.exe. @NAME@
  is an unofficial fan project, not made or endorsed by them, and unaffiliated with EA, DICE,
  Rockstar Games or Take-Two Interactive. It contains no Mirror's Edge or GTA V code or data:
  everything comes from your own copies.
  Coded with Claude (Anthropic's AI), through Claude Code.
