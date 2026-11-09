# Builds the files a GitHub release needs for automatic updates:
#   release-output\Aurora_<version>_x64-setup.exe  (installer to upload)
#   release-output\latest.json                              (update manifest to upload)
#
# Run by release_windows.bat. Can also be run by hand:
#   powershell -ExecutionPolicy Bypass -File scripts\make_latest_json.ps1 -Repo owner/repo

param(
    [Parameter(Mandatory = $true)]
    [string]$Repo
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

if ($Repo -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') {
    throw "Repo must look like owner/repo, for example myname/aurora-client. Got: $Repo"
}

# 1. All three version numbers must match before anything is published.
$conf = Get-Content (Join-Path $root "src-tauri\tauri.conf.json") -Raw | ConvertFrom-Json
$version = $conf.version
$pkgVersion = (Get-Content (Join-Path $root "package.json") -Raw | ConvertFrom-Json).version
$cargoText = Get-Content (Join-Path $root "src-tauri\Cargo.toml") -Raw
$cargoVersion = [regex]::Match($cargoText, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if ($pkgVersion -ne $version -or $cargoVersion -ne $version) {
    throw "Version mismatch. tauri.conf.json=$version, package.json=$pkgVersion, Cargo.toml=$cargoVersion. Make them equal and build again."
}

# 2. Find the installer and its signature.
$bundleDir = Join-Path $root "src-tauri\target\release\bundle\nsis"
$setup = Get-ChildItem $bundleDir -Filter "*-setup.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $setup) {
    throw "Installer not found in $bundleDir. Run the release build first."
}
$sigPath = "$($setup.FullName).sig"
if (-not (Test-Path $sigPath)) {
    throw "Signature file not found: $sigPath. The build was not signed. Check the signing key setup in RELEASING.md."
}

# 3. Copy both files to release-output with a clean name (no spaces, so the download URL works).
$outDir = Join-Path $root "release-output"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$assetName = "Aurora_${version}_x64-setup.exe"
Copy-Item $setup.FullName (Join-Path $outDir $assetName) -Force

# 4. Write latest.json. The launcher reads this file to decide whether an update exists.
$signature = (Get-Content $sigPath -Raw).Trim()
$manifest = [ordered]@{
    version   = $version
    notes     = "See the GitHub release page for what changed."
    pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    platforms = [ordered]@{
        "windows-x86_64" = [ordered]@{
            signature = $signature
            url       = "https://github.com/$Repo/releases/download/v$version/$assetName"
        }
    }
}
$json = $manifest | ConvertTo-Json -Depth 6
# Write UTF-8 without a byte-order mark, which some JSON readers reject.
[System.IO.File]::WriteAllText((Join-Path $outDir "latest.json"), $json, (New-Object System.Text.UTF8Encoding($false)))

Write-Host ""
Write-Host "Release $version is ready in: $outDir"
Write-Host "  $assetName"
Write-Host "  latest.json"
