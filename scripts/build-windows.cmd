@echo off
rem Build the release .exe with the Windows (MSVC) toolchain.
rem Keeps build output off OneDrive: .cargo\config.toml points at a Linux path, so override it here.
setlocal
if "%CARGO_TARGET_DIR%"=="" set CARGO_TARGET_DIR=C:\dev\targets\treasury
cd /d "%~dp0\.."
cargo build --release -p client
if errorlevel 1 exit /b 1
echo.
echo Built: %CARGO_TARGET_DIR%\release\treasury-client.exe
echo Run:   %CARGO_TARGET_DIR%\release\treasury-client.exe http://localhost:8787
endlocal
