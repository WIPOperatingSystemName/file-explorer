//! Freedesktop home-trash interoperability. Metadata stores percent-encoded
//! original paths and local deletion timestamps in standard `.trashinfo` files.

use super::operations::{absolute_entry, ensure_absent, move_no_replace};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashItem {
    pub original_path: PathBuf,
    pub trashed_path: PathBuf,
    pub info_path: PathBuf,
    pub deleted_at: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

pub fn trash_entries(paths: &[PathBuf]) -> Result<Vec<TrashItem>, String> {
    TrashStore::user()?.trash_many(paths)
}

pub fn list_trash() -> Result<Vec<TrashItem>, String> {
    TrashStore::user()?.list()
}

pub fn restore_trash(item: &TrashItem) -> Result<PathBuf, String> {
    TrashStore::user()?.restore(item)
}

struct TrashStore {
    root: PathBuf,
    files: PathBuf,
    info: PathBuf,
}

impl TrashStore {
    fn new(root: PathBuf) -> Self {
        Self {
            files: root.join("files"),
            info: root.join("info"),
            root,
        }
    }

    fn user() -> Result<Self, String> {
        let data = match std::env::var_os("XDG_DATA_HOME")
            .filter(|value| Path::new(value).is_absolute())
        {
            Some(data) => PathBuf::from(data),
            None => crate::preferences::user_home_directory()
                .ok_or_else(|| "Cannot determine the current user's home directory".to_string())?
                .join(".local/share"),
        };
        if !data.is_absolute() {
            return Err("Cannot determine the current user's data directory".to_string());
        }
        Ok(Self::new(data.join("Trash")))
    }

    fn prepare(&self) -> Result<(), String> {
        if let Some(parent) = self.root.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Cannot create Trash location: {error}"))?;
        }
        for path in [&self.root, &self.files, &self.info] {
            match fs::create_dir(path) {
                Ok(()) => {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                        .map_err(|error| format!("Cannot secure {}: {error}", path.display()))?;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    self.check_directory(path)?
                }
                Err(error) => return Err(format!("Cannot create {}: {error}", path.display())),
            }
        }
        Ok(())
    }

    fn check_directory(&self, path: &Path) -> Result<(), String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("Cannot inspect {}: {error}", path.display()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(format!(
                "Trash location {} must be a real directory",
                path.display()
            ));
        }
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(format!(
                "Trash location {} belongs to another user",
                path.display()
            ));
        }
        Ok(())
    }

    fn trash_many(&self, paths: &[PathBuf]) -> Result<Vec<TrashItem>, String> {
        self.prepare()?;
        let trash_root = self
            .root
            .canonicalize()
            .map_err(|error| format!("Cannot resolve Trash: {error}"))?;
        let mut sources = Vec::with_capacity(paths.len());
        for path in paths {
            let source = absolute_entry(path)?;
            let metadata = fs::symlink_metadata(&source)
                .map_err(|error| format!("Cannot inspect {}: {error}", path.display()))?;
            if source.starts_with(&trash_root)
                || (metadata.is_dir() && trash_root.starts_with(&source))
            {
                return Err(format!("Cannot move {} into its own Trash", path.display()));
            }
            sources.push((source, metadata.is_dir()));
        }
        for (index, (source, is_dir)) in sources.iter().enumerate() {
            if sources.iter().enumerate().any(|(other_index, (other, _))| {
                index != other_index && (other == source || (*is_dir && other.starts_with(source)))
            }) {
                return Err(
                    "Select each item once, without selecting a folder and its child together"
                        .to_string(),
                );
            }
        }
        let mut completed = Vec::new();
        for (source, _) in sources {
            match self.trash_one(&source) {
                Ok(item) => completed.push(item),
                Err(error) => {
                    return Err(if completed.is_empty() {
                        error
                    } else {
                        format!(
                            "{error}. {} item(s) were moved to Trash before this error.",
                            completed.len()
                        )
                    });
                }
            }
        }
        Ok(completed)
    }

    fn trash_one(&self, source: &Path) -> Result<TrashItem, String> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let deleted_at = deletion_timestamp();
        for _ in 0..128 {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let id = format!("telorgon-{stamp}-{}-{sequence}", std::process::id());
            let trashed_path = self.files.join(&id);
            let info_path = self.info.join(format!("{id}.trashinfo"));
            match ensure_absent(&trashed_path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("Cannot allocate a Trash entry: {error}")),
            }
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
            let mut info_file = match options.open(&info_path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("Cannot create Trash metadata: {error}")),
            };
            let metadata = format!(
                "[Trash Info]\nPath={}\nDeletionDate={deleted_at}\n",
                encode_path(source)
            );
            if let Err(error) = info_file
                .write_all(metadata.as_bytes())
                .and_then(|_| info_file.sync_all())
            {
                drop(info_file);
                let _ = fs::remove_file(&info_path);
                return Err(format!("Cannot save Trash metadata: {error}"));
            }
            drop(info_file);
            if let Err(error) = move_no_replace(source, &trashed_path) {
                // A cross-filesystem move can leave a complete trash copy if
                // removing the original fails. Keep its restoration metadata.
                if error.kind() == io::ErrorKind::AlreadyExists
                    || fs::symlink_metadata(&trashed_path).is_err()
                {
                    let _ = fs::remove_file(&info_path);
                }
                return Err(format!(
                    "Cannot move {} to Trash: {error}",
                    source.display()
                ));
            }
            return self.read_item(&info_path).map_err(|error| {
                format!("Item moved to Trash but metadata could not be read: {error}")
            });
        }
        Err("Cannot allocate a unique Trash entry".to_string())
    }

    fn list(&self) -> Result<Vec<TrashItem>, String> {
        if fs::symlink_metadata(&self.root)
            .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
        {
            return Ok(Vec::new());
        }
        for path in [&self.root, &self.files, &self.info] {
            self.check_directory(path)?;
        }
        let metadata =
            fs::read_dir(&self.info).map_err(|error| format!("Cannot read Trash: {error}"))?;
        let mut items = Vec::new();
        for entry in metadata {
            let entry = entry.map_err(|error| format!("Cannot read Trash: {error}"))?;
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "trashinfo")
            {
                // Corrupt or orphaned metadata must not hide other valid items.
                if let Ok(item) = self.read_item(&entry.path()) {
                    items.push(item);
                }
            }
        }
        items.sort_by(|left, right| {
            right
                .deleted_at
                .cmp(&left.deleted_at)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(items)
    }

    fn read_item(&self, info_path: &Path) -> Result<TrashItem, String> {
        if info_path.parent() != Some(self.info.as_path())
            || info_path
                .extension()
                .is_none_or(|extension| extension != "trashinfo")
        {
            return Err("Invalid Trash metadata location".to_string());
        }
        let id = info_path
            .file_stem()
            .ok_or_else(|| "Invalid Trash metadata name".to_string())?;
        let trashed_path = self.files.join(id);
        let entry = super::entry_for_path(&trashed_path)
            .map_err(|error| format!("Cannot inspect Trash entry: {error}"))?;
        let mut options = OpenOptions::new();
        options.read(true);
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let file = options
            .open(info_path)
            .map_err(|error| format!("Cannot read Trash metadata: {error}"))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("Cannot inspect Trash metadata: {error}"))?;
        if !metadata.is_file() || metadata.len() > 65536 {
            return Err("Invalid Trash metadata file".to_string());
        }
        let mut text = String::new();
        file.take(65537)
            .read_to_string(&mut text)
            .map_err(|error| format!("Invalid Trash metadata: {error}"))?;
        if text.len() > 65536 {
            return Err("Trash metadata is too large".to_string());
        }
        let mut section = false;
        let mut original_path = None;
        let mut deleted_at = None;
        for line in text.lines() {
            if line.starts_with('[') {
                section = line == "[Trash Info]";
                continue;
            }
            if !section {
                continue;
            }
            if let Some(path) = line.strip_prefix("Path=") {
                original_path = Some(decode_path(path)?);
            }
            if let Some(date) = line.strip_prefix("DeletionDate=") {
                deleted_at = Some(date.to_string());
            }
        }
        let original_path = original_path
            .ok_or_else(|| "Trash metadata is missing the original path".to_string())?;
        if !original_path.is_absolute() || original_path.file_name().is_none() {
            return Err("Trash metadata has an invalid original path".to_string());
        }
        let name = original_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        Ok(TrashItem {
            original_path,
            trashed_path,
            info_path: info_path.to_path_buf(),
            deleted_at: deleted_at.unwrap_or_default(),
            name,
            is_dir: entry.is_dir,
            size: entry.size,
        })
    }

    fn restore(&self, item: &TrashItem) -> Result<PathBuf, String> {
        for path in [&self.root, &self.files, &self.info] {
            self.check_directory(path)?;
        }
        let stored = self.read_item(&item.info_path)?;
        if stored.trashed_path != item.trashed_path || stored.original_path != item.original_path {
            return Err("The Trash entry changed; refresh Trash before restoring".to_string());
        }
        let target = absolute_entry(&stored.original_path)?;
        let trash_root = self
            .root
            .canonicalize()
            .map_err(|error| format!("Cannot resolve Trash: {error}"))?;
        if target.starts_with(&trash_root) {
            return Err("Cannot restore an item inside Trash".to_string());
        }
        ensure_absent(&target)
            .map_err(|error| format!("Cannot restore {}: {error}", target.display()))?;
        move_no_replace(&stored.trashed_path, &target)
            .map_err(|error| format!("Cannot restore {}: {error}", target.display()))?;
        fs::remove_file(&stored.info_path).map_err(|error| {
            format!(
                "Restored {} but could not remove its Trash metadata: {error}",
                target.display()
            )
        })?;
        Ok(target)
    }
}

fn encode_path(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut encoded = String::new();
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn decode_path(encoded: &str) -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStringExt;
    let mut decoded = Vec::new();
    let bytes = encoded.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err("Invalid percent encoding in Trash metadata".to_string());
            }
            let high = hex(bytes[index + 1])
                .ok_or_else(|| "Invalid percent encoding in Trash metadata".to_string())?;
            let low = hex(bytes[index + 2])
                .ok_or_else(|| "Invalid percent encoding in Trash metadata".to_string())?;
            index += 3;
            high * 16 + low
        } else {
            let value = bytes[index];
            index += 1;
            value
        };
        if byte == 0 {
            return Err("NUL byte in Trash metadata path".to_string());
        }
        decoded.push(byte);
    }
    Ok(PathBuf::from(OsString::from_vec(decoded)))
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn deletion_timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as libc::time_t;
    let mut calendar = std::mem::MaybeUninit::<libc::tm>::uninit();
    let mut buffer = [0u8; 32];
    // These libc calls use caller-owned buffers and produce the local time
    // required by the freedesktop Trash specification.
    unsafe {
        if libc::localtime_r(&seconds, calendar.as_mut_ptr()).is_null() {
            return String::new();
        }
        let calendar = calendar.assume_init();
        let length = libc::strftime(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            c"%Y-%m-%dT%H:%M:%S".as_ptr(),
            &calendar,
        );
        String::from_utf8_lossy(&buffer[..length]).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::operations::test_support::TempDir;

    #[test]
    fn trash_is_interoperable_and_restore_refuses_to_overwrite() {
        let temp = TempDir::new();
        let store = TrashStore::new(temp.0.join("Trash"));
        let source = temp.0.join("a # café.txt");
        fs::write(&source, b"original bytes").unwrap();
        let item = store.trash_many(&[source.clone()]).unwrap().remove(0);
        assert!(!source.exists());
        assert_eq!(fs::read(&item.trashed_path).unwrap(), b"original bytes");
        let metadata = fs::read_to_string(&item.info_path).unwrap();
        assert!(metadata.starts_with("[Trash Info]\nPath=/"));
        assert!(metadata.contains("a%20%23%20caf%C3%A9.txt"));
        assert!(metadata.contains("DeletionDate="));
        assert_eq!(store.list().unwrap(), vec![item.clone()]);
        fs::write(&source, b"new bytes").unwrap();
        assert!(store.restore(&item).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"new bytes");
        assert!(item.trashed_path.exists());
        fs::remove_file(&source).unwrap();
        assert_eq!(store.restore(&item).unwrap(), source);
        assert_eq!(fs::read(&source).unwrap(), b"original bytes");
        assert!(store.list().unwrap().is_empty());
        assert!(!item.info_path.exists());
    }

    #[test]
    fn trash_preserves_dangling_symlinks_and_duplicate_basenames() {
        let temp = TempDir::new();
        let store = TrashStore::new(temp.0.join("Trash"));
        fs::create_dir(temp.0.join("a")).unwrap();
        fs::create_dir(temp.0.join("b")).unwrap();
        let first = temp.0.join("a/link");
        let second = temp.0.join("b/link");
        std::os::unix::fs::symlink("missing-a", &first).unwrap();
        std::os::unix::fs::symlink("missing-b", &second).unwrap();
        let items = store.trash_many(&[first.clone(), second.clone()]).unwrap();
        assert_eq!(items.len(), 2);
        assert_ne!(items[0].trashed_path, items[1].trashed_path);
        assert_eq!(
            fs::read_link(&items[0].trashed_path).unwrap(),
            PathBuf::from("missing-a")
        );
        for item in &items {
            store.restore(item).unwrap();
        }
        assert_eq!(fs::read_link(&first).unwrap(), PathBuf::from("missing-a"));
        assert_eq!(fs::read_link(&second).unwrap(), PathBuf::from("missing-b"));
    }

    #[test]
    fn trash_rejects_its_ancestor_and_ignores_corrupt_metadata() {
        let temp = TempDir::new();
        let store = TrashStore::new(temp.0.join("Trash"));
        store.prepare().unwrap();
        assert!(store.trash_many(&[temp.0.clone()]).is_err());
        fs::write(store.info.join("bad.trashinfo"), "[Trash Info]\nPath=%ZZ").unwrap();
        fs::write(store.files.join("bad"), b"still here").unwrap();
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn percent_encoding_round_trips_non_utf8_paths() {
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(OsString::from_vec(b"/tmp/path-\xff #\n".to_vec()));
        assert_eq!(decode_path(&encode_path(&path)).unwrap(), path);
        assert!(decode_path("/tmp/%00").is_err());
        assert!(decode_path("/tmp/%x1").is_err());
    }
}
