@echo off
setlocal
set ELECTRON_RUN_AS_NODE=1
"%~dp0..\..\..\..\legate.exe" "%~dp0..\..\..\app.asar\out\main\cli.js" %*
exit /b %ERRORLEVEL%
