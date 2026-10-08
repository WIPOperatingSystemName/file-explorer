//! Per-user preferences and bookmarks stored in the XDG configuration directory.

use serde::{Deserialize, Serialize};
use std::ffi::CStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Preferences {
    pub bookmarks: Vec<PathBuf>,
    pub show_hidden: bool,
    pub view_mode: String,
    pub sort_by: String,
    pub descending: bool,
    pub last_directory: Option<PathBuf>,
}

impl Default for Preferences {
    fn default() -> Self {
        let home = home_directory();
        let directories = user_directories(&home);
        Self {
            bookmarks: directories
                .into_iter()
                .filter(|path| path.is_dir())
                .collect(),
            show_hidden: false,
            view_mode: "list".to_string(),
            sort_by: "name".to_string(),
            descending: false,
            last_directory: None,
        }
    }
}

impl Preferences {
    pub fn load() -> Self {
        config_file()
            .ok()
            .and_then(|path| Self::load_from(&path).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&config_file()?)
    }

    fn load_from(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|error| format!("Cannot read preferences: {error}"))?;
        let mut preferences: Self =
            serde_json::from_str(&text).map_err(|error| format!("Invalid preferences: {error}"))?;
        preferences.normalize();
        Ok(preferences)
    }

    fn normalize(&mut self) {
        if self.sort_by == "kind" {
            self.sort_by = "type".to_string();
        }
        if !["list", "grid"].contains(&self.view_mode.as_str()) {
            self.view_mode = "list".to_string();
        }
        if !["name", "size", "modified", "type"].contains(&self.sort_by.as_str()) {
            self.sort_by = "name".to_string();
        }
        let mut unique = Vec::new();
        for bookmark in self.bookmarks.drain(..) {
            if bookmark.is_absolute() && !unique.contains(&bookmark) {
                unique.push(bookmark);
            }
        }
        self.bookmarks = unique;
        if self
            .last_directory
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
        {
            self.last_directory = None;
        }
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| "Invalid preferences path".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("Cannot create preferences folder: {error}"))?;
        let contents = serde_json::to_vec_pretty(self)
            .map_err(|error| format!("Cannot serialize preferences: {error}"))?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary = parent.join(format!(".preferences-{}-{nonce}.tmp", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
        let mut file = options
            .open(&temporary)
            .map_err(|error| format!("Cannot write preferences: {error}"))?;
        let result = (|| {
            file.write_all(&contents)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)?;
            Ok::<(), std::io::Error>(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(format!("Cannot save preferences: {error}"));
        }
        Ok(())
    }
}

pub fn home_directory() -> PathBuf {
    user_home_directory()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
}

pub(crate) fn user_home_directory() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME").filter(|home| Path::new(home).is_absolute()) {
        return Some(PathBuf::from(home));
    }
    // HOME is normally supplied by the session. A portal service can start
    // without it, so consult the user database before choosing a fallback.
    let mut passwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0u8; 65_536];
    unsafe {
        let code = libc::getpwuid_r(
            libc::geteuid(),
            passwd.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        );
        if code != 0 || result.is_null() {
            return None;
        }
        let passwd = passwd.assume_init();
        if passwd.pw_dir.is_null() {
            return None;
        }
        use std::os::unix::ffi::OsStrExt;
        let home = PathBuf::from(std::ffi::OsStr::from_bytes(
            CStr::from_ptr(passwd.pw_dir).to_bytes(),
        ));
        home.is_absolute().then_some(home)
    }
}

fn config_file() -> Result<PathBuf, String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| Path::new(value).is_absolute())
        .map(PathBuf::from)
        .or_else(|| user_home_directory().map(|home| home.join(".config")))
        .ok_or_else(|| "Cannot determine the current user's configuration directory".to_string())?;
    Ok(config.join("telorgon-file-explorer/preferences.json"))
}

fn user_directories(home: &Path) -> Vec<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| Path::new(value).is_absolute())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let definitions = fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    let mut directories = Vec::new();
    for (key, default_name) in [
        ("XDG_DESKTOP_DIR", "Desktop"),
        ("XDG_DOCUMENTS_DIR", "Documents"),
        ("XDG_DOWNLOAD_DIR", "Downloads"),
        ("XDG_PICTURES_DIR", "Pictures"),
        ("XDG_MUSIC_DIR", "Music"),
        ("XDG_VIDEOS_DIR", "Videos"),
    ] {
        let configured = definitions.lines().find_map(|line| {
            let (name, value) = line.split_once('=')?;
            if name.trim() != key {
                return None;
            }
            let value = value.trim().strip_prefix('"')?.strip_suffix('"')?;
            let path = if let Some(suffix) = value.strip_prefix("$HOME/") {
                home.join(suffix)
            } else if value == "$HOME" {
                home.to_path_buf()
            } else {
                PathBuf::from(value)
            };
            path.is_absolute().then_some(path)
        });
        let path = configured.unwrap_or_else(|| home.join(default_name));
        if path != home && !directories.contains(&path) {
            directories.push(path);
        }
    }
    directories
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_round_trip_and_keep_existing_values_when_fields_are_missing() {
        let path = std::env::temp_dir().join(format!(
            "telorgon-preferences-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        let file = path.join("preferences.json");
        let preferences = Preferences {
            bookmarks: vec![PathBuf::from("/tmp/favorite")],
            show_hidden: true,
            view_mode: "grid".to_string(),
            sort_by: "size".to_string(),
            descending: true,
            last_directory: Some(PathBuf::from("/tmp")),
        };
        preferences.save_to(&file).unwrap();
        assert_eq!(Preferences::load_from(&file).unwrap(), preferences);
        fs::write(
            &file,
            r#"{"show_hidden":true,"view_mode":"invalid","bookmarks":["relative","/tmp","/tmp"]}"#,
        )
        .unwrap();
        let loaded = Preferences::load_from(&file).unwrap();
        assert!(loaded.show_hidden);
        assert_eq!(loaded.view_mode, "list");
        assert_eq!(loaded.bookmarks, vec![PathBuf::from("/tmp")]);
        fs::remove_dir_all(&path).unwrap();
    }
}
