@echo off
setlocal EnableExtensions
title Aurora - Windows build

cd /d "%~dp0"
set "ROOT=%~dp0"

echo ============================================================
echo  Aurora Launcher - automated Windows build
echo ============================================================
echo  Project folder: %ROOT%
echo.

REM Make tools installed by an earlier run visible in this window.
set "PATH=%USERPROFILE%\.cargo\bin;%ProgramFiles%\nodejs;%PATH%"

call :check_winget || goto :fail
call :ensure_node || goto :fail
call :ensure_rust || goto :fail
call :ensure_build_tools || goto :fail
call :clean_old_bundle
call :install_deps || goto :fail
call :build_app || goto :fail
call :collect_outputs || goto :fail

echo.
echo ============================================================
echo  BUILD SUCCEEDED
echo ============================================================
echo  Portable app : %ROOT%output\Aurora.exe
echo  Installer    : %ROOT%output\Aurora_Setup.exe
echo.
echo  Next: run output\Aurora_Setup.exe to install, or run
echo        output\Aurora.exe directly without installing.
echo ============================================================
pause
exit /b 0

:fail
echo.
echo ============================================================
echo  BUILD FAILED - read the ERROR line above.
echo  See README.md, section "Troubleshooting".
echo ============================================================
pause
exit /b 1


REM ------------------------------------------------------------
REM Helpers
REM ------------------------------------------------------------
:step
echo.
echo ---- %~1 ----
exit /b 0

:error
echo.
echo ERROR: %~1
exit /b 1


REM ------------------------------------------------------------
REM 1. winget
REM ------------------------------------------------------------
:check_winget
call :step "Checking winget (Windows Package Manager)"
where winget >nul 2>&1
if errorlevel 1 (
  call :error "winget was not found. Install or update 'App Installer' from the Microsoft Store, then run this script again."
  exit /b 1
)
echo winget found.
exit /b 0


REM ------------------------------------------------------------
REM 2. Node.js (for the web UI build)
REM ------------------------------------------------------------
:ensure_node
call :step "Checking Node.js (needed to build the web UI)"
where node >nul 2>&1
if errorlevel 1 (
  echo Node.js not found. Installing Node.js LTS with winget.
  echo Windows may ask for administrator permission.
  winget install --id OpenJS.NodeJS.LTS -e --accept-package-agreements --accept-source-agreements
  set "PATH=%ProgramFiles%\nodejs;%PATH%"
  where node >nul 2>&1
  if errorlevel 1 (
    call :error "Node.js was installed but is not visible yet. Close this window, open a new one, and run build_windows.bat again."
    exit /b 1
  )
)
for /f "delims=" %%v in ('node --version') do echo Found Node.js %%v
exit /b 0


REM ------------------------------------------------------------
REM 3. Rust toolchain (MSVC)
REM ------------------------------------------------------------
:ensure_rust
call :step "Checking Rust toolchain (rustup and cargo)"
where cargo >nul 2>&1
if errorlevel 1 (
  echo Rust not found. Installing rustup with winget.
  winget install --id Rustlang.Rustup -e --accept-package-agreements --accept-source-agreements
  set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
  where cargo >nul 2>&1
  if errorlevel 1 (
    call :error "Rust was installed but cargo is not visible yet. Open a new window and run build_windows.bat again."
    exit /b 1
  )
)
echo Setting the stable MSVC toolchain as default.
call rustup default stable-msvc
if errorlevel 1 (
  call :error "rustup could not set the stable-msvc toolchain. Check the error above and run: rustup default stable-msvc"
  exit /b 1
)
for /f "delims=" %%v in ('cargo --version') do echo Found %%v
exit /b 0


REM ------------------------------------------------------------
REM 4. Visual C++ Build Tools (the Rust MSVC linker needs them)
REM ------------------------------------------------------------
:ensure_build_tools
call :step "Checking Visual C++ Build Tools (needed to link Rust programs)"
call :find_vs
if defined VS_PATH (
  echo Visual C++ Build Tools found.
  exit /b 0
)
echo Visual C++ Build Tools not found. Installing the C++ workload with winget.
echo This is a large download and can take 10-30 minutes.
echo Windows may ask for administrator permission.
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --accept-package-agreements --accept-source-agreements --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
call :find_vs
if not defined VS_PATH (
  call :error "Visual C++ Build Tools are still missing. Restart Windows and run build_windows.bat again. If it still fails, install 'Desktop development with C++' from the Visual Studio Installer."
  exit /b 1
)
echo Visual C++ Build Tools installed.
exit /b 0

:find_vs
set "VS_PATH="
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" exit /b 0
for /f "usebackq delims=" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VS_PATH=%%i"
exit /b 0


REM ------------------------------------------------------------
REM 5. Remove stale installer output from earlier builds
REM ------------------------------------------------------------
:clean_old_bundle
if exist "%ROOT%src-tauri\target\release\bundle\nsis" rd /s /q "%ROOT%src-tauri\target\release\bundle\nsis"
exit /b 0


REM ------------------------------------------------------------
REM 6. JavaScript dependencies (reproducible when package-lock.json exists)
REM ------------------------------------------------------------
:install_deps
call :step "Installing JavaScript and build dependencies (npm)"
if exist "%ROOT%package-lock.json" (
  call npm ci
) else (
  echo No package-lock.json found - running npm install. Keep the lock file it creates.
  call npm install
)
if errorlevel 1 (
  call :error "npm dependency install failed. Check your internet connection and the error above, then run the script again."
  exit /b 1
)
exit /b 0


REM ------------------------------------------------------------
REM 7. Build the app and the Windows installer
REM ------------------------------------------------------------
:build_app
call :step "Building Aurora (web UI, Rust app, Windows installer)"
echo The first build downloads Rust crates and installer tools and can take 10-20 minutes.
call npx tauri build
if errorlevel 1 (
  call :error "The Tauri build failed. Read the first error message above. Common causes: missing C++ build tools, no internet access, or a Rust compile error."
  exit /b 1
)
exit /b 0


REM ------------------------------------------------------------
REM 8. Copy the two deliverables into the output folder
REM ------------------------------------------------------------
:collect_outputs
call :step "Collecting Aurora.exe and Aurora_Setup.exe into the output folder"
set "BUILT_EXE=%ROOT%src-tauri\target\release\Aurora.exe"
set "BUILT_SETUP="
for %%f in ("%ROOT%src-tauri\target\release\bundle\nsis\*-setup.exe") do set "BUILT_SETUP=%%~ff"
if not exist "%BUILT_EXE%" (
  call :error "Aurora.exe was not created where expected. Check the build output above."
  exit /b 1
)
if not defined BUILT_SETUP (
  call :error "The installer Aurora_Setup.exe was not created. Check the build output above."
  exit /b 1
)
if not exist "%ROOT%output" mkdir "%ROOT%output"
copy /y "%BUILT_EXE%" "%ROOT%output\Aurora.exe" >nul
if errorlevel 1 (
  call :error "Could not copy Aurora.exe to the output folder. Close any running Aurora window and try again."
  exit /b 1
)
copy /y "%BUILT_SETUP%" "%ROOT%output\Aurora_Setup.exe" >nul
if errorlevel 1 (
  call :error "Could not copy the installer to the output folder. Close any running Aurora_Setup.exe and try again."
  exit /b 1
)
echo Copied to output\Aurora.exe and output\Aurora_Setup.exe
exit /b 0
