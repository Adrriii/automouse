@echo off
REM Cuts a new release: bumps the version, builds, uploads to the CDN,
REM tags, and publishes the GitHub release.
REM
REM Double-click it, or run from a terminal:
REM   release.cmd                  (asks which part of the version to bump)
REM   release.cmd -Bump minor      (non-interactive)
REM   release.cmd -Bump patch -DryRun

setlocal
set "hadargs=%~1"

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0installer\release.ps1" %*
set "code=%ERRORLEVEL%"

REM Double-clicking passes no arguments, so keep the window open in that case.
if not defined hadargs (
    echo.
    pause
)
exit /b %code%
