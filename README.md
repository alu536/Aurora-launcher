# Aurora Launcher - Windows build guide

This folder contains the Aurora Launcher (milestone 1) and everything needed to build it on Windows 10 or Windows 11 by double-clicking `build_windows.bat`.

## Read this first: what has been verified

I could not compile or run this project in the environment where it was written, so nothing here has been tested on Windows yet.

| Item | Status |
|---|---|
| Logo and icon files (`src-tauri/icons`, `public/logo.png`) | Generated and checked for valid PNG/ICO structure |
| Version 0.2.0 UI (`src/App.tsx`, `src/styles.css`) | Written and checked for bracket balance and CSS class coverage; **not rendered or compiled** |
| JSON configuration files | Parsed successfully |
| `build_windows.bat` | Written carefully, **not run yet** |
| Rust code (`src-tauri/src/main.rs`) | **Not compiled yet** |
| Minecraft pipeline (`src-tauri/src/minecraft.rs`) | Written against Mojang's documented format, **not compiled or run**; no game has been started |
| React/TypeScript UI | **Not type-checked or run yet** |
| `Aurora.exe` and `Aurora_Setup.exe` | **Not built yet** - you produce them by running the script |

When you run the build, the first failure (if any) is the most useful thing to send back. Include the full text of the `ERROR:` line and the lines just above it.

## What this milestone includes

- A Lunar-style layout (version 0.4.0): a top navigation bar, a hero with animated embers and a spinning accretion ring around the logo, a bottom dock with account, Play, and stats, and a feature row. Cosmetics, emotes, marketplace, and rewards show as "Soon"
- A version picker that lists Minecraft releases from Mojang's official manifest
- An Install button that downloads the selected version, verifies every file with SHA-1, and extracts the native libraries, with a live progress bar
- Fabric and mods (version 0.5.0): switch the loader to Fabric on Home, install Fabric for a version, then use the Mods tab to add Fabric `.jar` files, turn them on and off, or remove them. Each Fabric version has its own `instances\fabric-<version>` folder with its own `mods`, saves, and logs. Mods without a `fabric.mod.json` are rejected. Mods need the game to launch, which needs sign-in, so they can't be tested in game yet
- Emote library (version 0.6.0): drop `.emotecraft` files anywhere in the launcher, or use Choose files on the Emotes tab. The launcher copies each file into its library and reads the name, description, author, body parts, and embedded icon from the file. Press **B** to open the emote wheel. Choosing an emote on the wheel only records it for now. Playback needs the in-game client, which isn't built yet
- Automatic updates: the launcher checks for signed updates at startup and installs them when Minecraft is not running. See `RELEASING.md` to publish one
- A Play button that checks Java and starts Minecraft. It stops with a clear message until Microsoft sign-in exists, because Minecraft won't start without a signed-in account
- Settings for RAM allocation (2 to 16 GB), Java path with a "Check Java" button, and a game folder
- Settings saved to your user AppData folder, so updates don't erase them
- Desktop and Start Menu shortcuts from the installer (see "Verify after install" below)

## What this milestone does NOT include yet

These are part of the full Aurora plan but are not implemented here. The launcher says so in its UI rather than showing fake data.

- Microsoft sign-in (needs an Azure app registration and Mojang/Minecraft services approval)
- The Aurora backend, database, permissions, owner dashboard
- The in-game client (Fabric mod), cosmetics, capes, emotes, marketplace, daily rewards

Each of these will be built and tested in its own milestone.

## Requirements

- Windows 10 (version 1809 or later) or Windows 11, 64-bit
- An internet connection (downloads Node.js, Rust, Visual C++ Build Tools, Rust crates, and the installer tools)
- About 15 GB of free disk space
- `winget` (Windows Package Manager). It comes with Windows 11 and most current Windows 10 installs. If the script says it's missing, install or update "App Installer" from the Microsoft Store.
- Permission to approve Windows administrator (UAC) prompts for the Node.js and Visual C++ Build Tools installs
- A project path with no special characters. Avoid `&`, `!`, `%`, and `^` in the folder name. Good example: `C:\Projects\Aurora`.

You do not need to install Python, Rust, Node.js, or Visual Studio yourself. The script checks for each one and installs what is missing.

## Step-by-step: build

1. **Extract the project.** Right-click `Aurora-project.zip` and choose "Extract All". Put the folder somewhere simple, such as `C:\Projects\Aurora`. Don't run the script from inside the zip.
2. **Start the build.** Open the `Aurora` folder and double-click `build_windows.bat`.
3. **Approve prompts.** If Windows asks for administrator permission to install Node.js or Visual C++ Build Tools, click Yes.
4. **Handle "installed, please reopen" messages.** If the script installs Node.js, Rust, or the build tools, it may stop and ask you to open a new window. Close the window and double-click `build_windows.bat` again. The tools are already installed, so the second run goes further.
5. **Wait.** The first build takes about 10 to 30 minutes, mostly downloading and compiling Rust crates. Later builds are much faster.
6. **Check the result.** The window ends with either `BUILD SUCCEEDED` or `BUILD FAILED`. On success, two files are in the `output` folder:
   - `output\Aurora.exe` - runs the launcher without installing it
   - `output\Aurora_Setup.exe` - the Windows installer

## Step-by-step: install

1. Double-click `output\Aurora_Setup.exe`.
2. Follow the installer. It installs for the current user only, so no administrator rights are needed.
3. Start Aurora from the desktop shortcut or the Start Menu (search "Aurora").

## What the build script does

`build_windows.bat` runs these steps in order and stops with a clear message at the first failure:

1. Checks that `winget` is available.
2. Checks for Node.js. If missing, installs Node.js LTS with winget.
3. Checks for Rust (`cargo`). If missing, installs rustup with winget and sets the stable MSVC toolchain as default.
4. Checks for Visual C++ Build Tools with `vswhere`. If missing, installs the C++ workload with winget.
5. Removes old installer output so you never get a stale file.
6. Installs npm packages with `npm ci` when `package-lock.json` exists (exact versions), or `npm install` the first time.
7. Runs `npx tauri build`. This compiles the web UI, compiles the Rust app, and creates the NSIS installer. Tauri downloads its installer tools on the first build.
8. Copies the two results into `output\`.

## Verify after install

Because the installer config was written without a Windows test, check these by hand and tell me what you see:

- [ ] Desktop shortcut "Aurora" exists after install. If you see **two** desktop shortcuts, delete one and tell me. It means the installer hook and the default template both create one.
- [ ] Start Menu entry "Aurora" exists.
- [ ] The window title is "Aurora Launcher" and the logo appears in the icon rail and on the Home hero.
- [ ] Home: the background orbs drift and the logo floats. Turn on Windows "Show animations" off and check that motion stops (reduced-motion support).
- [ ] Settings: the preset buttons change the RAM value, and the bottom save bar appears only when something has changed.
- [ ] Version picker: the list fills with Minecraft release versions (needs internet). If it stays empty, read the error text under the Play button.
- [ ] Install: pick a version and press Install. The progress bar moves through Version list, Game client, Libraries, Natives, and Assets. Run it twice; the second time should finish almost immediately.
- [ ] Play: with no sign-in, it should stop with the sign-in message and not crash. Check that `versions\<version>\install-complete.json` exists after Install.
- [ ] Saving shows a green toast in the bottom-right corner that disappears after about 4 seconds.
- [ ] Settings: change RAM, press Save, close the app, reopen it. The value should still be there.
- [ ] Settings: type `java` in Java path and press "Check Java". If Java 21 is installed, it should show the version line. If not, you should see a clear error.
- [ ] Settings: type a bad path (for example `C:\nope\java.exe`) and press "Check Java". You should see "File not found", not a crash.
- [ ] Uninstall from Windows Settings > Apps. The desktop and Start Menu shortcuts should disappear.
- [ ] Install a second time over the first install. Your saved settings should remain.

## Updating without losing data

- **Where data lives:** `%APPDATA%\com.aurora.launcher\settings.json`. The installer only writes program files under the install folder, so installing a newer `Aurora_Setup.exe` keeps your settings.
- **Damaged settings file:** if the file can't be read, the launcher renames it to `settings.corrupt.json` and starts with defaults. Your original file is kept for inspection.
- **Forward compatibility:** missing settings fields are filled with defaults, so future versions can add settings without breaking older files.
- **Bumping the version:** when releasing a new version, change the version in all three files: `package.json`, `src-tauri\Cargo.toml`, and `src-tauri\tauri.conf.json`.
- **Do not change the identifier:** `com.aurora.launcher` in `src-tauri\tauri.conf.json` decides the AppData folder. Changing it would make existing users lose their settings.

## Publishing updates

To release a version that installed copies update themselves automatically, follow `RELEASING.md`. It covers the one-time signing key setup, the release script `release_windows.bat`, and the GitHub Release steps.

## Emotes

Emotes are stored in `%APPDATA%\com.aurora.launcher\emotes`, one folder per emote with the original file, its icon, and `meta.json`. Only add emotes you have permission to use. Emote creators keep their rights, and a marketplace later needs the creator's consent.

The importer reads only what it can verify in the file. It rejects files with no body-part names. It does not check the animation data, because that format isn't documented in this project.

## Microsoft sign-in

Sign-in needs an approved Azure client ID before it works. See `AUTH.md` for the setup steps and the Mojang approval.

## Reproducible builds

- After your first successful build, keep these files under version control: `package-lock.json` and `src-tauri\Cargo.lock`. They pin exact versions, so the same project builds the same way later.
- The script uses `npm ci` whenever `package-lock.json` exists, so it refuses to silently pick newer versions.

## Troubleshooting

| Message or symptom | What to do |
|---|---|
| `winget was not found` | Install or update "App Installer" from the Microsoft Store and run the script again. |
| `Node.js was installed but is not visible yet` | Close the window, open a new one, run the script again. |
| `Rust was installed but cargo is not visible yet` | Same as above. |
| `Visual C++ Build Tools are still missing` | Restart Windows, run the script again. If it still fails, open the Visual Studio Installer, click Modify on Build Tools 2022, and tick "Desktop development with C++". |
| `npm dependency install failed` | Check your internet connection. If a proxy or firewall is in use, configure npm's proxy settings first. |
| `The Tauri build failed` | Scroll up to the first error. Common causes: missing C++ build tools, no internet during the first build (installer tools are downloaded), or a Rust compile error. Send me that first error. |
| Build works but the installer is missing | Check that the build log shows `Bundling ... setup.exe`. If not, send me the log. |
| Windows SmartScreen warns about the installer | Expected: the installer is not code-signed. Click "More info" then "Run anyway". Signing needs a certificate from a trusted authority. |
| Path with special characters causes odd errors | Move the folder to a simple path such as `C:\Projects\Aurora`. |

## Project layout

```
Aurora/
  build_windows.bat          Automated Windows build (double-click this)
  README.md                  This guide
  package.json               Web UI dependencies and scripts
  index.html                 Web UI entry page
  vite.config.ts             Web UI build settings
  tsconfig.json              TypeScript settings
  src/                       Web UI (React + TypeScript)
    main.tsx
    App.tsx                  Launcher screens: Play, Settings, About
    styles.css               Dark Aurora theme
  public/logo.png            Logo shown in the launcher
  assets/logo.png            Full-size logo source (1024 px)
  scripts/make_icons.py      Regenerates logo and icons (optional, needs Python)
  src-tauri/                 Rust app and Windows packaging
    Cargo.toml               Rust dependencies
    build.rs                 Build hook for Tauri
    tauri.conf.json          App name, publisher, window, installer settings
    capabilities/default.json  Permissions for the main window
    src/main.rs              Settings storage, Java check, app info, Minecraft commands
    src/minecraft.rs         Version list, verified downloads, natives, launch command
    windows/hooks.nsi        Installer hook: desktop shortcut
    icons/                   App icons (icon.ico for Windows)
  output/                    Created by the build: Aurora.exe and Aurora_Setup.exe
```

Python is only needed if you want to regenerate the icons with `python scripts\make_icons.py`. The build itself does not use Python.
