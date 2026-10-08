use std::collections::HashSet;
use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferMode {
    Copy,
    Move,
}

/// Create exactly one child directory. An existing file, folder, or symlink is
/// always an error, including a dangling symlink.
pub fn create_folder(parent: &Path, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    let path = parent.join(name);
    fs::create_dir(&path).map_err(|error| format!("Cannot create {}: {error}", path.display()))?;
    Ok(path)
}

/// Rename without replacing another entry, even if one appears concurrently.
pub fn rename_entry(source: &Path, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    let source = absolute_entry(source)?;
    let parent = source
        .parent()
        .ok_or_else(|| "Cannot rename the filesystem root".to_string())?;
    let target = parent.join(name);
    if source == target {
        fs::symlink_metadata(&source)
            .map_err(|error| format!("Cannot inspect {}: {error}", source.display()))?;
        return Ok(target);
    }
    move_no_replace(&source, &target).map_err(|error| {
        format!(
            "Cannot rename {} to {}: {error}",
            source.display(),
            target.display()
        )
    })?;
    Ok(target)
}

/// Transfer entries into an existing directory with no replacement semantics.
/// All names and collisions are checked before the first item is transferred.
/// A later I/O failure reports how many items were already completed.
pub fn transfer_entries(
    sources: &[PathBuf],
    destination: &Path,
    mode: TransferMode,
) -> Result<Vec<PathBuf>, String> {
    let destination = destination
        .canonicalize()
        .map_err(|error| format!("Cannot open destination {}: {error}", destination.display()))?;
    if !destination.is_dir() {
        return Err(format!("{} is not a folder", destination.display()));
    }
    let mut targets = Vec::with_capacity(sources.len());
    let mut prepared_sources = Vec::with_capacity(sources.len());
    let mut names = HashSet::new();
    let mut canonical_sources = Vec::with_capacity(sources.len());
    for source in sources {
        // Resolve only the parent. Normalizing the final component prevents a
        // trailing slash from causing symlink_metadata to follow a symlink.
        let source = absolute_entry(source)?;
        let metadata = fs::symlink_metadata(&source)
            .map_err(|error| format!("Cannot inspect {}: {error}", source.display()))?;
        let name = source
            .file_name()
            .ok_or_else(|| "Cannot transfer the filesystem root".to_string())?;
        if !names.insert(name.to_os_string()) {
            return Err(format!(
                "More than one selected entry is named {}",
                name.to_string_lossy()
            ));
        }
        let target = destination.join(name);
        ensure_absent(&target)
            .map_err(|error| format!("Cannot use {}: {error}", target.display()))?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            let canonical_source = source
                .canonicalize()
                .map_err(|error| format!("Cannot resolve {}: {error}", source.display()))?;
            if destination.starts_with(&canonical_source) {
                return Err(format!(
                    "Cannot transfer {} into itself or one of its subfolders",
                    source.display()
                ));
            }
            canonical_sources.push((canonical_source, true));
        } else {
            // Canonicalize only the parent: a broken symlink is still a valid
            // transfer source and its target must never be followed.
            canonical_sources.push((source.clone(), false));
        }
        targets.push(target);
        prepared_sources.push(source);
    }
    // Moving both a directory and its child would invalidate a later source.
    // Reject this for copies as well so selection behavior stays predictable.
    for (index, (source, is_dir)) in canonical_sources.iter().enumerate() {
        if *is_dir
            && canonical_sources
                .iter()
                .enumerate()
                .any(|(other_index, (other, _))| index != other_index && other.starts_with(source))
        {
            return Err(format!(
                "Selection contains both {} and one of its children",
                source.display()
            ));
        }
    }
    let mut completed = Vec::with_capacity(sources.len());
    for (source, target) in prepared_sources.iter().zip(targets) {
        let result = match mode {
            TransferMode::Copy => copy_no_replace(source, &target),
            TransferMode::Move => move_no_replace(source, &target),
        };
        if let Err(error) = result {
            let partial = if completed.is_empty() {
                String::new()
            } else {
                format!(
                    " {} item(s) were completed before this error.",
                    completed.len()
                )
            };
            return Err(format!(
                "Cannot transfer {}: {error}.{partial}",
                source.display()
            ));
        }
        completed.push(target);
    }
    Ok(completed)
}

pub fn validate_name(name: &str) -> Result<(), String> {
    let mut components = Path::new(name).components();
    if name.is_empty()
        || name.contains('\0')
        || name.contains('/')
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err("Enter a single file or folder name without a slash".to_string());
    }
    Ok(())
}

pub(crate) fn absolute_entry(path: &Path) -> Result<PathBuf, String> {
    let normalized: PathBuf = path.components().collect();
    let name = normalized
        .file_name()
        .ok_or_else(|| "Cannot move or restore the filesystem root".to_string())?;
    let parent = normalized
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = parent
        .canonicalize()
        .map_err(|error| format!("Cannot open original folder {}: {error}", parent.display()))?;
    Ok(parent.join(name))
}

pub(crate) fn ensure_absent(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(io::Error::new(
            ErrorKind::AlreadyExists,
            "an entry with this name already exists",
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn copy_no_replace(source: &Path, target: &Path) -> io::Result<()> {
    copy_at_depth(source, target, 0)
}

fn copy_at_depth(source: &Path, target: &Path, depth: usize) -> io::Result<()> {
    if depth > 256 {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "folder nesting exceeds the transfer limit",
        ));
    }
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        let link = fs::read_link(source)?;
        #[cfg(unix)]
        return std::os::unix::fs::symlink(link, target);
        #[cfg(not(unix))]
        return Err(io::Error::new(
            ErrorKind::Unsupported,
            "symlink transfers require Unix",
        ));
    }
    if metadata.is_dir() {
        fs::create_dir(target)?;
        let result = (|| {
            for child in fs::read_dir(source)? {
                let child = child?;
                copy_at_depth(&child.path(), &target.join(child.file_name()), depth + 1)?;
            }
            preserve_times(&File::open(target)?, &metadata)?;
            fs::set_permissions(target, metadata.permissions())?;
            Ok(())
        })();
        if result.is_err() {
            // This target was created exclusively by this operation.
            let _ = fs::remove_dir_all(target);
        }
        return result;
    }
    if !metadata.is_file() {
        return Err(io::Error::new(
            ErrorKind::Unsupported,
            "sockets, devices, and pipes cannot be copied",
        ));
    }
    let mut source_options = OpenOptions::new();
    source_options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        source_options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut input = source_options.open(source)?;
    let source_metadata = input.metadata()?;
    if !source_metadata.is_file() {
        return Err(io::Error::new(
            ErrorKind::Unsupported,
            "the source is no longer a regular file",
        ));
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let result = (|| {
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        preserve_times(&output, &source_metadata)?;
        output.set_permissions(source_metadata.permissions())?;
        Ok(())
    })();
    drop(output);
    if result.is_err() {
        let _ = fs::remove_file(target);
    }
    result
}

fn preserve_times(file: &File, metadata: &fs::Metadata) -> io::Result<()> {
    let mut times = FileTimes::new();
    if let Ok(accessed) = metadata.accessed() {
        times = times.set_accessed(accessed);
    }
    if let Ok(modified) = metadata.modified() {
        times = times.set_modified(modified);
    }
    file.set_times(times)
}

pub(crate) fn move_no_replace(source: &Path, target: &Path) -> io::Result<()> {
    match rename_no_replace(source, target) {
        Ok(()) => Ok(()),
        Err(error)
            if error.raw_os_error() == Some(libc::EXDEV)
                || error.raw_os_error() == Some(libc::ENOSYS)
                || error.kind() == ErrorKind::Unsupported =>
        {
            copy_no_replace(source, target)?;
            if let Err(error) = remove_entry(source) {
                // A complete destination remains available if source removal
                // fails part-way through a cross-filesystem directory move.
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "copied to {}, but could not completely remove the source: {error}",
                        target.display()
                    ),
                ));
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "linux")]
fn rename_no_replace(source: &Path, target: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "source contains a NUL byte"))?;
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "destination contains a NUL byte"))?;
    // renameat2(RENAME_NOREPLACE) is atomic and treats dangling symlinks as
    // occupied destinations. Plain std::fs::rename would silently overwrite.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "linux"))]
fn rename_no_replace(_source: &Path, _target: &Path) -> io::Result<()> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-replace rename is unavailable",
    ))
}

pub(crate) fn remove_entry(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    pub struct TempDir(pub PathBuf);
    impl TempDir {
        pub fn new() -> Self {
            loop {
                let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "telorgon-explorer-test-{}-{id}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("cannot create test directory: {error}"),
                }
            }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::TempDir;

    #[test]
    fn copy_and_rename_never_clobber_existing_entries() {
        let temp = TempDir::new();
        let source = temp.0.join("source");
        let destination = temp.0.join("destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("file"), b"source bytes").unwrap();
        fs::write(destination.join("file"), b"keep me").unwrap();
        assert!(
            transfer_entries(&[source.join("file")], &destination, TransferMode::Copy).is_err()
        );
        assert_eq!(fs::read(destination.join("file")).unwrap(), b"keep me");
        fs::write(source.join("other"), b"other bytes").unwrap();
        assert!(rename_entry(&source.join("file"), "other").is_err());
        assert_eq!(fs::read(source.join("other")).unwrap(), b"other bytes");
        assert!(rename_entry(&source.join("file"), "../escape").is_err());
        assert!(rename_entry(&source.join("file"), "trailing/").is_err());
    }

    #[test]
    fn recursive_copy_rejects_destination_inside_source_before_creating_anything() {
        let temp = TempDir::new();
        let source = temp.0.join("folder");
        let destination = source.join("child");
        fs::create_dir_all(&destination).unwrap();
        assert!(transfer_entries(&[source.clone()], &destination, TransferMode::Copy).is_err());
        assert!(!destination.join("folder").exists());
        assert!(transfer_entries(&[source.clone()], &destination, TransferMode::Move).is_err());
        assert!(source.exists());
    }

    #[test]
    fn copies_and_moves_symlinks_without_following_them() {
        let temp = TempDir::new();
        let source = temp.0.join("source");
        let destination = temp.0.join("destination");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        std::os::unix::fs::symlink("missing-target", source.join("broken")).unwrap();
        transfer_entries(&[source.join("broken")], &destination, TransferMode::Copy).unwrap();
        assert_eq!(
            fs::read_link(destination.join("broken")).unwrap(),
            PathBuf::from("missing-target")
        );
        assert!(
            transfer_entries(&[source.join("broken")], &destination, TransferMode::Move).is_err()
        );
        assert!(fs::symlink_metadata(source.join("broken")).is_ok());
        fs::remove_file(destination.join("broken")).unwrap();
        transfer_entries(&[source.join("broken")], &destination, TransferMode::Move).unwrap();
        assert!(fs::symlink_metadata(source.join("broken")).is_err());
        assert_eq!(
            fs::read_link(destination.join("broken")).unwrap(),
            PathBuf::from("missing-target")
        );
    }

    #[test]
    fn trailing_slash_does_not_turn_a_directory_symlink_into_a_recursive_copy() {
        let temp = TempDir::new();
        let source = temp.0.join("source");
        let destination = temp.0.join("destination");
        let real = temp.0.join("real");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::create_dir(&real).unwrap();
        fs::write(real.join("untouched"), b"original").unwrap();
        std::os::unix::fs::symlink("../real", source.join("link")).unwrap();
        let with_slash = PathBuf::from(format!("{}/", source.join("link").display()));
        transfer_entries(&[with_slash.clone()], &destination, TransferMode::Copy).unwrap();
        assert!(
            fs::symlink_metadata(destination.join("link"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        rename_entry(&with_slash, "renamed").unwrap();
        assert!(
            fs::symlink_metadata(source.join("renamed"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(real.join("untouched")).unwrap(), b"original");
    }

    #[test]
    fn batch_preflight_does_not_partially_copy_before_a_collision() {
        let temp = TempDir::new();
        let destination = temp.0.join("destination");
        fs::create_dir(&destination).unwrap();
        fs::write(temp.0.join("first"), b"first").unwrap();
        fs::write(temp.0.join("second"), b"second").unwrap();
        fs::write(destination.join("second"), b"existing").unwrap();
        assert!(
            transfer_entries(
                &[temp.0.join("first"), temp.0.join("second")],
                &destination,
                TransferMode::Copy
            )
            .is_err()
        );
        assert!(!destination.join("first").exists());
    }

    #[test]
    fn directories_and_content_transfer_intact() {
        let temp = TempDir::new();
        let source = temp.0.join("folder");
        let destination = temp.0.join("destination");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("nested/document"), b"hello").unwrap();
        transfer_entries(&[source.clone()], &destination, TransferMode::Copy).unwrap();
        assert_eq!(
            fs::read(destination.join("folder/nested/document")).unwrap(),
            b"hello"
        );
        assert_eq!(
            fs::metadata(source.join("nested/document"))
                .unwrap()
                .modified()
                .unwrap(),
            fs::metadata(destination.join("folder/nested/document"))
                .unwrap()
                .modified()
                .unwrap()
        );
        rename_entry(&source, "renamed").unwrap();
        assert!(!source.exists());
        assert_eq!(
            fs::read(temp.0.join("renamed/nested/document")).unwrap(),
            b"hello"
        );
    }
}
