@echo off
setlocal EnableExtensions
title Aurora - release build

cd /d "%~dp0"
set "ROOT=%~dp0"

echo ============================================================
echo  Aurora - signed release build
echo ============================================================
echo.
echo Run build_windows.bat once first, so all tools are installed.
echo.

REM ---- 1. Signing key ----
if not exist "%ROOT%release-keys\aurora-updater.key" (
  call :error "Signing key not found at release-keys\aurora-updater.key. Follow RELEASING.md step 2 to create it."
  goto :fail
)

REM ---- 2. GitHub repository for downloads ----
if "%AURORA_REPO%"=="" (
  call :error "AURORA_REPO is not set. Run this first, then start this script again from the same window:  set AURORA_REPO=yourname/aurora-client"
  goto :fail
)

REM ---- 3. Placeholders must be replaced before a public release ----
findstr /c:"REPLACE_WITH_PUBLIC_KEY" "%ROOT%src-tauri\tauri.conf.json" >nul 2>&1
if not errorlevel 1 (
  call :error "The updater public key is still a placeholder in src-tauri\tauri.conf.json. Paste the public key printed by 'tauri signer generate' (RELEASING.md step 2)."
  goto :fail
)
findstr /c:"REPLACE-OWNER" "%ROOT%src-tauri\tauri.conf.json" >nul 2>&1
if not errorlevel 1 (
  call :error "The update endpoint in src-tauri\tauri.conf.json still has REPLACE-OWNER/REPLACE-REPO. Put your GitHub owner and repository there (RELEASING.md step 2)."
  goto :fail
)

REM ---- 4. Tools from the normal build ----
set "PATH=%USERPROFILE%\.cargo\bin;%ProgramFiles%\nodejs;%PATH%"
if not exist "%ROOT%node_modules" (
  call :error "node_modules is missing. Run build_windows.bat once first."
  goto :fail
)

REM ---- 5. Sign the build ----
set "TAURI_SIGNING_PRIVATE_KEY=%ROOT%release-keys\aurora-updater.key"
echo.
echo ---- Building and signing the installer ----
echo If the key has a password, it is read from TAURI_SIGNING_PRIVATE_KEY_PASSWORD.
call npx tauri build --config src-tauri\tauri.release.conf.json
if errorlevel 1 (
  call :error "The release build failed. Read the first error above. A missing signing password is the most common cause."
  goto :fail
)

REM ---- 6. Collect files and write latest.json ----
echo.
echo ---- Writing release files ----
powershell -NoProfile -ExecutionPolicy Bypass -File "%ROOT%scripts\make_latest_json.ps1" -Repo "%AURORA_REPO%"
if errorlevel 1 (
  call :error "Could not write the release files. Read the message above."
  goto :fail
)

echo.
echo ============================================================
echo  RELEASE FILES READY
echo ============================================================
echo  Upload these to a GitHub Release tagged v<version>:
echo    release-output\Aurora_<version>_x64-setup.exe
echo    release-output\latest.json
echo  See RELEASING.md step 7.
echo ============================================================
pause
exit /b 0

:fail
echo.
echo ============================================================
echo  RELEASE FAILED - read the ERROR line above.
echo  See RELEASING.md.
echo ============================================================
pause
exit /b 1

:error
echo.
echo ERROR: %~1
exit /b 1
