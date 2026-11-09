# Releasing Aurora with automatic updates

Every installed copy of Aurora checks for updates a few seconds after it starts. If a newer signed version is published, the launcher downloads it, installs it, and restarts. Your settings and Minecraft files are not touched.

Before you publish to players, you must do the one-time setup below. The updater refuses to install anything not signed with your private key, so the key matters more than anything else in this guide.

## How it works

1. You build a release. The build creates the installer and a **signature** made with your private key.
2. You upload the installer and a small file, `latest.json`, to a GitHub Release.
3. Each installed launcher downloads `latest.json` from `https://github.com/<owner>/<repo>/releases/latest/download/latest.json`.
4. If the version in that file is newer, the launcher downloads the installer, checks the signature against the **public key** built into the app, and installs it only if the check passes.

The signing key is separate from Windows code signing. The updater key proves an update came from you. Windows code signing (a paid certificate) stops the "unknown publisher" SmartScreen warning. Get that later if you can.

## One-time setup

### Step 1: Install the build tools

Run `build_windows.bat` once from the project folder. It installs everything and creates `node_modules`.

### Step 2: Create the signing key and point the app at your repository

1. Open a Command Prompt in the project folder and run:

   ```
   npx tauri signer generate -w release-keys\aurora-updater.key
   ```

2. Choose a strong password when asked. Write it down in a password manager.
3. The command prints a **public key**, a long line of text. Copy it.
4. Open `src-tauri\tauri.conf.json` and replace the placeholder in `plugins.updater.pubkey` with that public key.
5. In the same file, replace `REPLACE-OWNER` and `REPLACE-REPO` in `plugins.updater.endpoints` with your GitHub username and repository name.

**Back up `release-keys\aurora-updater.key` and its password in at least two safe places, such as a password manager and an encrypted drive.**

- If you lose the key, you cannot publish updates that existing installs will accept. Players would have to reinstall by hand.
- If someone else gets the key, they can publish malicious updates to every player.
- The `release-keys` folder is listed in `.gitignore`. Never commit it, upload it, or send it to anyone.

### Step 3: Create the GitHub repository

1. Create a GitHub repository. It **must be public**, or players can't download the releases without signing in. The source code is public too, so don't put secrets in the repository.
2. Upload the project source to it, excluding `release-keys` and `release-output`. Git ignores both folders through `.gitignore`.

If you don't want the source public, host `latest.json` and the installer on your own web server instead. You would change the endpoint in `tauri.conf.json` and the `url` built by `scripts\make_latest_json.ps1`.

## Every release

### Step 4: Bump the version in three places

Use the same version number in all three:

- `package.json` (the `"version"` field)
- `src-tauri\tauri.conf.json` (the `"version"` field)
- `src-tauri\Cargo.toml` (the `version = "..."` line)

The release script stops if they don't match.

### Step 5: Set the repository and run the release script

In the same Command Prompt window:

```
set AURORA_REPO=yourname/aurora-client
set TAURI_SIGNING_PRIVATE_KEY_PASSWORD=the-password-you-chose
release_windows.bat
```

The `set` commands only last for that window. Don't put the password in a file.

The script checks the setup, builds the installer, signs it, and writes two files to `release-output\`:

- `Aurora_<version>_x64-setup.exe`
- `latest.json`

### Step 6: Test the update before publishing

Test with a copy of the previous version:

1. Install the previous version, for example 0.3.0, on a test PC.
2. Publish the new version as described below.
3. Start the installed 0.3.0. After about four seconds it should download the update, restart, and show the new version under About.
4. Check that your settings are still there.

### Step 7: Publish the GitHub Release

1. On GitHub, open the repository, go to **Releases**, and choose **Draft a new release**.
2. Set the tag to `v<version>`, for example `v0.3.1`. The tag must match the version exactly.
3. Upload both `release-output` files: the installer and `latest.json`.
4. Make sure it's not a pre-release. The updater uses the latest non-pre-release.
5. Publish.

Installed launchers pick it up the next time they start, or when a player clicks **Check for updates** under About.

## Rules for players

- Updates install automatically when the launcher starts. If Minecraft is running, the update waits until the game closes.
- The launcher never downgrades on its own. To roll back a bad release, publish a higher version with the fix.
- The first time a player installs, Windows SmartScreen may warn that the publisher is unknown. Until the installer is code-signed with a Windows certificate, expect that warning on new installs.

## Troubleshooting

| Problem | Fix |
|---|---|
| `Signing key not found` | Run step 2. The key must be at `release-keys\aurora-updater.key`. |
| `AURORA_REPO is not set` | Run the `set AURORA_REPO=...` command in the same window. |
| `public key is still a placeholder` / `REPLACE-OWNER` | Finish step 2 and edit `src-tauri\tauri.conf.json`. |
| `Version mismatch` | Make the three version numbers in step 4 equal, then build again. |
| Build fails asking for a password | Set `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in the window, as in step 5. |
| Launcher says "Could not check for updates" | Check that the release is public and that `latest.json` opens in a browser at the endpoint URL. |
| Launcher says "Update failed" with a signature error | The release was signed with a different key from the one in the app. Don't publish an update with the wrong key. Players must reinstall once. |
| Updates never show up | The new version number must be higher than the installed one. The GitHub release must not be a pre-release. |

## Files involved

- `src-tauri\tauri.conf.json`: public key and update endpoint (`plugins.updater`)
- `src-tauri\tauri.release.conf.json`: turns on signed update files for release builds only
- `release_windows.bat`: release build
- `scripts\make_latest_json.ps1`: checks versions, collects the installer, writes `latest.json`
- `release-keys\aurora-updater.key`: your private key (secret, not in the repository)
- `release-output\`: files to upload (not in the repository)
