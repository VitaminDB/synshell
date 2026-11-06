@echo off
rem syn-vm guest tools: install.cmd [/quiet]
rem Exit codes: 0 - installed, 3010 - reboot required (setup finishes by itself after reboot), other - error.
rem Log: C:\ProgramData\syn-vm\install.log
setlocal
set "PSARGS="
if /i "%~1"=="/quiet" set "PSARGS=-Quiet"
net session >nul 2>&1
if errorlevel 1 (
    powershell -NoProfile -ExecutionPolicy Bypass -Command "$p = Start-Process -FilePath powershell -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','\"%~dp0install.ps1\"' -Verb RunAs -Wait -PassThru; exit $p.ExitCode"
    exit /b %errorlevel%
)
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %PSARGS%
exit /b %errorlevel%
