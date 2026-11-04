@echo off
rem Build the Windows desktop shell (Tauri) into an installer under dist\.
rem Needs the Rust MSVC toolchain and Node; NSIS is fetched by the Tauri CLI on first use.
setlocal
if "%CARGO_TARGET_DIR%"=="" set CARGO_TARGET_DIR=C:\dev\targets\treasury-desktop
cd /d "%~dp0\..\web"
if not exist node_modules call "%~dp0web-setup.cmd"
if not exist src-tauri\icons\icon.ico call npx tauri icon src-tauri\icons\source.png
call npx tauri build
if errorlevel 1 exit /b 1
if not exist "%~dp0..\dist" mkdir "%~dp0..\dist"
for %%f in ("%CARGO_TARGET_DIR%\release\bundle\nsis\*-setup.exe") do copy /y "%%f" "%~dp0..\dist\NBB-Treasury-Setup.exe" >nul
copy /y "%CARGO_TARGET_DIR%\release\treasury-desktop.exe" "%~dp0..\dist\NBB-Treasury.exe" >nul
echo.
echo Installer: dist\NBB-Treasury-Setup.exe   (per-user install, no admin rights)
echo Portable:  dist\NBB-Treasury.exe          (needs the WebView2 runtime, present on Windows 10/11)
endlocal
