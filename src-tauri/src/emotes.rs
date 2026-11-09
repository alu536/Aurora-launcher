//! Emote library for Emotecraft (.emotecraft) files.
//!
//! What this module reads, all from the file itself:
//! - the body-part names the animation uses (head, body, arms, legs),
//! - the name, description and author, stored as length-prefixed quoted strings,
//! - the embedded PNG icon.
//!
//! Playback is NOT implemented here. Emotes play in the game through the in-game client,
//! which is a later milestone. The file layout beyond these fields is not documented in
//! this project, so nothing else is interpreted.

#![allow(dead_code)]

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_EMOTE_BYTES: usize = 10 * 1024 * 1024;
const KNOWN_BONES: [&str; 6] = ["head", "body", "rightArm", "leftArm", "rightLeg", "leftLeg"];
const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
const PNG_END: &[u8] = b"IEND\xaeB`\x82";

/// What is saved next to each imported emote in meta.json.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EmoteMeta {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub bones: Vec<String>,
    pub size_kb: u64,
    pub source_file: String,
}

/// What the UI receives: the saved metadata plus the icon as a data URL.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmoteEntry {
    #[serde(flatten)]
    pub meta: EmoteMeta,
    pub icon: Option<String>,
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Locates the embedded PNG icon, if present. Returns its byte range.
fn png_range(data: &[u8]) -> Option<(usize, usize)> {
    let start = find(data, PNG_SIG)?;
    let end = find(&data[start..], PNG_END)? + start + PNG_END.len();
    Some((start, end))
}

/// Reads the first three length-prefixed quoted strings: name, description, author.
/// Each is a 4-byte big-endian length (counting the quote marks), then the quoted text.
fn read_quoted_fields(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 4 < data.len() && out.len() < 3 {
        let len = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        if (3..=300).contains(&len)
            && i + 4 + len <= data.len()
            && data[i + 4] == b'"'
            && data[i + 4 + len - 1] == b'"'
        {
            let inner = &data[i + 5..i + 4 + len - 1];
            if inner.iter().all(|&c| (0x20..0x7f).contains(&c)) {
                out.push(String::from_utf8_lossy(inner).to_string());
                i += 4 + len;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn slug(file_name: &str) -> String {
    let s: String = file_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(40)
        .collect();
    if s.is_empty() {
        "emote".to_string()
    } else {
        s
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Copies .emotecraft files into the library. Each emote gets its own folder with the
/// original file, its icon, and meta.json. Returns the metadata of what was added.
pub fn import_files(library: &Path, paths: &[String]) -> Result<Vec<EmoteMeta>, String> {
    fs::create_dir_all(library)
        .map_err(|e| format!("Could not create the emote library {}: {e}", library.display()))?;

    let mut added = Vec::new();
    for source in paths {
        let src = Path::new(source);
        let file_name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if !file_name.to_lowercase().ends_with(".emotecraft") {
            return Err(format!("{file_name} is not an .emotecraft file."));
        }

        let data = fs::read(src).map_err(|e| format!("Could not read {file_name}: {e}"))?;
        if data.len() > MAX_EMOTE_BYTES {
            return Err(format!("{file_name} is larger than 10 MB, so it was not imported."));
        }

        // Body-part names must appear before the icon. Searching only that part avoids
        // false matches inside the compressed PNG data.
        let icon = png_range(&data);
        let text_area = match icon {
            Some((start, _)) => &data[..start],
            None => &data[..],
        };
        let bones: Vec<String> = KNOWN_BONES
            .iter()
            .filter(|b| contains(text_area, b.as_bytes()))
            .map(|b| b.to_string())
            .collect();
        let fields = read_quoted_fields(&data);
        if bones.is_empty() || fields.is_empty() {
            return Err(format!(
                "{file_name} does not look like an Emotecraft emote: the body-part names or the emote name were not found."
            ));
        }

        let id = format!("{}-{}", now_millis(), slug(&file_name));
        let dir = library.join(&id);
        fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
        fs::write(dir.join("emote.emotecraft"), &data)
            .map_err(|e| format!("Could not save {file_name}: {e}"))?;
        if let Some((start, end)) = icon {
            fs::write(dir.join("icon.png"), &data[start..end])
                .map_err(|e| format!("Could not save the icon for {file_name}: {e}"))?;
        }

        let meta = EmoteMeta {
            id,
            name: fields.first().cloned().unwrap_or_else(|| file_name.clone()),
            description: fields.get(1).cloned().unwrap_or_default(),
            author: fields.get(2).cloned().unwrap_or_default(),
            bones,
            size_kb: (data.len() / 1024) as u64,
            source_file: file_name.clone(),
        };
        let json = serde_json::to_string_pretty(&meta)
            .map_err(|e| format!("Could not save metadata for {file_name}: {e}"))?;
        fs::write(dir.join("meta.json"), json)
            .map_err(|e| format!("Could not save metadata for {file_name}: {e}"))?;
        added.push(meta);
    }
    Ok(added)
}

pub fn list(library: &Path) -> Result<Vec<EmoteEntry>, String> {
    if !library.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(library).map_err(|e| format!("Could not read the emote library: {e}"))? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let Ok(text) = fs::read_to_string(path.join("meta.json")) else {
            continue;
        };
        let Ok(meta) = serde_json::from_str::<EmoteMeta>(&text) else {
            continue;
        };
        let icon = fs::read(path.join("icon.png")).ok().map(|bytes| {
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )
        });
        out.push(EmoteEntry { meta, icon });
    }
    out.sort_by(|a, b| a.meta.name.to_lowercase().cmp(&b.meta.name.to_lowercase()));
    Ok(out)
}

/// Removes an emote folder. The id is checked so it can't point outside the library.
pub fn remove(library: &Path, id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= 120
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err("Invalid emote id.".to_string());
    }
    let dir = library.join(id);
    if !dir.is_dir() {
        return Err("That emote was not found.".to_string());
    }
    fs::remove_dir_all(&dir).map_err(|e| format!("Could not remove the emote: {e}"))
}
