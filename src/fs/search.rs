use super::{Entry, entry_for_path};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

const MAX_VISITED: usize = 250_000;
const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchStatus {
    pub visited: usize,
    pub matches: usize,
    pub limited: bool,
    pub error: Option<String>,
}

/// A bounded background search. Calling `poll` never blocks the UI thread.
/// Hidden directories are pruned when disabled and symlinks are never traversed.
pub struct SearchTask {
    receiver: mpsc::Receiver<Entry>,
    cancelled: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    status: Arc<Mutex<SearchStatus>>,
}

impl SearchTask {
    pub fn start(
        root: &Path,
        query: &str,
        include_hidden: bool,
        max_results: usize,
    ) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|error| format!("Cannot search {}: {error}", root.display()))?;
        if !root.is_dir() {
            return Err(format!("{} is not a folder", root.display()));
        }
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Err("Enter a filename to search for".to_string());
        }
        let limit = max_results.clamp(1, 50_000);
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(SearchStatus::default()));
        let worker_cancelled = Arc::clone(&cancelled);
        let worker_finished = Arc::clone(&finished);
        let worker_status = Arc::clone(&status);
        std::thread::Builder::new()
            .name("explorer-search".to_string())
            .spawn(move || {
                search(
                    root,
                    &query,
                    include_hidden,
                    limit,
                    sender,
                    &worker_cancelled,
                    &worker_status,
                );
                worker_finished.store(true, Ordering::Release);
            })
            .map_err(|error| format!("Cannot start search: {error}"))?;
        Ok(Self {
            receiver,
            cancelled,
            finished,
            status,
        })
    }

    pub fn poll(&self) -> Vec<Entry> {
        self.receiver.try_iter().collect()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub fn status(&self) -> SearchStatus {
        self.status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

impl Drop for SearchTask {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn search(
    root: PathBuf,
    query: &str,
    include_hidden: bool,
    max_results: usize,
    sender: mpsc::Sender<Entry>,
    cancelled: &AtomicBool,
    shared_status: &Mutex<SearchStatus>,
) {
    let mut pending = vec![(root, 0usize)];
    let mut status = SearchStatus::default();
    'directories: while let Some((directory, depth)) = pending.pop() {
        if cancelled.load(Ordering::Acquire) {
            break;
        }
        let children = match fs::read_dir(&directory) {
            Ok(children) => children,
            Err(error) => {
                status.error.get_or_insert_with(|| {
                    format!(
                        "Some folders could not be searched: {}: {error}",
                        directory.display()
                    )
                });
                continue;
            }
        };
        for child in children {
            if cancelled.load(Ordering::Acquire) {
                break 'directories;
            }
            if status.visited >= MAX_VISITED {
                status.limited = true;
                break 'directories;
            }
            let child = match child {
                Ok(child) => child,
                Err(error) => {
                    status.error.get_or_insert_with(|| {
                        format!("Some entries could not be searched: {error}")
                    });
                    continue;
                }
            };
            status.visited += 1;
            let entry = match entry_for_path(&child.path()) {
                Ok(entry) => entry,
                Err(_) => continue, // Concurrent deletions and unreadable entries are skipped.
            };
            if !include_hidden && entry.is_hidden() {
                continue;
            }
            if entry.is_dir && !entry.is_symlink {
                if depth < MAX_DEPTH {
                    pending.push((entry.path.clone(), depth + 1));
                } else {
                    status.limited = true;
                }
            }
            if entry.name.to_lowercase().contains(query) {
                if sender.send(entry).is_err() {
                    break 'directories;
                }
                status.matches += 1;
                if status.matches >= max_results {
                    status.limited = true;
                    break 'directories;
                }
            }
            if status.visited % 128 == 0 {
                *shared_status
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = status.clone();
            }
        }
    }
    *shared_status
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = status;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::operations::test_support::TempDir;
    use std::time::{Duration, Instant};

    fn finish(task: &SearchTask) -> Vec<Entry> {
        let timeout = Instant::now() + Duration::from_secs(5);
        let mut entries = Vec::new();
        while !task.is_finished() {
            entries.extend(task.poll());
            assert!(Instant::now() < timeout, "search did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
        entries.extend(task.poll());
        entries
    }

    #[test]
    fn search_prunes_hidden_folders_and_does_not_follow_directory_links() {
        let temp = TempDir::new();
        fs::create_dir(temp.0.join("visible")).unwrap();
        fs::create_dir(temp.0.join(".hidden")).unwrap();
        fs::write(temp.0.join("visible/Match.txt"), b"").unwrap();
        fs::write(temp.0.join(".hidden/match.txt"), b"").unwrap();
        std::os::unix::fs::symlink(&temp.0, temp.0.join("visible/cycle")).unwrap();
        let task = SearchTask::start(&temp.0, "match", false, 100).unwrap();
        let entries = finish(&task);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Match.txt");
        let task = SearchTask::start(&temp.0, "match", true, 100).unwrap();
        assert_eq!(finish(&task).len(), 2);
    }

    #[test]
    fn search_caps_result_count_and_can_be_cancelled() {
        let temp = TempDir::new();
        for index in 0..20 {
            fs::write(temp.0.join(format!("match-{index}")), b"").unwrap();
        }
        let task = SearchTask::start(&temp.0, "match", false, 3).unwrap();
        assert_eq!(finish(&task).len(), 3);
        assert!(task.status().limited);
        let task = SearchTask::start(&temp.0, "match", false, 100).unwrap();
        task.cancel();
        assert!(finish(&task).len() <= 20);
    }
}
