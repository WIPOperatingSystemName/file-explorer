use crate::{
    fs::{self, Entry, TransferMode, TrashItem},
    preferences::{Preferences, home_directory},
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use telorgon::app::{Signal, SignalWriter};

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub directory: PathBuf,
    pub entries: Vec<Entry>,
    pub selected: Vec<PathBuf>,
    pub anchor: Option<PathBuf>,
    pub cursor: Option<PathBuf>,
    pub loading: bool,
    pub busy: bool,
    pub message: String,
    pub error: bool,
    pub trash: Option<Vec<TrashItem>>,
    pub history: Vec<PathBuf>,
    pub position: usize,
    pub query: String,
    pub generation: u64,
    pub undo_trash: Vec<TrashItem>,
    pub preview: Option<Preview>,
    pub preview_generation: u64,
}
#[derive(Clone, Debug)]
pub struct Preview {
    pub path: PathBuf,
    pub detail: String,
    pub text: Option<String>,
    pub image: Option<telorgon::graphics::render::ImageResource>,
}
#[derive(Clone)]
pub struct Browser(Arc<Inner>);
#[derive(Clone, Default)]
pub struct FileWorkers(Arc<Mutex<Vec<thread::JoinHandle<()>>>>);
impl FileWorkers {
    fn spawn(&self, work: impl FnOnce() + Send + 'static) -> Result<(), String> {
        let handle = thread::Builder::new()
            .name("explorer-file-operation".into())
            .spawn(work)
            .map_err(|error| format!("Cannot start file operation: {error}"))?;
        let mut workers = self.0.lock().unwrap();
        let mut index = 0;
        while index < workers.len() {
            if workers[index].is_finished() {
                let _ = workers.swap_remove(index).join();
            } else {
                index += 1;
            }
        }
        workers.push(handle);
        Ok(())
    }
    pub fn finish(&self) {
        let workers = std::mem::take(&mut *self.0.lock().unwrap());
        for worker in workers {
            let _ = worker.join();
        }
    }
}
struct Inner {
    state: Mutex<Snapshot>,
    signal: Signal<Snapshot>,
    writer: SignalWriter<Snapshot>,
    workers: FileWorkers,
}
impl PartialEq for Browser {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Browser {
    pub fn new(path: PathBuf, workers: FileWorkers) -> Self {
        let path = if path.is_file() {
            path.parent().unwrap_or(Path::new("/")).to_path_buf()
        } else {
            path
        };
        let state = Snapshot {
            directory: path.clone(),
            entries: vec![],
            selected: vec![],
            anchor: None,
            cursor: None,
            loading: false,
            busy: false,
            message: String::new(),
            error: false,
            trash: None,
            history: vec![path],
            position: 0,
            query: String::new(),
            generation: 0,
            undo_trash: vec![],
            preview: None,
            preview_generation: 0,
        };
        let (signal, writer) = Signal::new(state.clone());
        let browser = Self(Arc::new(Inner {
            state: Mutex::new(state),
            signal,
            writer,
            workers,
        }));
        browser.refresh();
        let weak = Arc::downgrade(&browser.0);
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(2));
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                let browser = Browser(inner);
                let state = browser.snapshot();
                if state.loading || state.busy || state.trash.is_some() || !state.query.is_empty() {
                    continue;
                }
                if let Ok(mut entries) = fs::list_directory(&state.directory) {
                    entries.sort_by(|a, b| a.path.cmp(&b.path));
                    let mut previous = state.entries.clone();
                    previous.sort_by(|a, b| a.path.cmp(&b.path));
                    if previous != entries {
                        browser.change(|current| {
                            if current.generation == state.generation && !current.busy {
                                current.entries = entries;
                                current.selected.retain(|path| {
                                    current.entries.iter().any(|entry| &entry.path == path)
                                });
                            }
                        });
                    }
                }
            }
        });
        browser
    }
    pub fn signal(&self) -> &Signal<Snapshot> {
        &self.0.signal
    }
    pub fn snapshot(&self) -> Snapshot {
        self.0.state.lock().unwrap().clone()
    }
    fn change(&self, f: impl FnOnce(&mut Snapshot)) {
        let mut state = self.0.state.lock().unwrap();
        f(&mut state);
        self.0.writer.publish(state.clone());
    }
    pub fn message(&self, message: impl Into<String>, error: bool) {
        let message = message.into();
        self.change(|state| {
            state.message = message;
            state.error = error;
        });
    }
    pub fn navigate(&self, path: PathBuf) {
        if self.snapshot().busy {
            return;
        }
        self.change(|state| {
            state.history.truncate(state.position + 1);
            if state.history.last() != Some(&path) {
                state.history.push(path.clone());
            }
            state.position = state.history.len() - 1;
            state.directory = path;
            state.trash = None;
            state.selected.clear();
            state.anchor = None;
            state.cursor = None;
            state.preview = None;
            state.preview_generation += 1;
            state.query.clear();
        });
        self.refresh();
    }
    pub fn history(&self, forward: bool) {
        let state = self.snapshot();
        if state.busy {
            return;
        }
        let next = if forward {
            state.position.checked_add(1)
        } else {
            state.position.checked_sub(1)
        };
        if let Some(position) = next.filter(|i| *i < state.history.len()) {
            self.change(|state| {
                state.position = position;
                state.directory = state.history[position].clone();
                state.trash = None;
                state.selected.clear();
                state.anchor = None;
                state.cursor = None;
                state.preview = None;
                state.preview_generation += 1;
                state.query.clear();
            });
            self.refresh();
        }
    }
    pub fn refresh(&self) {
        let (path, generation, trash) = {
            self.change(|state| {
                state.loading = true;
                state.generation += 1;
                state.query.clear();
            });
            let state = self.snapshot();
            (state.directory, state.generation, state.trash.is_some())
        };
        let browser = self.clone();
        thread::spawn(move || {
            let result = if trash {
                fs::list_trash().map(|items| {
                    let entries = items
                        .iter()
                        .map(|item| Entry {
                            path: item.trashed_path.clone(),
                            name: item.name.clone(),
                            is_dir: item.is_dir,
                            is_symlink: false,
                            size: item.size,
                            modified: None,
                            kind: "Trashed".into(),
                        })
                        .collect();
                    (entries, Some(items))
                })
            } else {
                fs::list_directory(&path).map(|entries| (entries, None))
            };
            browser.change(|state| {
                if state.generation != generation {
                    return;
                }
                state.loading = false;
                match result {
                    Ok((entries, items)) => {
                        state.entries = entries;
                        state.trash = items;
                        state
                            .selected
                            .retain(|path| state.entries.iter().any(|entry| &entry.path == path));
                    }
                    Err(error) => {
                        state.entries.clear();
                        state.message = error;
                        state.error = true;
                    }
                }
            });
        });
    }
    pub fn search(&self, query: String, include_hidden: bool) {
        if query.trim().is_empty() {
            self.refresh();
            return;
        }
        self.change(|state| {
            state.generation += 1;
            state.query = query.clone();
            state.entries.clear();
            state.selected.clear();
            state.anchor = None;
            state.cursor = None;
            state.loading = true;
            state.preview = None;
            state.preview_generation += 1;
        });
        let state = self.snapshot();
        let browser = self.clone();
        thread::spawn(move || {
            match fs::SearchTask::start(&state.directory, &query, include_hidden, 10_000) {
                Ok(search) => loop {
                    if browser.snapshot().generation != state.generation {
                        search.cancel();
                        break;
                    }
                    let mut batch = search.poll();
                    let finished = search.is_finished();
                    if finished {
                        batch.extend(search.poll());
                    }
                    if !batch.is_empty() || finished {
                        browser.change(|current| {
                            if current.generation != state.generation { return; }
                            current.entries.extend(batch);
                            current.loading = !finished;
                            if finished { let status = search.status();
                                current.message = status.error.clone().unwrap_or_else(|| if status.limited {
                                    "Showing the first 10,000 search results. Narrow your search.".into()
                                } else { format!("Search complete · {} results", status.matches) });
                                current.error = status.error.is_some();
                            }
                        });
                    }
                    if finished {
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
                },
                Err(error) => browser.change(|current| {
                    if current.generation == state.generation {
                        current.loading = false;
                        current.message = error;
                        current.error = true;
                    }
                }),
            }
        });
    }
    pub fn trash_location(&self) {
        if self.snapshot().busy {
            return;
        }
        self.change(|state| {
            state.trash = Some(vec![]);
            state.selected.clear();
            state.anchor = None;
            state.cursor = None;
            state.preview = None;
            state.preview_generation += 1;
        });
        self.refresh();
    }
    pub fn select(&self, path: PathBuf, additive: bool, range: bool, ordered: &[PathBuf]) {
        self.change(|state| {
            if range {
                let anchor = state
                    .anchor
                    .as_ref()
                    .and_then(|p| ordered.iter().position(|v| v == p));
                let target = ordered.iter().position(|p| p == &path);
                if let (Some(a), Some(b)) = (anchor, target) {
                    state.selected = ordered[a.min(b)..=a.max(b)].to_vec();
                    state.cursor = Some(path);
                    return;
                }
            }
            state.anchor = Some(path.clone());
            state.cursor = Some(path.clone());
            if additive {
                if let Some(index) = state.selected.iter().position(|value| value == &path) {
                    state.selected.remove(index);
                } else {
                    state.selected.push(path);
                }
            } else {
                state.selected = vec![path];
            }
        });
    }
    pub fn select_all(&self, paths: Vec<PathBuf>) {
        self.change(|state| {
            state.anchor = paths.first().cloned();
            state.cursor = paths.last().cloned();
            state.selected = paths;
        });
    }
    pub fn select_initial(&self, path: PathBuf) {
        self.change(|state| {
            state.anchor = Some(path.clone());
            state.cursor = Some(path.clone());
            state.selected = vec![path];
        });
    }
    fn operation(
        &self,
        done: impl FnOnce() -> Result<(String, Vec<TrashItem>), String> + Send + 'static,
    ) {
        if self.snapshot().busy {
            return;
        }
        self.change(|state| {
            state.busy = true;
            state.message = "Working…".into();
            state.error = false;
        });
        let browser = self.clone();
        let result = self.0.workers.spawn(move || {
            let result = done();
            browser.change(|state| {
                state.busy = false;
                match result {
                    Ok((message, trash)) => {
                        state.message = message;
                        state.error = false;
                        if !trash.is_empty() {
                            state.undo_trash = trash;
                        }
                    }
                    Err(error) => {
                        state.message = error;
                        state.error = true;
                    }
                }
            });
            browser.refresh();
        });
        if let Err(error) = result {
            self.change(|state| {
                state.busy = false;
                state.message = error;
                state.error = true;
            });
        }
    }
    pub fn create_folder(&self, name: String) {
        let path = self.snapshot().directory;
        self.operation(move || {
            fs::create_folder(&path, &name).map(|_| ("Folder created".into(), vec![]))
        });
    }
    pub fn rename(&self, name: String) {
        let selected = self.snapshot().selected;
        if selected.len() != 1 {
            return;
        }
        let path = selected[0].clone();
        self.operation(move || fs::rename_entry(&path, &name).map(|_| ("Renamed".into(), vec![])));
    }
    pub fn transfer(&self, paths: Vec<PathBuf>, mode: TransferMode) {
        let destination = self.snapshot().directory;
        self.operation(move || {
            fs::transfer_entries(&paths, &destination, mode).map(|paths| {
                (
                    format!(
                        "{} items {}",
                        paths.len(),
                        if mode == TransferMode::Copy {
                            "copied"
                        } else {
                            "moved"
                        }
                    ),
                    vec![],
                )
            })
        });
    }
    pub fn trash_selected(&self) {
        let paths = self.snapshot().selected;
        self.operation(move || {
            fs::trash_entries(&paths)
                .map(|items| (format!("{} items moved to Trash", items.len()), items))
        });
    }
    pub fn restore_selected(&self) {
        let state = self.snapshot();
        let items: Vec<_> = state
            .trash
            .unwrap_or_default()
            .into_iter()
            .filter(|item| state.selected.contains(&item.trashed_path))
            .collect();
        self.restore(items);
    }
    pub fn undo_trash(&self) {
        let items = self.snapshot().undo_trash;
        self.change(|state| state.undo_trash.clear());
        self.restore(items);
    }
    fn restore(&self, items: Vec<TrashItem>) {
        self.operation(move || {
            for item in &items {
                fs::restore_trash(item)?;
            }
            Ok((format!("{} items restored", items.len()), vec![]))
        });
    }
    pub fn preview(&self, path: PathBuf) {
        self.change(|state| state.preview_generation += 1);
        let generation = self.snapshot().preview_generation;
        let browser = self.clone();
        thread::spawn(move || {
            let result = std::fs::symlink_metadata(&path)
                .map_err(|e| e.to_string())
                .map(|metadata| {
                    let detail = format!(
                        "{}\n\n{}\n{} bytes\nModified: {}\n{}",
                        path.display(),
                        if metadata.is_dir() {
                            "Folder"
                        } else if metadata.is_symlink() {
                            "Symbolic link"
                        } else {
                            "File"
                        },
                        metadata.len(),
                        metadata
                            .modified()
                            .ok()
                            .map(crate::ui::date)
                            .unwrap_or_else(|| "Unknown".into()),
                        if metadata.permissions().readonly() {
                            "Read only"
                        } else {
                            "Writable"
                        }
                    );
                    let text = if metadata.is_file() && metadata.len() <= 64 * 1024 {
                        std::fs::read_to_string(&path)
                            .ok()
                            .filter(|s| !s.contains('\0'))
                    } else {
                        None
                    };
                    let image = if metadata.is_file() && metadata.len() <= 32 * 1024 * 1024 {
                        decode_preview(&path).ok()
                    } else {
                        None
                    };
                    Preview {
                        path,
                        detail,
                        text,
                        image,
                    }
                });
            browser.change(|state| {
                if state.preview_generation != generation {
                    return;
                }
                match result {
                    Ok(preview) => state.preview = Some(preview),
                    Err(error) => {
                        state.message = error;
                        state.error = true;
                    }
                }
            });
        });
    }
    pub fn close_preview(&self) {
        self.change(|state| {
            state.preview = None;
            state.preview_generation += 1;
        });
    }
    pub fn open(&self, path: PathBuf) {
        let browser = self.clone();
        thread::spawn(move || {
            match std::process::Command::new("xdg-open")
                .arg(&path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .output()
            {
                Ok(output) if output.status.success() => {}
                Ok(output) => browser.message(
                    format!(
                        "Could not open {}: {}",
                        path.display(),
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                    true,
                ),
                Err(error) => browser.message(
                    format!("Could not launch default application: {error}"),
                    true,
                ),
            }
        });
    }
}
pub fn initial_directory(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(home_directory)
}
fn decode_preview(
    path: &Path,
) -> Result<telorgon::graphics::render::ImageResource, Box<dyn std::error::Error>> {
    use telorgon::graphics::render::{
        ImageAlphaMode, ImageColorEncoding, ImagePixelFormat, ImageResource,
    };
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?.thumbnail(480, 320).to_rgba8();
    static NEXT_IMAGE: std::sync::atomic::AtomicU32 =
        std::sync::atomic::AtomicU32::new(0x7100_0000);
    Ok(ImageResource {
        image: telorgon::ImageId(NEXT_IMAGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)),
        content_version: 1,
        extent: telorgon::SizeI {
            width: image.width() as i32,
            height: image.height() as i32,
        },
        color_encoding: ImageColorEncoding::Srgb,
        alpha_mode: ImageAlphaMode::Straight,
        pixel_format: ImagePixelFormat::Rgba8,
        pixels: image.into_raw().into(),
    })
}
pub fn visible_entries(snapshot: &Snapshot, preferences: &Preferences) -> Vec<Entry> {
    let mut entries: Vec<_> = snapshot
        .entries
        .iter()
        .filter(|entry| preferences.show_hidden || !entry.name.starts_with('.'))
        .cloned()
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| {
                let order = match preferences.sort_by.as_str() {
                    "size" => a.size.cmp(&b.size),
                    "modified" => a.modified.cmp(&b.modified),
                    "type" => a.kind.cmp(&b.kind),
                    _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                };
                if preferences.descending {
                    order.reverse()
                } else {
                    order
                }
            })
            .then_with(|| a.name.cmp(&b.name))
    });
    entries
}
