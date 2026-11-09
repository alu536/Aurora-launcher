#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod auth;
mod emotes;
mod minecraft;

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::Manager;

const RAM_MIN_MB: u32 = 2048;
const RAM_MAX_MB: u32 = 16384;
const SETTINGS_FILE: &str = "settings.json";

/// User settings, stored in %APPDATA%\com.aurora.launcher\settings.json.
/// `serde(default)` lets future versions add fields without breaking older files.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub ram_mb: u32,
    pub java_path: String,
    pub game_dir: String,
    pub selected_version: String,
    pub loader: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ram_mb: 4096,
            java_path: String::new(),
            game_dir: String::new(),
            selected_version: String::new(),
            loader: "vanilla".to_string(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    name: String,
    version: String,
}

/// Returns the per-user data folder, creating it if needed.
fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Could not find the app data folder: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Where Minecraft files go: the Game folder setting, or AppData\...\minecraft by default.
fn game_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let settings = load_settings(app.clone())?;
    if settings.game_dir.trim().is_empty() {
        Ok(data_dir(app)?.join("minecraft"))
    } else {
        Ok(PathBuf::from(settings.game_dir.trim()))
    }
}

/// Rejects values the launcher should never store.
fn validate(s: &Settings) -> Result<(), String> {
    if s.loader != "vanilla" && s.loader != "fabric" {
        return Err("Loader must be vanilla or fabric.".to_string());
    }
    if s.ram_mb < RAM_MIN_MB || s.ram_mb > RAM_MAX_MB {
        return Err(format!("RAM must be between {RAM_MIN_MB} and {RAM_MAX_MB} MB."));
    }
    if s.java_path.len() > 1024 || s.game_dir.len() > 1024 || s.selected_version.len() > 64 {
        return Err("A setting value is too long.".to_string());
    }
    if s.java_path.contains('\0') || s.game_dir.contains('\0') || s.selected_version.contains('\0') {
        return Err("Settings contain invalid characters.".to_string());
    }
    Ok(())
}

#[tauri::command]
fn load_settings(app: tauri::AppHandle) -> Result<Settings, String> {
    let path = data_dir(&app)?.join(SETTINGS_FILE);
    if !path.exists() {
        return Ok(Settings::default());
    }
    let text = fs::read_to_string(&path)
        .map_err(|e| format!("Could not read {}: {e}", path.display()))?;

    match serde_json::from_str::<Settings>(&text) {
        Ok(settings) if validate(&settings).is_ok() => Ok(settings),
        _ => {
            // Keep the unreadable file for inspection, then start from defaults.
            let backup = path.with_extension("corrupt.json");
            let _ = fs::rename(&path, &backup);
            Ok(Settings::default())
        }
    }
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: Settings) -> Result<(), String> {
    validate(&settings)?;
    let dir = data_dir(&app)?;
    let final_path = dir.join(SETTINGS_FILE);
    let tmp_path = dir.join("settings.json.tmp");

    let json = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("Could not encode settings: {e}"))?;
    // Write to a temporary file first, then swap it in, so a crash never leaves half a file.
    fs::write(&tmp_path, json).map_err(|e| format!("Could not write settings: {e}"))?;
    fs::rename(&tmp_path, &final_path).map_err(|e| format!("Could not save settings: {e}"))?;
    Ok(())
}

/// Runs `java -version` and returns its first output line.
#[tauri::command]
fn check_java(path: String) -> Result<String, String> {
    let trimmed = path.trim();
    let exe = if trimmed.is_empty() { "java".to_string() } else { trimmed.to_string() };

    if exe != "java" && !Path::new(&exe).is_file() {
        return Err(format!("File not found: {exe}"));
    }

    let mut cmd = Command::new(&exe);
    cmd.arg("-version");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = cmd.output().map_err(|e| {
        format!("Could not run '{exe}': {e}. Install Java 21 or enter the full path to java.exe.")
    })?;

    // `java -version` writes its version line to stderr.
    let text = String::from_utf8_lossy(&output.stderr).to_string();
    let first_line = text
        .lines()
        .next()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty());

    match first_line {
        Some(line) => Ok(line),
        None => Err(format!("'{exe}' ran but did not print a Java version.")),
    }
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        name: "Aurora".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Lists Minecraft release versions from Mojang's official version manifest.
#[tauri::command]
async fn list_versions() -> Result<Vec<String>, String> {
    minecraft::list_release_versions().await
}

#[tauri::command]
fn is_installed(app: tauri::AppHandle, version: String) -> Result<bool, String> {
    minecraft::check_version_id(&version)?;
    let root = game_root(&app)?;
    Ok(minecraft::is_installed(&root, &version))
}

/// Downloads and verifies a version without starting the game.
#[tauri::command]
async fn install_version(app: tauri::AppHandle, version: String) -> Result<String, String> {
    minecraft::check_version_id(&version)?;
    let root = game_root(&app)?;
    minecraft::install(&app, &root, &version).await?;
    Ok(format!("Minecraft {version} is installed."))
}

/// Installs if needed, then starts the game. Stops with a clear message when sign-in is missing.
#[tauri::command]
async fn launch_game(app: tauri::AppHandle, version: String) -> Result<String, String> {
    minecraft::check_version_id(&version)?;
    let settings = load_settings(app.clone())?;
    let root = game_root(&app)?;
    let fabric = settings.loader == "fabric";
    minecraft::launch(app, root, version, settings.ram_mb, settings.java_path, fabric).await
}

/// Installs Fabric for a Minecraft version, including its vanilla files.
#[tauri::command]
async fn install_fabric(app: tauri::AppHandle, version: String) -> Result<String, String> {
    minecraft::check_version_id(&version)?;
    let root = game_root(&app)?;
    minecraft::install_fabric(&app, &root, &version).await?;
    Ok(format!("Fabric is installed for Minecraft {version}."))
}

#[tauri::command]
fn is_fabric_installed(app: tauri::AppHandle, version: String) -> Result<bool, String> {
    minecraft::check_version_id(&version)?;
    let root = game_root(&app)?;
    Ok(minecraft::fabric_installed(&root, &version))
}

#[tauri::command]
fn list_mods(app: tauri::AppHandle, version: String) -> Result<Vec<minecraft::ModEntry>, String> {
    minecraft::check_version_id(&version)?;
    let root = game_root(&app)?;
    minecraft::list_mods(&root, &version)
}

#[tauri::command]
fn add_mods(app: tauri::AppHandle, version: String, paths: Vec<String>) -> Result<usize, String> {
    minecraft::check_version_id(&version)?;
    let root = game_root(&app)?;
    minecraft::add_mods(&root, &version, &paths)
}

#[tauri::command]
fn set_mod_enabled(
    app: tauri::AppHandle,
    version: String,
    file_name: String,
    enabled: bool,
) -> Result<(), String> {
    minecraft::check_version_id(&version)?;
    let root = game_root(&app)?;
    minecraft::set_mod_enabled(&root, &version, &file_name, enabled)
}

#[tauri::command]
fn remove_mod(app: tauri::AppHandle, version: String, file_name: String) -> Result<(), String> {
    minecraft::check_version_id(&version)?;
    let root = game_root(&app)?;
    minecraft::remove_mod(&root, &version, &file_name)
}

/// Emotes are shared by every version, so they live in one library folder.
fn emote_library(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("emotes"))
}

#[tauri::command]
fn list_emotes(app: tauri::AppHandle) -> Result<Vec<emotes::EmoteEntry>, String> {
    emotes::list(&emote_library(&app)?)
}

/// Imports .emotecraft files and returns the full library afterwards.
#[tauri::command]
fn import_emotes(app: tauri::AppHandle, paths: Vec<String>) -> Result<Vec<emotes::EmoteEntry>, String> {
    let library = emote_library(&app)?;
    emotes::import_files(&library, &paths)?;
    emotes::list(&library)
}

#[tauri::command]
fn remove_emote(app: tauri::AppHandle, id: String) -> Result<(), String> {
    emotes::remove(&emote_library(&app)?, &id)
}

/// Starts Microsoft sign-in and returns the code to show the player.
#[tauri::command]
async fn sign_in_start(app: tauri::AppHandle) -> Result<auth::DeviceCodePrompt, String> {
    auth::start_sign_in(app).await
}

#[tauri::command]
async fn account_status() -> Result<auth::AccountInfo, String> {
    auth::status().await
}

#[tauri::command]
fn sign_out() -> Result<(), String> {
    auth::sign_out();
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            load_settings,
            save_settings,
            check_java,
            app_info,
            list_versions,
            is_installed,
            install_version,
            launch_game,
            install_fabric,
            is_fabric_installed,
            list_mods,
            add_mods,
            set_mod_enabled,
            remove_mod,
            list_emotes,
            import_emotes,
            remove_emote,
            sign_in_start,
            account_status,
            sign_out
        ])
        .run(tauri::generate_context!())
        .expect("error while running Aurora");
}
