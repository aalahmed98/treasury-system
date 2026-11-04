@echo off
rem Type-check and build the web client into web\dist, which the server serves at /app.
setlocal
cd /d "%~dp0\..\web"
if not exist node_modules call "%~dp0web-setup.cmd"
call npm run build
if errorlevel 1 exit /b 1
echo.
echo Built web\dist. The server serves it at http://localhost:8787/app/
endlocal
