@echo off
rem Removes Faith Runner V from GTA V (its files go to the Recycle Bin; ScriptHookV is left alone).
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" -Uninstall %*
