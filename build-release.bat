@echo off
setlocal
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
set "VERSION=0.2.36"
set "PACKAGE=dist\Lilt-v%VERSION%-windows-x64"
set "ARCHIVE=dist\Lilt-v%VERSION%-windows-x64.zip"

cargo build --release
if errorlevel 1 exit /b %errorlevel%

if not exist "dist" mkdir "dist"
if not exist "%PACKAGE%" mkdir "%PACKAGE%"
copy /y "target\release\lilt-player.exe" "%PACKAGE%\lilt-player.exe" >nul
if errorlevel 1 exit /b %errorlevel%
copy /y "assets\app_icon.png" "%PACKAGE%\app_icon.png" >nul
if errorlevel 1 exit /b %errorlevel%
copy /y "assets\app_icon.ico" "%PACKAGE%\app_icon.ico" >nul
if errorlevel 1 exit /b %errorlevel%
copy /y "README.md" "%PACKAGE%\README.md" >nul
if errorlevel 1 exit /b %errorlevel%

powershell -NoProfile -ExecutionPolicy Bypass -Command "Compress-Archive -Path '%PACKAGE%\*' -DestinationPath '%ARCHIVE%' -Force"
if errorlevel 1 exit /b %errorlevel%

if not exist ".tools\wix.exe" dotnet tool install --tool-path .tools wix --version 7.0.1
if errorlevel 1 exit /b %errorlevel%
.tools\wix.exe extension list -g | findstr /b /c:"WixToolset.UI.wixext 7.0.1" >nul
if errorlevel 1 .tools\wix.exe extension add -g WixToolset.UI.wixext/7.0.1
if errorlevel 1 exit /b %errorlevel%
.tools\wix.exe build installer\Package.wxs -loc installer\Lilt.en-us.wxl -culture en-US -ext WixToolset.UI.wixext/7.0.1 -o "dist\Lilt-v%VERSION%-windows-x64.msi"
if errorlevel 1 exit /b %errorlevel%
del /q "dist\Lilt-v%VERSION%-windows-x64.wixpdb" 2>nul

echo.
echo Release executable: target\release\lilt-player.exe
echo Portable package:   %ARCHIVE%
echo MSI installer:      dist\Lilt-v%VERSION%-windows-x64.msi
