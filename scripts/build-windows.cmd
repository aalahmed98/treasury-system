@echo off
rem Build the release .exe with the Windows (MSVC) toolchain.
rem Keeps build output off OneDrive: .cargo\config.toml points at a Linux path, so override it here.
setlocal
if "%CARGO_TARGET_DIR%"=="" set CARGO_TARGET_DIR=C:\dev\targets\treasury
cd /d "%~dp0\.."
cargo build --release -p client
if errorlevel 1 exit /b 1
if not exist "%~dp0..\dist" mkdir "%~dp0..\dist"
copy /y "%CARGO_TARGET_DIR%\release\treasury-client.exe" "%~dp0..\dist\treasury-client.exe" >nul
echo.
echo Built: %CARGO_TARGET_DIR%\release\treasury-client.exe
echo Copy:  %~dp0..\dist\treasury-client.exe
echo Run:   dist\treasury-client.exe   (the server must be running; see scripts\start-dev.sh)
endlocal
