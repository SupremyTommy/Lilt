@echo off
setlocal
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cargo build --release
if errorlevel 1 exit /b %errorlevel%
echo.
echo Standalone executable: target\release\lilt-player.exe
