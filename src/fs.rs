//! Filesystem operations shared by the explorer, picker, and download dialogs.
//!
//! Transfers deliberately do not replace existing entries. Symlinks are copied
//! as links, and recursive operations never follow them.

mod operations;
mod search;
mod trash;

pub use operations::{TransferMode, create_folder, rename_entry, transfer_entries, validate_name};
pub use search::{SearchStatus, SearchTask};
pub use trash::{TrashItem, list_trash, restore_trash, trash_entries};

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub kind: String,
}

impl Entry {
    pub fn is_hidden(&self) -> bool {
        self.name.starts_with('.')
    }
}

pub fn list_directory(path: &Path) -> Result<Vec<Entry>, String> {
    let directory =
        fs::read_dir(path).map_err(|error| format!("Cannot open {}: {error}", path.display()))?;
    let mut entries = Vec::new();
    for child in directory {
        let child = child.map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
        match entry_for_path(&child.path()) {
            Ok(entry) => entries.push(entry),
            // A concurrent rename or deletion should not prevent browsing.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Cannot inspect {}: {error}",
                    child.path().display()
                ));
            }
        }
    }
    Ok(entries)
}

pub(crate) fn entry_for_path(path: &Path) -> std::io::Result<Entry> {
    let metadata = fs::symlink_metadata(path)?;
    let is_symlink = metadata.file_type().is_symlink();
    let target_metadata = if is_symlink {
        fs::metadata(path).ok()
    } else {
        None
    };
    let is_dir = if is_symlink {
        target_metadata.as_ref().is_some_and(fs::Metadata::is_dir)
    } else {
        metadata.is_dir()
    };
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned();
    let kind = if is_symlink {
        if target_metadata.is_none() {
            "Broken link".to_string()
        } else if is_dir {
            "Link to folder".to_string()
        } else {
            "Symbolic link".to_string()
        }
    } else if is_dir {
        "Folder".to_string()
    } else if !metadata.is_file() {
        "Special file".to_string()
    } else {
        file_kind(path)
    };
    Ok(Entry {
        path: path.to_path_buf(),
        name,
        is_dir,
        is_symlink,
        size: if is_dir { 0 } else { metadata.len() },
        modified: metadata.modified().ok(),
        kind,
    })
}

fn file_kind(path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    let label = match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif" | "tiff" => "Image",
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "m4v" => "Video",
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" => "Audio",
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst" => "Archive",
        "pdf" => "PDF document",
        "txt" | "md" | "rst" | "log" => "Text document",
        "doc" | "docx" | "odt" | "rtf" => "Document",
        "xls" | "xlsx" | "ods" | "csv" | "tsv" => "Spreadsheet",
        "ppt" | "pptx" | "odp" => "Presentation",
        "rs" | "c" | "h" | "cpp" | "hpp" | "py" | "js" | "ts" | "html" | "css" | "sh" => {
            "Source code"
        }
        "json" | "toml" | "yaml" | "yml" | "xml" | "ini" | "conf" => "Configuration",
        "desktop" => "Application shortcut",
        "" => "File",
        _ => return format!("{} file", extension.to_uppercase()),
    };
    label.to_string()
}

/// Match shell-style filename filters used by file-picker requests.
/// `*` matches any sequence and `?` matches one character, case-insensitively.
pub fn matches_pattern(name: &str, pattern: &str) -> bool {
    let name: Vec<char> = name.to_lowercase().chars().collect();
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let (mut input, mut token, mut star, mut retry) = (0, 0, None, 0);
    while input < name.len() {
        if token < pattern.len() && (pattern[token] == '?' || pattern[token] == name[input]) {
            input += 1;
            token += 1;
        } else if token < pattern.len() && pattern[token] == '*' {
            star = Some(token);
            token += 1;
            retry = input;
        } else if let Some(star_token) = star {
            retry += 1;
            input = retry;
            token = star_token + 1;
        } else {
            return false;
        }
    }
    while token < pattern.len() && pattern[token] == '*' {
        token += 1;
    }
    token == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_patterns_accept_extensions_and_unicode() {
        assert!(matches_pattern("Photo.PNG", "*.png"));
        assert!(matches_pattern("café.txt", "caf?.txt"));
        assert!(matches_pattern("README", "*"));
        assert!(matches_pattern("a.txt", "*a*.*"));
        assert!(!matches_pattern("image.jpeg", "*.png"));
        assert!(!matches_pattern("", "?"));
    }
}
