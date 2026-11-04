@echo off
rem Install the web client's dependencies with the Windows Node toolchain.
rem node_modules lives under C:\dev (a junction), off OneDrive, like the cargo target dir.
setlocal
cd /d "%~dp0\..\web"
if not exist "C:\dev\node\treasury-web\node_modules" mkdir "C:\dev\node\treasury-web\node_modules"
if not exist node_modules mklink /J node_modules "C:\dev\node\treasury-web\node_modules" >nul
if exist package-lock.json (
  call npm ci --no-audit --no-fund
) else (
  call npm install --no-audit --no-fund
)
if errorlevel 1 exit /b 1
echo.
echo Web client dependencies installed.
endlocal
