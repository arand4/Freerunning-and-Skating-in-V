@echo off
rem Installs Faith Runner V into GTA V (finds GTA V and Mirror's Edge by itself, or asks).
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %*
