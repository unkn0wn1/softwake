@echo off
REM Portable Softwake: starts softwake-ui from this folder.
REM softwake-ui spawns softwaked serve when the daemon is not already listening.
cd /d "%~dp0"
start "" "%~dp0softwake-ui.exe"
