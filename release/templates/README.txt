@NAME@ @VERSION@
Mirror's Edge parkour and Skate 3 skating in GTA V Legacy Story Mode.

This is a BETA: things will break. Please report what you find, with the logs (see below).

WHAT IT IS
  One launcher with two cards:
  - Faith Runner (parkour), by Tanner (github.com/tnrjns): Mirror's Edge movement (run, wallrun, wallclimb, vault, ledge grab,
    pull-up, slide, roll, ladders) on GTA's real buildings, cars and props. Faith's first-person
    arms and legs or your GTA character's own, and her sounds, come from your Mirror's Edge:
    Install copies the 16 files it needs (about 65 MB) into GTA V\FaithRunnerV\MirrorsEdge, so
    Mirror's Edge can be uninstalled afterwards.
  - SK8V by becaring (github.com/becaring): Skate 3's controls and physics. It isn't part of this download: the
    launcher fetches becaring's newest release from github.com/becaring/SK8V and runs his own
    setup, right inside the launcher, with your own Skate 3 disc.
  The launcher also checks for updates to both.

REQUIREMENTS
  - Windows 10 or 11 x64 (with the Microsoft Edge WebView2 runtime, which Windows already has),
    GTA V Legacy 1.0.3889.0 (Story Mode, BattlEye off). Not Enhanced, not Online.
  - ScriptHookV v3889.0 with its ASI loader (dinput8.dll): https://www.dev-c.com/gtav/scripthookv/
  - For parkour: Mirror's Edge (2008) for PC (Steam, EA app or disc), installed while you
    install Faith Runner. Keyboard and mouse or a controller.
  - For SK8V: your own Skate 3 Xbox 360 disc image (ISO or extracted folder), a controller,
    an internet connection (207 MB download, plus 31 MB of Python the first time) and about
    7 GB free on the GTA drive during setup (about 2 GB stays).

INSTALL
  1. Unzip this folder anywhere (not inside the GTA folder) and close GTA V.
     Install ScriptHookV if you haven't: from its download's bin folder, copy BOTH ScriptHookV.dll
     and dinput8.dll (the ASI loader) into the GTA V folder. Without dinput8.dll no mods load.
  2. Run Launcher.exe. It finds GTA V and Mirror's Edge by itself when it can; Change picks
     another folder.
  3. Faith Runner card: Install. SK8V card: pick your Skate 3 disc and stance, then Download & install.
  4. Press PLAY.
  Keep the launcher: open it again to get updates. Your parkour settings are kept.

INSTALL WITHOUT THE LAUNCHER WINDOW
  Faith Runner, from a command prompt in this folder (GTA closed):
    Launcher.exe --install-parkour --gta "<GTA V folder>" --mirrors "<Mirror's Edge folder>"
  It does the same as the Install button (copies the mod and Faith's files from Mirror's Edge)
  and prints the result. Leave out --gta or --mirrors to let it find the folder itself.
  By hand instead: copy parkour\FaithRunnerV.asi and parkour\FaithRunnerV.ini into the GTA V
  folder and put your Mirror's Edge folder after "MirrorsEdge =" in the ini. Faith Runner then
  reads Faith's files from Mirror's Edge each time GTA starts, so keep Mirror's Edge installed.
  SK8V: see "Install" in SK8V's README (github.com/becaring/SK8V), which also has a command line.

CONTROLS (Faith Runner)
  F8, or both sticks clicked together       parkour on / off
  Hold F8 (or both sticks) half a second    parkour settings menu (body, sounds, look speed...)
  Move                                      W A S D  /  left stick
  Look                                      mouse  /  right stick
  Jump, wallrun, climb, grab                Space  /  A or LB
  Crouch, slide, roll, let go               Shift, Ctrl or C  /  B or LT
  Quick turn (180)                          Q  /  Y
  SK8V's controls: see its README on github.com/becaring/SK8V.

KNOWN LIMITS (beta)
  - Parkour and skating don't run at the same time: put the board away before pressing F8.
  - First person only for parkour.
  - Water doesn't end parkour mode yet: press F8 before you jump in.
  - Pedestrians have no collision yet: you run through people.
  - Missions are untested: switch parkour off with F8 if one acts up.
  - Only ladders are found in the world so far: no swing poles, balance beams, door barges,
    barbed wire or soft landings yet.
  - A fall from about 20 m ends parkour with GTA's ragdoll (LethalFall in FaithRunnerV.ini).
  - Teleporting while either mode is on can drop you through the map: switch it off first.

LOGS
  Parkour: FaithRunnerV.log in the GTA V folder. SK8V: %LOCALAPPDATA%\SkateV.
  The launcher keeps its downloads and settings in %LOCALAPPDATA%\@NAME@ (safe to delete).

UNINSTALL (GTA closed)
  Faith Runner: the Remove button on its card (its files and the FaithRunnerV folder with Faith's
  files go to the Recycle Bin). By hand: delete FaithRunnerV.asi, FaithRunnerV.ini and the
  FaithRunnerV folder from the GTA V folder.
  SK8V: see "Uninstall" in SK8V's README.
  ScriptHookV is left alone either way.

CREDITS AND LICENCES
  See NOTICE.txt. In short: Faith Runner by Tanner (MIT), SK8V by becaring (Apache-2.0), live GTA
  collision reading by Sol4ra. @NAME@ is an unofficial fan project, not made or endorsed by
  them, and unaffiliated with EA, DICE, Rockstar Games or Take-Two Interactive. It contains no
  Mirror's Edge, Skate 3 or GTA V code or data: everything comes from your own copies.
  Coded with Claude (Anthropic's AI), through Claude Code.
