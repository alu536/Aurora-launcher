//! Minecraft install and launch pipeline for Aurora.
//!
//! Files come only from Mojang's official endpoints, and every download is checked
//! against its SHA-1 hash from Mojang's metadata. The game will not start without a
//! signed-in Minecraft session; see `current_session`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use futures::stream::{self, StreamExt};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use tauri::{AppHandle, Emitter};

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const ASSET_BASE_URL: &str = "https://resources.download.minecraft.net";
const DOWNLOAD_CONCURRENCY: usize = 12;
const MARKER_FILE: &str = "install-complete.json";
const LAUNCHER_NAME: &str = "aurora-client";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------- Progress events ----------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub stage: String,
    pub done: usize,
    pub total: usize,
    pub message: String,
}

fn emit(app: &AppHandle, stage: &str, done: usize, total: usize, message: &str) {
    let _ = app.emit(
        "install-progress",
        Progress {
            stage: stage.to_string(),
            done,
            total,
            message: message.to_string(),
        },
    );
}

// ---------- Mojang metadata types ----------

#[derive(Deserialize)]
struct Manifest {
    versions: Vec<ManifestEntry>,
}

#[derive(Deserialize, Clone)]
struct ManifestEntry {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    url: String,
    #[serde(default)]
    sha1: String,
}

#[derive(Deserialize, Clone)]
struct Artifact {
    path: Option<String>,
    #[serde(default)]
    sha1: String,
    url: String,
}

#[derive(Deserialize)]
struct Downloads {
    client: Artifact,
}

#[derive(Deserialize, Clone)]
struct LibDownloads {
    artifact: Option<Artifact>,
    classifiers: Option<HashMap<String, Artifact>>,
}

#[derive(Deserialize, Clone)]
struct OsRule {
    name: Option<String>,
    arch: Option<String>,
}

#[derive(Deserialize, Clone)]
struct Rule {
    action: String,
    os: Option<OsRule>,
    features: Option<Value>,
}

#[derive(Deserialize, Clone)]
struct Library {
    name: String,
    downloads: Option<LibDownloads>,
    rules: Option<Vec<Rule>>,
    natives: Option<HashMap<String, String>>,
    url: Option<String>,
    sha1: Option<String>,
}

#[derive(Deserialize)]
struct Arguments {
    #[serde(default)]
    game: Vec<Value>,
    #[serde(default)]
    jvm: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JavaVersion {
    major_version: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetIndexRef {
    id: String,
    sha1: String,
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionJson {
    id: String,
    main_class: String,
    asset_index: AssetIndexRef,
    downloads: Downloads,
    libraries: Vec<Library>,
    arguments: Option<Arguments>,
    minecraft_arguments: Option<String>,
    java_version: Option<JavaVersion>,
}

#[derive(Deserialize)]
struct AssetIndex {
    objects: HashMap<String, AssetObject>,
}

#[derive(Deserialize)]
struct AssetObject {
    hash: String,
}

// ---------- Helpers ----------

/// Accepts only simple version names such as "1.21.4", so a name can never escape the game folder.
pub fn check_version_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= 32
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("Invalid version name: {id}"))
    }
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "osx",
        _ => "linux",
    }
}

/// Evaluates Mojang's rule list. Feature-based rules (demo mode, custom resolution) never match.
fn rule_allows(rules: &[Rule]) -> bool {
    let mut allowed = false;
    for rule in rules {
        if rule.features.is_some() {
            continue;
        }
        let os_matches = match &rule.os {
            None => true,
            Some(os) => {
                os.name.as_deref().map_or(true, |n| n == os_name())
                    && os.arch.as_deref().map_or(true, |a| a != "x86")
            }
        };
        if os_matches {
            allowed = rule.action == "allow";
        }
    }
    allowed
}

fn sha1_hex(data: &[u8]) -> String {
    let digest = Sha1::digest(data);
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

fn file_matches(path: &Path, expected_sha1: &str) -> bool {
    match fs::read(path) {
        Ok(data) => sha1_hex(&data).eq_ignore_ascii_case(expected_sha1),
        Err(_) => false,
    }
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("Aurora/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("Could not start the network client: {e}"))
}

async fn fetch_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client.get(url).send().await.map_err(|e| {
        format!("Could not reach {url}. Check your internet connection. ({e})")
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Server returned HTTP {status} for {url}"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Download interrupted for {url}: {e}"))?;
    Ok(bytes.to_vec())
}

async fn fetch_json<T: DeserializeOwned>(client: &reqwest::Client, url: &str) -> Result<T, String> {
    let bytes = fetch_bytes(client, url).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Unexpected data from {url}: {e}"))
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let data = fs::read(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    serde_json::from_slice(&data).map_err(|e| format!("Could not parse {}: {e}", path.display()))
}

/// Downloads `url` to `path` unless a file with the expected SHA-1 is already there.
async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    expected_sha1: &str,
) -> Result<(), String> {
    if !expected_sha1.is_empty() && file_matches(path, expected_sha1) {
        return Ok(());
    }
    let data = fetch_bytes(client, url).await?;
    if !expected_sha1.is_empty() && !sha1_hex(&data).eq_ignore_ascii_case(expected_sha1) {
        return Err(format!(
            "Checksum mismatch for {url}. The download may be corrupt. Try again."
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("part");
    fs::write(&tmp, &data).map_err(|e| format!("Could not write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("Could not save {}: {e}", path.display()))?;
    Ok(())
}

#[derive(Clone)]
struct DownloadTask {
    url: String,
    path: PathBuf,
    sha1: String,
}

/// Runs many downloads at once and reports progress.
async fn download_all(
    app: &AppHandle,
    client: &reqwest::Client,
    stage: &str,
    tasks: Vec<DownloadTask>,
) -> Result<(), String> {
    let total = tasks.len();
    emit(app, stage, 0, total, "");
    if total == 0 {
        return Ok(());
    }
    let mut done = 0usize;
    let mut running = stream::iter(tasks.into_iter().map(|t| {
        let client = client.clone();
        async move { download_verified(&client, &t.url, &t.path, &t.sha1).await }
    }))
    .buffer_unordered(DOWNLOAD_CONCURRENCY);

    while let Some(result) = running.next().await {
        result?;
        done += 1;
        if done % 25 == 0 || done == total {
            emit(app, stage, done, total, "");
        }
    }
    Ok(())
}

fn version_dir(root: &Path, version: &str) -> PathBuf {
    root.join("versions").join(version)
}

fn load_version_json(root: &Path, version: &str) -> Result<VersionJson, String> {
    let path = version_dir(root, version).join(format!("{version}.json"));
    read_json(&path)
}

/// True when a previous install finished completely.
pub fn is_installed(root: &Path, version: &str) -> bool {
    version_dir(root, version).join(MARKER_FILE).is_file()
}

// ---------- Version list ----------

pub async fn list_release_versions() -> Result<Vec<String>, String> {
    let client = http_client()?;
    let manifest: Manifest = fetch_json(&client, MANIFEST_URL).await?;
    Ok(manifest
        .versions
        .into_iter()
        .filter(|v| v.kind == "release")
        .map(|v| v.id)
        .collect())
}

// ---------- Install ----------

fn extract_natives(jar: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(jar).map_err(|e| format!("Could not open {}: {e}", jar.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("{} is not a valid archive: {e}", jar.display()))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Could not read {}: {e}", jar.display()))?;
        if entry.is_dir() {
            continue;
        }
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        if rel.starts_with("META-INF") {
            continue;
        }
        let out = dest.join(&rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
        }
        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .map_err(|e| format!("Could not extract {}: {e}", rel.display()))?;
        fs::write(&out, &data).map_err(|e| format!("Could not write {}: {e}", out.display()))?;
    }
    Ok(())
}

struct LibraryPlan {
    tasks: Vec<DownloadTask>,
    classpath: Vec<PathBuf>,
    native_jars: Vec<PathBuf>,
}

/// Works out which libraries this OS needs, without downloading anything.
fn plan_libraries(root: &Path, libs: &[Library]) -> Result<LibraryPlan, String> {
    let libs_root = root.join("libraries");
    let mut plan = LibraryPlan {
        tasks: Vec::new(),
        classpath: Vec::new(),
        native_jars: Vec::new(),
    };

    for lib in libs {
        if let Some(rules) = &lib.rules {
            if !rule_allows(rules) {
                continue;
            }
        }
        if lib.downloads.is_none() {
            // Fabric libraries give a Maven base URL instead of download info.
            let base = lib
                .url
                .as_deref()
                .ok_or_else(|| format!("Library {} has no download information.", lib.name))?;
            let rel = maven_path(&lib.name)
                .ok_or_else(|| format!("Library name {} is not valid.", lib.name))?;
            let path = libs_root.join(&rel);
            plan.tasks.push(DownloadTask {
                url: maven_url(base, &rel),
                path: path.clone(),
                sha1: lib.sha1.clone().unwrap_or_default(),
            });
            plan.classpath.push(path);
            continue;
        }

        let downloads = lib
            .downloads
            .as_ref()
            .ok_or_else(|| format!("Library {} has no download information.", lib.name))?;

        if let Some(natives) = &lib.natives {
            if let Some(key_template) = natives.get(os_name()) {
                let key = key_template.replace("${arch}", "64");
                let art = downloads
                    .classifiers
                    .as_ref()
                    .and_then(|c| c.get(&key))
                    .ok_or_else(|| format!("Missing native files {key} for {}", lib.name))?;
                let rel = art
                    .path
                    .clone()
                    .ok_or_else(|| format!("No file path for {}", lib.name))?;
                let path = libs_root.join(rel);
                plan.tasks.push(DownloadTask {
                    url: art.url.clone(),
                    path: path.clone(),
                    sha1: art.sha1.clone(),
                });
                plan.native_jars.push(path);
                continue;
            }
        }

        if let Some(art) = &downloads.artifact {
            let rel = art
                .path
                .clone()
                .ok_or_else(|| format!("No file path for {}", lib.name))?;
            let path = libs_root.join(rel);
            plan.tasks.push(DownloadTask {
                url: art.url.clone(),
                path: path.clone(),
                sha1: art.sha1.clone(),
            });
            plan.classpath.push(path);
        }
    }
    Ok(plan)
}

/// Downloads and verifies everything needed for `version`. Skips work if it already finished.
/// Returns the parsed version file, which the launch step needs.
async fn install_files(app: &AppHandle, root: &Path, version: &str) -> Result<VersionJson, String> {
    check_version_id(version)?;
    if is_installed(root, version) {
        emit(app, "Ready", 1, 1, "Already installed");
        return load_version_json(root, version);
    }

    let client = http_client()?;
    fs::create_dir_all(root)
        .map_err(|e| format!("Could not create game folder {}: {e}", root.display()))?;

    emit(app, "Version list", 0, 1, "Reading Mojang's version list");
    let manifest: Manifest = fetch_json(&client, MANIFEST_URL).await?;
    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == version)
        .ok_or_else(|| format!("Minecraft {version} was not found in Mojang's version list."))?
        .clone();

    let vdir = version_dir(root, version);
    fs::create_dir_all(&vdir)
        .map_err(|e| format!("Could not create {}: {e}", vdir.display()))?;
    download_verified(
        &client,
        &entry.url,
        &vdir.join(format!("{version}.json")),
        &entry.sha1,
    )
    .await?;
    let vj = load_version_json(root, version)?;

    // Game client
    emit(app, "Game client", 0, 1, "Downloading the game client");
    download_verified(
        &client,
        &vj.downloads.client.url,
        &vdir.join(format!("{version}.jar")),
        &vj.downloads.client.sha1,
    )
    .await?;
    emit(app, "Game client", 1, 1, "Game client verified");

    // Libraries and natives
    let plan = plan_libraries(root, &vj.libraries)?;
    download_all(app, &client, "Libraries", plan.tasks).await?;

    let natives_dir = vdir.join("natives");
    fs::create_dir_all(&natives_dir)
        .map_err(|e| format!("Could not create {}: {e}", natives_dir.display()))?;
    emit(app, "Natives", 0, plan.native_jars.len(), "Extracting native libraries");
    for (i, jar) in plan.native_jars.iter().enumerate() {
        extract_natives(jar, &natives_dir)?;
        emit(app, "Natives", i + 1, plan.native_jars.len(), "");
    }

    // Assets
    let assets_root = root.join("assets");
    let index_path = assets_root
        .join("indexes")
        .join(format!("{}.json", vj.asset_index.id));
    download_verified(
        &client,
        &vj.asset_index.url,
        &index_path,
        &vj.asset_index.sha1,
    )
    .await?;
    let index: AssetIndex = read_json(&index_path)?;
    let objects_root = assets_root.join("objects");
    let asset_tasks: Vec<DownloadTask> = index
        .objects
        .values()
        .map(|o| {
            let prefix = &o.hash[..2];
            DownloadTask {
                url: format!("{ASSET_BASE_URL}/{prefix}/{}", o.hash),
                path: objects_root.join(prefix).join(&o.hash),
                sha1: o.hash.clone(),
            }
        })
        .collect();
    download_all(app, &client, "Assets", asset_tasks).await?;

    // Mark the install complete so later launches skip the checks.
    let marker = serde_json::json!({ "version": version }).to_string();
    fs::write(vdir.join(MARKER_FILE), marker)
        .map_err(|e| format!("Could not finish the install: {e}"))?;
    emit(app, "Ready", 1, 1, "Installed");
    Ok(vj)
}

/// Public install entry point used by the Install button.
pub async fn install(app: &AppHandle, root: &Path, version: &str) -> Result<(), String> {
    install_files(app, root, version).await?;
    Ok(())
}

// ---------- Launch ----------

pub struct Session {
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub user_type: String,
}

/// Minecraft only starts with a valid session from a Microsoft account that owns the game.
/// Sign-in is the next milestone, so for now this returns a clear message instead of a session.
fn current_session() -> Result<Session, String> {
    Err(
        "Sign-in is not set up yet. Minecraft needs a signed-in Microsoft account that owns \
         Minecraft Java Edition, and sign-in is the next milestone. You can still use Install to \
         download and verify the game files."
            .to_string(),
    )
}

fn parse_java_major(text: &str) -> Option<u32> {
    let start = text.find('"')? + 1;
    let rest = &text[start..];
    let end = rest.find('"')?;
    let mut parts = rest[..end]
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty());
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

fn java_major(exe: &str) -> Result<u32, String> {
    if exe != "java" && !Path::new(exe).is_file() {
        return Err(format!("Java was not found at {exe}. Fix the Java path in Settings."));
    }
    let mut cmd = Command::new(exe);
    cmd.arg("-version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd.output().map_err(|e| {
        format!("Could not run Java ({exe}): {e}. Install Java 21 or set the path in Settings.")
    })?;
    let text = String::from_utf8_lossy(&output.stderr).to_string();
    parse_java_major(&text).ok_or_else(|| "Could not read the Java version from java -version.".to_string())
}

fn substitute(template: &str, subs: &HashMap<&str, String>) -> String {
    let mut out = template.to_string();
    for (key, value) in subs {
        out = out.replace(&format!("${{{key}}}"), value);
    }
    out
}

fn push_arg(out: &mut Vec<String>, item: &Value, subs: &HashMap<&str, String>) {
    match item {
        Value::String(s) => out.push(substitute(s, subs)),
        Value::Object(map) => {
            let rules: Vec<Rule> = match map.get("rules") {
                Some(r) => serde_json::from_value(r.clone()).unwrap_or_default(),
                None => Vec::new(),
            };
            if !rules.is_empty() && !rule_allows(&rules) {
                return;
            }
            match map.get("value") {
                Some(Value::String(s)) => out.push(substitute(s, subs)),
                Some(Value::Array(values)) => {
                    for v in values {
                        if let Value::String(s) = v {
                            out.push(substitute(s, subs));
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// Installs if needed, checks the session and Java, then starts Minecraft.
pub async fn launch(
    app: AppHandle,
    root: PathBuf,
    version: String,
    ram_mb: u32,
    java_path: String,
    fabric: bool,
) -> Result<String, String> {
    check_version_id(&version)?;

    // 1. Sign-in comes first. Installing files is pointless if the game cannot start anyway.
    let session = crate::auth::launch_session().await?;

    // 2. Make sure the files are present and verified.
    let mut vj = install_files(&app, &root, &version).await?;

    // Fabric runs from its own instance folder, which has its own mods folder.
    let game_dir = if fabric { instance_dir(&root, &version) } else { root.clone() };
    if fabric {
        if !fabric_installed(&root, &version) {
            install_fabric(&app, &root, &version).await?;
        }
        let profile: FabricProfile = read_json(&instance_dir(&root, &version).join(FABRIC_PROFILE))?;
        merge_fabric(&mut vj, profile);
    }

    // 3. Check Java.
    let java_exe = if java_path.trim().is_empty() {
        "java".to_string()
    } else {
        java_path.trim().to_string()
    };
    let found = java_major(&java_exe)?;
    let needed = vj.java_version.as_ref().map(|j| j.major_version).unwrap_or(8);
    if found < needed {
        return Err(format!(
            "This version needs Java {needed} or newer, but {java_exe} is Java {found}. Set the Java path in Settings."
        ));
    }

    // 4. Build the command line.
    let plan = plan_libraries(&root, &vj.libraries)?;
    let vdir = version_dir(&root, &version);
    let natives_dir = vdir.join("natives");
    let mut classpath = plan.classpath.clone();
    classpath.push(vdir.join(format!("{version}.jar")));
    let sep = if cfg!(windows) { ";" } else { ":" };
    let cp_string = classpath
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(sep);

    let assets_root = root.join("assets");
    let libraries_root = root.join("libraries");
    let mut subs: HashMap<&str, String> = HashMap::new();
    subs.insert("natives_directory", natives_dir.display().to_string());
    subs.insert("launcher_name", LAUNCHER_NAME.to_string());
    subs.insert("launcher_version", env!("CARGO_PKG_VERSION").to_string());
    subs.insert("classpath", cp_string.clone());
    subs.insert("classpath_separator", sep.to_string());
    subs.insert("library_directory", libraries_root.display().to_string());
    subs.insert("auth_player_name", session.username.clone());
    subs.insert("version_name", version.clone());
    subs.insert("game_directory", game_dir.display().to_string());
    subs.insert("assets_root", assets_root.display().to_string());
    subs.insert("assets_index_name", vj.asset_index.id.clone());
    subs.insert("auth_uuid", session.uuid.clone());
    subs.insert("auth_access_token", session.access_token.clone());
    subs.insert("clientid", String::new());
    subs.insert("auth_xuid", String::new());
    subs.insert("user_type", session.user_type.clone());
    subs.insert("version_type", "release".to_string());

    let mut args: Vec<String> = vec![format!("-Xmx{ram_mb}M")];
    match &vj.arguments {
        Some(a) => {
            for item in &a.jvm {
                push_arg(&mut args, item, &subs);
            }
        }
        None => {
            args.push(format!("-Djava.library.path={}", natives_dir.display()));
            args.push("-cp".to_string());
            args.push(cp_string.clone());
        }
    }
    args.push(vj.main_class.clone());
    match &vj.arguments {
        Some(a) => {
            for item in &a.game {
                push_arg(&mut args, item, &subs);
            }
        }
        None => {
            if let Some(legacy) = &vj.minecraft_arguments {
                for part in legacy.split_whitespace() {
                    args.push(substitute(part, &subs));
                }
            }
        }
    }

    // 5. Start the game with its output going to a log file.
    let logs_dir = game_dir.join("logs");
    fs::create_dir_all(&logs_dir)
        .map_err(|e| format!("Could not create {}: {e}", logs_dir.display()))?;
    let log_path = logs_dir.join("launcher-latest.log");
    let log = fs::File::create(&log_path)
        .map_err(|e| format!("Could not create {}: {e}", log_path.display()))?;
    let log_err = log
        .try_clone()
        .map_err(|e| format!("Could not open the log file: {e}"))?;

    let mut cmd = Command::new(&java_exe);
    cmd.args(&args)
        .current_dir(&game_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start Java ({java_exe}): {e}"))?;

    let watcher = app.clone();
    std::thread::spawn(move || {
        let code = child.wait().ok().and_then(|s| s.code());
        let _ = watcher.emit("game-exited", code);
    });

    Ok(format!("Minecraft {version} is starting. Log: {}", log_path.display()))
}

// ---------- Fabric loader ----------

const FABRIC_META: &str = "https://meta.fabricmc.net/v2/versions/loader";
const FABRIC_MARKER: &str = "fabric-install.json";
const FABRIC_PROFILE: &str = "fabric-profile.json";

#[derive(Deserialize)]
struct LoaderEntry {
    loader: LoaderVersion,
}

#[derive(Deserialize)]
struct LoaderVersion {
    version: String,
    stable: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FabricProfile {
    main_class: String,
    libraries: Vec<Library>,
    arguments: Option<Arguments>,
}

/// Maven coordinates ("group:artifact:version[:classifier]") to a relative jar path.
fn maven_path(name: &str) -> Option<String> {
    let parts: Vec<&str> = name.split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    let (group, artifact, version) = (parts[0], parts[1], parts[2]);
    let classifier = parts.get(3).map(|c| format!("-{c}")).unwrap_or_default();
    Some(format!(
        "{}/{artifact}/{version}/{artifact}-{version}{classifier}.jar",
        group.replace('.', "/")
    ))
}

fn maven_url(base: &str, rel: &str) -> String {
    if base.ends_with('/') {
        format!("{base}{rel}")
    } else {
        format!("{base}/{rel}")
    }
}

/// Fabric's Maven does not list hashes in the profile, so read each library's .sha1 file.
async fn fill_missing_hashes(client: &reqwest::Client, libs: &mut [Library]) -> Result<(), String> {
    for lib in libs.iter_mut() {
        if lib.downloads.is_some() || lib.sha1.is_some() {
            continue;
        }
        let (Some(base), Some(rel)) = (lib.url.as_deref(), maven_path(&lib.name)) else {
            continue;
        };
        let url = format!("{}.sha1", maven_url(base, &rel));
        let text = fetch_bytes(client, &url).await?;
        lib.sha1 = Some(String::from_utf8_lossy(&text).trim().to_string());
    }
    Ok(())
}

/// Each Fabric instance is a separate game folder: mods, saves, config, and logs live here.
fn instance_dir(root: &Path, version: &str) -> PathBuf {
    root.join("instances").join(format!("fabric-{version}"))
}

pub fn fabric_installed(root: &Path, version: &str) -> bool {
    instance_dir(root, version).join(FABRIC_MARKER).is_file()
}

/// Installs the vanilla files, then the latest stable Fabric loader for that version.
pub async fn install_fabric(app: &AppHandle, root: &Path, version: &str) -> Result<(), String> {
    check_version_id(version)?;
    install_files(app, root, version).await?;

    let client = http_client()?;
    let dir = instance_dir(root, version);
    fs::create_dir_all(dir.join("mods"))
        .map_err(|e| format!("Could not create {}: {e}", dir.display()))?;

    emit(app, "Fabric", 0, 1, "Finding the latest Fabric loader");
    let loaders: Vec<LoaderEntry> = fetch_json(&client, &format!("{FABRIC_META}/{version}")).await?;
    let loader_version = loaders
        .iter()
        .find(|l| l.loader.stable)
        .or_else(|| loaders.first())
        .ok_or_else(|| format!("Fabric does not support Minecraft {version} yet."))?
        .loader
        .version
        .clone();

    emit(app, "Fabric", 0, 1, "Downloading the Fabric profile");
    let profile_bytes =
        fetch_bytes(&client, &format!("{FABRIC_META}/{version}/{loader_version}/profile/json")).await?;
    let mut profile: FabricProfile = serde_json::from_slice(&profile_bytes)
        .map_err(|e| format!("Unexpected Fabric profile data: {e}"))?;
    fs::write(dir.join(FABRIC_PROFILE), &profile_bytes)
        .map_err(|e| format!("Could not save the Fabric profile: {e}"))?;

    fill_missing_hashes(&client, &mut profile.libraries).await?;
    let plan = plan_libraries(root, &profile.libraries)?;
    download_all(app, &client, "Fabric libraries", plan.tasks).await?;

    fs::write(
        dir.join(FABRIC_MARKER),
        serde_json::json!({ "loader": loader_version }).to_string(),
    )
    .map_err(|e| format!("Could not finish the Fabric install: {e}"))?;
    emit(app, "Ready", 1, 1, "Fabric is installed");
    Ok(())
}

/// Puts the Fabric entry point and libraries on top of the vanilla version.
fn merge_fabric(vj: &mut VersionJson, profile: FabricProfile) {
    vj.main_class = profile.main_class;
    let mut libs = profile.libraries;
    libs.append(&mut vj.libraries);
    vj.libraries = libs;
    if let Some(extra) = profile.arguments {
        if let Some(args) = vj.arguments.as_mut() {
            args.jvm.extend(extra.jvm);
            args.game.extend(extra.game);
        } else {
            vj.arguments = Some(extra);
        }
    }
}

// ---------- Mods ----------

fn mods_dir(root: &Path, version: &str) -> PathBuf {
    instance_dir(root, version).join("mods")
}

/// Only plain file names ending in .jar or .jar.disabled, so nothing can point outside the mods folder.
fn check_mod_file_name(name: &str) -> Result<(), String> {
    let lower = name.to_lowercase();
    let ok = !name.is_empty()
        && name.len() <= 200
        && !name.chars().any(|c| c == '/' || c == '\\')
        && !name.contains("..")
        && (lower.ends_with(".jar") || lower.ends_with(".jar.disabled"));
    if ok {
        Ok(())
    } else {
        Err(format!("Not a mod file name: {name}"))
    }
}

/// Reads "name" and "version" from the mod's fabric.mod.json, if it has one.
fn read_mod_info(jar: &Path) -> (String, String) {
    let fallback = (String::new(), String::new());
    let Ok(file) = fs::File::open(jar) else {
        return fallback;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return fallback;
    };
    let Ok(mut entry) = archive.by_name("fabric.mod.json") else {
        return fallback;
    };
    let mut text = String::new();
    if entry.read_to_string(&mut text).is_err() {
        return fallback;
    }
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return fallback;
    };
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| v.get("id").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    let version = v.get("version").and_then(Value::as_str).unwrap_or("").to_string();
    (name, version)
}

fn is_fabric_mod(jar: &Path) -> bool {
    let Ok(file) = fs::File::open(jar) else {
        return false;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return false;
    };
    // Bind the result first, so the borrow of `archive` ends before `archive` is dropped.
    let found = archive.by_name("fabric.mod.json").is_ok();
    found
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModEntry {
    pub file_name: String,
    pub enabled: bool,
    pub name: String,
    pub mod_version: String,
    pub size_kb: u64,
}

pub fn list_mods(root: &Path, version: &str) -> Result<Vec<ModEntry>, String> {
    check_version_id(version)?;
    let dir = mods_dir(root, version);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| format!("Could not read the mods folder: {e}"))? {
        let entry = entry.map_err(|e| format!("Could not read the mods folder: {e}"))?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        let lower = file_name.to_lowercase();
        let enabled = if lower.ends_with(".jar") {
            true
        } else if lower.ends_with(".jar.disabled") {
            false
        } else {
            continue;
        };
        let size_kb = entry.metadata().map(|m| m.len() / 1024).unwrap_or(0);
        let (name, mod_version) = read_mod_info(&entry.path());
        out.push(ModEntry {
            file_name,
            enabled,
            name,
            mod_version,
            size_kb,
        });
    }
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.file_name.cmp(&b.file_name))
    });
    Ok(out)
}

/// Copies Fabric mod .jar files into the instance's mods folder. Returns how many were added.
pub fn add_mods(root: &Path, version: &str, sources: &[String]) -> Result<usize, String> {
    check_version_id(version)?;
    let dir = mods_dir(root, version);
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create the mods folder: {e}"))?;

    let mut added = 0usize;
    for source in sources {
        let src = Path::new(source);
        let file_name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if !file_name.to_lowercase().ends_with(".jar") {
            return Err(format!("{file_name} is not a .jar file. Only Fabric mod .jar files can be added."));
        }
        check_mod_file_name(&file_name)?;
        if !is_fabric_mod(src) {
            return Err(format!(
                "{file_name} is not a Fabric mod: it has no fabric.mod.json. Check that it is built for Fabric."
            ));
        }
        fs::copy(src, dir.join(&file_name))
            .map_err(|e| format!("Could not copy {file_name}: {e}"))?;
        added += 1;
    }
    Ok(added)
}

/// Enables a mod by removing the ".disabled" suffix, or disables it by adding the suffix.
pub fn set_mod_enabled(root: &Path, version: &str, file_name: &str, enabled: bool) -> Result<(), String> {
    check_version_id(version)?;
    check_mod_file_name(file_name)?;
    let dir = mods_dir(root, version);
    let current = dir.join(file_name);
    let base = file_name.strip_suffix(".disabled").unwrap_or(file_name);
    let target_name = if enabled {
        base.to_string()
    } else {
        format!("{base}.disabled")
    };
    if target_name == file_name {
        return Ok(());
    }
    if !current.is_file() {
        return Err(format!("{file_name} was not found."));
    }
    if dir.join(&target_name).exists() {
        return Err(format!("{target_name} already exists."));
    }
    fs::rename(&current, dir.join(&target_name))
        .map_err(|e| format!("Could not change {file_name}: {e}"))
}

pub fn remove_mod(root: &Path, version: &str, file_name: &str) -> Result<(), String> {
    check_version_id(version)?;
    check_mod_file_name(file_name)?;
    let path = mods_dir(root, version).join(file_name);
    if !path.is_file() {
        return Err(format!("{file_name} was not found."));
    }
    fs::remove_file(&path).map_err(|e| format!("Could not remove {file_name}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_major_parses_modern_and_legacy_output() {
        assert_eq!(parse_java_major("openjdk version \"21.0.1\" 2023-10-17"), Some(21));
        assert_eq!(parse_java_major("java version \"1.8.0_381\""), Some(8));
        assert_eq!(parse_java_major("no version here"), None);
    }

    #[test]
    fn version_names_are_restricted() {
        assert!(check_version_id("1.21.4").is_ok());
        assert!(check_version_id("../evil").is_err());
        assert!(check_version_id("").is_err());
    }
}
