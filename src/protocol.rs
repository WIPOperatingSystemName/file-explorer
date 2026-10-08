//! Stable JSON protocol shared by Telorgon clients and the portal backend.
//! `uris` preserve the actual Unix path bytes; `paths` are display strings.
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

pub const MAX_PAYLOAD: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PickerMode {
    #[default]
    Open,
    Save,
    Folder,
    SaveFiles,
    Download,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterRule {
    /// XDG filter type: 0 is a filename glob; 1 is a MIME type.
    pub kind: u32,
    pub pattern: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFilter {
    pub name: String,
    pub rules: Vec<FilterRule>,
}

impl FileFilter {
    pub fn matches(&self, path: &Path) -> bool {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        self.rules.is_empty()
            || self.rules.iter().any(|rule| match rule.kind {
                0 => glob_matches(&rule.pattern, &name),
                1 => mime_matches(&rule.pattern, path),
                _ => false,
            })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickerChoice {
    pub id: String,
    pub label: String,
    pub options: Vec<(String, String)>,
    pub selected: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PickerRequest {
    pub mode: PickerMode,
    pub title: String,
    pub app_id: String,
    pub parent_window: String,
    pub multiple: bool,
    pub directory: bool,
    pub modal: bool,
    pub accept_label: Option<String>,
    pub current_folder: Option<String>,
    pub current_folder_bytes: Option<Vec<u8>>,
    pub current_name: Option<String>,
    pub current_file: Option<String>,
    pub current_file_bytes: Option<Vec<u8>>,
    pub filters: Vec<FileFilter>,
    pub current_filter: Option<FileFilter>,
    pub choices: Vec<PickerChoice>,
    pub files: Vec<String>,
    pub files_bytes: Vec<Vec<u8>>,
    pub source_url: Option<String>,
}

impl Default for PickerRequest {
    fn default() -> Self {
        Self {
            mode: PickerMode::Open,
            title: "Open a file".into(),
            app_id: String::new(),
            parent_window: String::new(),
            multiple: false,
            directory: false,
            modal: true,
            accept_label: None,
            current_folder: None,
            current_folder_bytes: None,
            current_name: None,
            current_file: None,
            current_file_bytes: None,
            filters: Vec::new(),
            current_filter: None,
            choices: Vec::new(),
            files: Vec::new(),
            files_bytes: Vec::new(),
            source_url: None,
        }
    }
}

impl PickerRequest {
    pub fn folder_path(&self) -> Option<PathBuf> {
        raw_path(
            self.current_folder_bytes.as_deref(),
            self.current_folder.as_deref(),
        )
    }

    pub fn file_path(&self) -> Option<PathBuf> {
        raw_path(
            self.current_file_bytes.as_deref(),
            self.current_file.as_deref(),
        )
    }

    pub fn initial_path(&self) -> Option<PathBuf> {
        self.folder_path().or_else(|| {
            self.file_path()
                .and_then(|p| p.parent().map(Path::to_path_buf))
        })
    }

    pub fn selects_folders(&self) -> bool {
        self.directory || matches!(self.mode, PickerMode::Folder | PickerMode::SaveFiles)
    }

    pub fn is_save(&self) -> bool {
        matches!(self.mode, PickerMode::Save | PickerMode::Download)
    }

    pub fn matches_path(&self, path: &Path) -> bool {
        path.is_dir()
            || self
                .current_filter
                .as_ref()
                .or_else(|| self.filters.first())
                .is_none_or(|f| f.matches(path))
    }

    pub fn validate_selection(&self, paths: &[PathBuf]) -> io::Result<()> {
        let allows_multiple =
            self.multiple && matches!(self.mode, PickerMode::Open | PickerMode::Folder);
        if paths.is_empty() || paths.len() > 4096 || (!allows_multiple && paths.len() != 1) {
            return invalid("Select one item, or enable multiple selection.");
        }
        for path in paths {
            if !path.is_absolute() || path.as_os_str().as_bytes().contains(&0) {
                return invalid("Selection must be an absolute local path without NUL bytes.");
            }
            if self.selects_folders() {
                if !path.is_dir() {
                    return invalid("Select an existing folder.");
                }
            } else if self.is_save() {
                let name = path
                    .file_name()
                    .ok_or_else(|| invalid_error("Choose a filename."))?;
                if name.is_empty() || !path.parent().is_some_and(Path::is_dir) || path.is_dir() {
                    return invalid("Choose a filename in an existing folder.");
                }
            } else if !path.exists() || path.is_dir() {
                return invalid("Select an existing file.");
            }
            if !self.selects_folders() && !self.matches_path(path) {
                return invalid("Selection does not match the selected file filter.");
            }
        }
        Ok(())
    }

    pub fn validate(&self) -> io::Result<()> {
        if self.filters.len() > 128
            || self.choices.len() > 64
            || self.files.len().max(self.files_bytes.len()) > 4096
        {
            return invalid("Picker request contains too many filters, choices, or files.");
        }
        if self
            .filters
            .iter()
            .chain(self.current_filter.iter())
            .any(|f| f.rules.len() > 128 || f.rules.iter().any(|r| r.kind > 1))
        {
            return invalid("Invalid file filter.");
        }
        let mut choice_ids = std::collections::HashSet::new();
        for choice in &self.choices {
            let mut option_ids = std::collections::HashSet::new();
            if choice.id.is_empty()
                || choice.options.len() > 128
                || !choice_ids.insert(&choice.id)
                || choice
                    .options
                    .iter()
                    .any(|(id, _)| id.is_empty() || !option_ids.insert(id))
            {
                return invalid("Invalid picker choice.");
            }
            if choice.options.is_empty() {
                if !matches!(choice.selected.as_str(), "true" | "false" | "") {
                    return invalid("Invalid boolean choice.");
                }
            } else if !choice.selected.is_empty()
                && !choice.options.iter().any(|(id, _)| id == &choice.selected)
            {
                return invalid("Choice default is not an available option.");
            }
        }
        for raw in self
            .current_folder_bytes
            .iter()
            .chain(self.current_file_bytes.iter())
        {
            if raw.contains(&0) {
                return invalid("Raw paths must not contain NUL bytes.");
            }
        }
        if self.mode == PickerMode::SaveFiles {
            let names = self.save_names()?;
            if names.is_empty() {
                return invalid("SaveFiles requires at least one filename.");
            }
        }
        Ok(())
    }

    fn save_names(&self) -> io::Result<Vec<OsString>> {
        let names = if self.files_bytes.is_empty() {
            self.files.iter().map(OsString::from).collect::<Vec<_>>()
        } else {
            self.files_bytes
                .iter()
                .cloned()
                .map(OsString::from_vec)
                .collect()
        };
        for name in &names {
            let bytes = name.as_bytes();
            if bytes.is_empty()
                || bytes.contains(&0)
                || bytes.contains(&b'/')
                || bytes == b"."
                || bytes == b".."
            {
                return invalid("SaveFiles names must be plain filenames.");
            }
        }
        Ok(names)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PickerResponse {
    pub cancelled: bool,
    pub uris: Vec<String>,
    pub paths: Vec<String>,
    pub writable: bool,
    pub current_filter: Option<FileFilter>,
    pub choices: Vec<(String, String)>,
}

impl PickerResponse {
    pub fn cancelled() -> Self {
        Self {
            cancelled: true,
            ..Self::default()
        }
    }
}

pub fn read_request(reader: impl Read) -> io::Result<PickerRequest> {
    let bytes = read_bounded(reader)?;
    let request: PickerRequest = serde_json::from_slice(&bytes).map_err(invalid_error)?;
    request.validate()?;
    Ok(request)
}

pub fn read_response(reader: impl Read) -> io::Result<PickerResponse> {
    serde_json::from_slice(&read_bounded(reader)?).map_err(invalid_error)
}

pub fn write_response(mut writer: impl Write, response: &PickerResponse) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(response).map_err(invalid_error)?;
    if bytes.len() > MAX_PAYLOAD {
        return invalid("Picker response is too large.");
    }
    bytes.push(b'\n');
    writer.write_all(&bytes)?;
    writer.flush()
}

pub fn selected_response(request: &PickerRequest, paths: &[PathBuf]) -> io::Result<PickerResponse> {
    request.validate_selection(paths)?;
    let paths = if request.mode == PickerMode::SaveFiles {
        let mut destinations = Vec::new();
        for name in request.save_names()? {
            let destination = unique_destination(&paths[0], &name, &destinations)?;
            destinations.push(destination);
        }
        destinations
    } else {
        paths.to_vec()
    };
    Ok(PickerResponse {
        cancelled: false,
        uris: paths
            .iter()
            .map(|p| path_to_uri(p))
            .collect::<io::Result<_>>()?,
        paths: paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        writable: request.is_save() || request.mode == PickerMode::SaveFiles,
        current_filter: request
            .current_filter
            .clone()
            .or_else(|| request.filters.first().cloned()),
        choices: request
            .choices
            .iter()
            .map(|c| {
                let value = if c.selected.is_empty() {
                    c.options
                        .first()
                        .map(|(id, _)| id.as_str())
                        .unwrap_or("false")
                } else {
                    &c.selected
                };
                (c.id.clone(), value.to_owned())
            })
            .collect(),
    })
}

pub fn path_to_uri(path: &Path) -> io::Result<String> {
    if !path.is_absolute() || path.as_os_str().as_bytes().contains(&0) {
        return invalid("Expected an absolute local path.");
    }
    let mut uri = String::from("file://");
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
            uri.push(byte as char);
        } else {
            uri.push('%');
            uri.push(HEX[(byte >> 4) as usize] as char);
            uri.push(HEX[(byte & 15) as usize] as char);
        }
    }
    Ok(uri)
}

pub fn uri_to_path(uri: &str) -> io::Result<PathBuf> {
    let rest = uri
        .strip_prefix("file://")
        .ok_or_else(|| invalid_error("Only local file:// URIs are supported."))?;
    let rest = rest
        .strip_prefix("localhost/")
        .map(|p| format!("/{p}"))
        .unwrap_or_else(|| rest.to_owned());
    if !rest.starts_with('/') || rest.contains(['?', '#']) {
        return invalid("Expected a local file URI without query or fragment.");
    }
    let mut out = Vec::with_capacity(rest.len());
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return invalid("Invalid URI escape.");
            }
            let hi = hex(bytes[i + 1]).ok_or_else(|| invalid_error("Invalid URI escape."))?;
            let lo = hex(bytes[i + 2]).ok_or_else(|| invalid_error("Invalid URI escape."))?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    if out.contains(&0) {
        return invalid("A file URI cannot contain NUL bytes.");
    }
    Ok(PathBuf::from(OsString::from_vec(out)))
}

fn raw_path(bytes: Option<&[u8]>, display: Option<&str>) -> Option<PathBuf> {
    bytes
        .map(|p| PathBuf::from(OsString::from_vec(p.to_vec())))
        .or_else(|| display.map(PathBuf::from))
}

fn read_bounded(reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_PAYLOAD + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PAYLOAD {
        return invalid("Picker JSON exceeds the 256 KiB limit.");
    }
    Ok(bytes)
}

fn invalid<T>(message: impl ToString) -> io::Result<T> {
    Err(invalid_error(message))
}
fn invalid_error(message: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}
fn hex(byte: u8) -> Option<u8> {
    (byte as char).to_digit(16).map(|n| n as u8)
}

fn unique_destination(
    folder: &Path,
    name: &std::ffi::OsStr,
    previous: &[PathBuf],
) -> io::Result<PathBuf> {
    let path = folder.join(name);
    if !previous.contains(&path) && destination_available(&path)? {
        return Ok(path);
    }
    let stem = Path::new(name).file_stem().unwrap_or(name);
    let extension = Path::new(name).extension();
    for index in 1_u64.. {
        let mut next = stem.to_os_string();
        next.push(format!(" ({index})"));
        if let Some(extension) = extension {
            next.push(".");
            next.push(extension);
        }
        let candidate = folder.join(next);
        if !previous.contains(&candidate) && destination_available(&candidate)? {
            return Ok(candidate);
        }
    }
    unreachable!()
}

fn destination_available(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        // An inaccessible path is not evidence that a destination is unused.
        Err(error) => Err(error),
    }
}

// Glob matching is deliberately bounded O(pattern * filename), with no recursion.
fn glob_matches(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let mut previous = vec![false; n.len() + 1];
    previous[0] = true;
    let mut i = 0;
    while i < p.len() {
        let mut next = vec![false; n.len() + 1];
        if p[i] == '*' {
            next[0] = previous[0];
            for j in 1..=n.len() {
                next[j] = previous[j] || next[j - 1];
            }
            i += 1;
        } else if p[i] == '[' {
            if let Some(close) = p[i + 1..].iter().position(|c| *c == ']') {
                let end = i + 1 + close;
                let set = &p[i + 1..end];
                for j in 1..=n.len() {
                    next[j] = previous[j - 1] && bracket_matches(set, n[j - 1]);
                }
                i = end + 1;
            } else {
                for j in 1..=n.len() {
                    next[j] = previous[j - 1] && n[j - 1] == '[';
                }
                i += 1;
            }
        } else {
            let literal = if p[i] == '\\' && i + 1 < p.len() {
                i += 1;
                p[i]
            } else {
                p[i]
            };
            for j in 1..=n.len() {
                next[j] = previous[j - 1] && (literal == '?' || literal == n[j - 1]);
            }
            i += 1;
        }
        previous = next;
    }
    previous[n.len()]
}

fn bracket_matches(set: &[char], character: char) -> bool {
    let negated = set.first().is_some_and(|c| matches!(c, '!' | '^'));
    let set = if negated { &set[1..] } else { set };
    let mut matched = false;
    let mut i = 0;
    while i < set.len() {
        if i + 2 < set.len() && set[i + 1] == '-' {
            matched |= set[i] <= character && character <= set[i + 2];
            i += 3;
        } else {
            matched |= set[i] == character;
            i += 1;
        }
    }
    matched != negated
}

fn mime_matches(pattern: &str, path: &Path) -> bool {
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let mime = match extension.as_str() {
        "txt" | "log" | "md" | "rs" | "c" | "h" | "py" | "sh" | "toml" | "yaml" | "yml" => {
            "text/plain"
        }
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "csv" => "text/csv",
        "json" => "application/json",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "mp3" => "audio/mpeg",
        "wav" => "audio/x-wav",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" => "application/gzip",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    };
    pattern == mime
        || pattern
            .strip_suffix("/*")
            .is_some_and(|prefix| mime.starts_with(&format!("{prefix}/")))
        || pattern == "*/*"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uri_roundtrip_preserves_non_utf8_and_reserved_bytes() {
        let path = PathBuf::from(OsString::from_vec(b"/tmp/sp ace#%\xff".to_vec()));
        let uri = path_to_uri(&path).unwrap();
        assert_eq!(uri, "file:///tmp/sp%20ace%23%25%FF");
        assert_eq!(uri_to_path(&uri).unwrap(), path);
    }
    #[test]
    fn reject_remote_or_malformed_file_uris() {
        for uri in [
            "https://example.org/a",
            "file://server/a",
            "file:///a%00",
            "file:///a%XX",
            "file:///a#b",
        ] {
            assert!(uri_to_path(uri).is_err(), "{uri}");
        }
        assert_eq!(
            uri_to_path("file://localhost/tmp/a").unwrap(),
            PathBuf::from("/tmp/a")
        );
    }
    #[test]
    fn patterns_cover_wildcards_and_brackets() {
        assert!(glob_matches("*.rs", "main.rs"));
        assert!(glob_matches("photo[0-9]?.jpg", "photo42.jpg"));
        assert!(!glob_matches("[!a]*.png", "abc.png"));
        assert!(glob_matches("file?.txt", "fileé.txt"));
        assert!(!glob_matches("*.txt", "file.txt.pdf"));
    }
    #[test]
    fn oversized_and_invalid_requests_fail() {
        assert!(read_request(vec![b' '; MAX_PAYLOAD + 1].as_slice()).is_err());
        assert!(
            read_request(br#"{"mode":"save_files","files":["../unsafe"]}"#.as_slice()).is_err()
        );
        assert!(
            read_request(
                br#"{"filters":[{"name":"Bad","rules":[{"kind":8,"pattern":"*"}]}]}"#.as_slice()
            )
            .is_err()
        );
    }
    #[test]
    fn save_files_are_unique_and_ordered() {
        let folder = std::env::temp_dir().join(format!("telorgon-protocol-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("report.txt"), b"existing").unwrap();
        let request = PickerRequest {
            mode: PickerMode::SaveFiles,
            files: vec!["report.txt".into(), "report.txt".into()],
            ..Default::default()
        };
        let response = selected_response(&request, &[folder.clone()]).unwrap();
        assert!(response.paths[0].ends_with("report (1).txt"));
        assert!(response.paths[1].ends_with("report (2).txt"));
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn save_files_preserve_existing_dangling_symlinks() {
        let folder =
            std::env::temp_dir().join(format!("telorgon-protocol-dangling-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let existing = folder.join("report.txt");
        std::os::unix::fs::symlink("missing-target", &existing).unwrap();
        assert!(!existing.exists());
        let request = PickerRequest {
            mode: PickerMode::SaveFiles,
            files: vec!["report.txt".into()],
            ..Default::default()
        };
        let response = selected_response(&request, &[folder.clone()]).unwrap();
        assert_eq!(
            response.uris,
            vec![path_to_uri(&folder.join("report (1).txt")).unwrap()]
        );
        assert_eq!(
            std::fs::read_link(existing).unwrap(),
            PathBuf::from("missing-target")
        );
        std::fs::remove_dir_all(folder).unwrap();
    }
}
