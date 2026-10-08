//! HTTP downloads with reactive progress and atomic destination publication.
//!
//! Curl handles TLS and HTTP on a background worker. All curl arguments are
//! passed directly, and its user configuration is disabled. A private staging
//! directory in the destination folder keeps incomplete files out of the way.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use telorgon::authoring::compose::{Signal, SignalWriter};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Completed,
    Cancelled,
    Failed(String),
}

impl DownloadStatus {
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadJob {
    pub id: u64,
    pub url: String,
    pub destination: PathBuf,
    pub filename: String,
    pub status: DownloadStatus,
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub bytes_per_second: u64,
}

enum Publication {
    Jobs(Vec<DownloadJob>),
    Shutdown,
}

struct Shared {
    jobs: Mutex<Vec<DownloadJob>>,
    controls: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    publications: mpsc::Sender<Publication>,
}

impl Shared {
    fn change(&self, change: impl FnOnce(&mut Vec<DownloadJob>)) {
        // Sending under the jobs lock preserves event order. Only the publisher
        // calls SignalWriter, so signal callbacks can reenter the manager safely.
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut jobs);
        let _ = self.publications.send(Publication::Jobs(jobs.clone()));
    }

    fn update(&self, id: u64, change: impl FnOnce(&mut DownloadJob)) {
        self.change(|jobs| {
            if let Some(job) = jobs.iter_mut().find(|job| job.id == id) {
                change(job);
            }
        });
    }
}

/// Keep this manager alive for as long as its downloads are needed. Dropping it
/// cancels every active transfer, reaps curl processes, and removes partial files.
pub struct DownloadManager {
    shared: Arc<Shared>,
    signal: Signal<Vec<DownloadJob>>,
    next_id: AtomicU64,
    workers: Mutex<Vec<JoinHandle<()>>>,
    publisher: Option<JoinHandle<()>>,
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloadManager {
    pub fn new() -> Self {
        let (signal, writer) = Signal::new(Vec::new());
        let (sender, receiver) = mpsc::channel();
        let publisher = thread::Builder::new()
            .name("download-publication".into())
            .spawn(move || publish_changes(receiver, writer))
            .expect("could not create download publication worker");
        Self {
            shared: Arc::new(Shared {
                jobs: Mutex::new(Vec::new()),
                controls: Mutex::new(HashMap::new()),
                publications: sender,
            }),
            signal,
            next_id: AtomicU64::new(1),
            workers: Mutex::new(Vec::new()),
            publisher: Some(publisher),
        }
    }

    pub fn jobs(&self) -> Signal<Vec<DownloadJob>> {
        self.signal.clone()
    }

    /// Destination is chosen by the save prompt. Existing files are preserved.
    /// Only validation and thread dispatch happen on the calling/UI thread.
    pub fn start(&self, url: String, destination: PathBuf) -> Result<u64, String> {
        self.start_with_overwrite(url, destination, false)
    }

    /// Use `overwrite = true` only after the user confirms replacement.
    pub fn start_with_overwrite(
        &self,
        url: String,
        destination: PathBuf,
        overwrite: bool,
    ) -> Result<u64, String> {
        let url = validate_url(&url)?;
        let filename = destination
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "Choose a filename for the download.".to_string())?
            .to_string_lossy()
            .into_owned();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let cancelled = Arc::new(AtomicBool::new(false));
        self.shared
            .controls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, cancelled.clone());
        self.shared.change(|jobs| {
            jobs.push(DownloadJob {
                id,
                url: url.clone(),
                destination: destination.clone(),
                filename,
                status: DownloadStatus::Queued,
                received_bytes: 0,
                total_bytes: None,
                bytes_per_second: 0,
            });
        });
        let shared = self.shared.clone();
        let worker = thread::Builder::new()
            .name(format!("download-{id}"))
            .spawn(move || {
                run_worker(id, &url, &destination, overwrite, &cancelled, &shared);
                shared
                    .controls
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
            });
        match worker {
            Ok(worker) => {
                let mut workers = self.workers.lock().unwrap_or_else(|e| e.into_inner());
                // Reap completed threads without waiting for active transfers.
                let mut index = 0;
                while index < workers.len() {
                    if workers[index].is_finished() {
                        let finished = workers.swap_remove(index);
                        let _ = finished.join();
                    } else {
                        index += 1;
                    }
                }
                workers.push(worker);
                Ok(id)
            }
            Err(error) => {
                self.shared
                    .controls
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                let error = format!("Could not start the download: {error}");
                self.shared
                    .update(id, |job| job.status = DownloadStatus::Failed(error.clone()));
                Err(error)
            }
        }
    }

    pub fn cancel(&self, id: u64) -> bool {
        let controls = self
            .shared
            .controls
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(control) = controls.get(&id) {
            control.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }

    pub fn clear_finished(&self) {
        self.shared
            .change(|jobs| jobs.retain(|job| !job.status.is_finished()));
    }
}

impl Drop for DownloadManager {
    fn drop(&mut self) {
        for control in self
            .shared
            .controls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            control.store(true, Ordering::Release);
        }
        for worker in self
            .workers
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
        {
            let _ = worker.join();
        }
        let _ = self.shared.publications.send(Publication::Shutdown);
        if let Some(publisher) = self.publisher.take() {
            let _ = publisher.join();
        }
    }
}

fn publish_changes(receiver: mpsc::Receiver<Publication>, writer: SignalWriter<Vec<DownloadJob>>) {
    while let Ok(publication) = receiver.recv() {
        match publication {
            Publication::Jobs(jobs) => {
                writer.publish_if_changed(jobs);
            }
            Publication::Shutdown => break,
        }
    }
}

fn run_worker(
    id: u64,
    url: &str,
    destination: &Path,
    overwrite: bool,
    cancelled: &AtomicBool,
    shared: &Shared,
) {
    let result = transfer(id, url, destination, overwrite, cancelled, shared);
    shared.update(id, |job| {
        job.bytes_per_second = 0;
        job.status = match result {
            Ok(()) => DownloadStatus::Completed,
            Err(TransferError::Cancelled) => DownloadStatus::Cancelled,
            Err(TransferError::Failed(error)) => DownloadStatus::Failed(error),
        };
    });
}

enum TransferError {
    Cancelled,
    Failed(String),
}

impl From<io::Error> for TransferError {
    fn from(error: io::Error) -> Self {
        Self::Failed(error.to_string())
    }
}

fn transfer(
    id: u64,
    url: &str,
    destination: &Path,
    overwrite: bool,
    cancelled: &AtomicBool,
    shared: &Shared,
) -> Result<(), TransferError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(TransferError::Cancelled);
    }
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(TransferError::Failed(format!(
            "The destination folder does not exist: {}",
            parent.display()
        )));
    }
    if !overwrite && fs::symlink_metadata(destination).is_ok() {
        return Err(TransferError::Failed(
            "A file already exists at this destination. Choose another name or confirm replacement."
                .into(),
        ));
    }
    let staging = Staging::new(parent)?;
    let payload_path = staging.directory.join("payload.part");
    let payload = private_file(&payload_path)?;
    let headers_path = staging.directory.join("headers");
    let stderr_path = staging.directory.join("error");
    let stderr = private_file(&stderr_path)?;
    let mut command = Command::new("/usr/bin/curl");
    command
        .arg("--disable")
        .args([
            "--globoff",
            "--fail",
            "--location",
            "--max-redirs",
            "10",
            "--proto",
            "=http,https",
            "--proto-redir",
            "=http,https",
            "--connect-timeout",
            "30",
            "--speed-time",
            "60",
            "--speed-limit",
            "1",
            "--silent",
            "--show-error",
            "--output",
            "-",
            "--dump-header",
        ])
        .arg(&headers_path)
        .arg("--url")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::from(payload.try_clone()?))
        .stderr(Stdio::from(stderr));
    let mut child = TransferProcess(command.spawn().map_err(|error| {
        TransferError::Failed(format!("Could not start /usr/bin/curl: {error}"))
    })?);
    shared.update(id, |job| job.status = DownloadStatus::Downloading);
    let mut last_bytes = 0;
    let mut last_progress = Instant::now();
    let completion = loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(TransferError::Cancelled);
        }
        let status = match child.0.try_wait() {
            Ok(status) => status,
            Err(error) => return Err(error.into()),
        };
        let bytes = payload.metadata()?.len();
        let total = content_length(&headers_path);
        let now = Instant::now();
        let elapsed = now.duration_since(last_progress).as_secs_f64();
        let rate = if elapsed > 0.0 {
            ((bytes.saturating_sub(last_bytes)) as f64 / elapsed) as u64
        } else {
            0
        };
        shared.update(id, |job| {
            job.received_bytes = bytes;
            job.total_bytes = total;
            job.bytes_per_second = rate;
        });
        last_progress = now;
        last_bytes = bytes;
        if let Some(status) = status {
            break status;
        }
        thread::sleep(Duration::from_millis(100));
    };
    if cancelled.load(Ordering::Acquire) {
        return Err(TransferError::Cancelled);
    }
    if !completion.success() {
        let mut error = String::new();
        let _ = File::open(&stderr_path)
            .and_then(|file| file.take(16 * 1024).read_to_string(&mut error));
        let error = error.trim();
        return Err(TransferError::Failed(if error.is_empty() {
            format!("The download failed ({completion}).")
        } else {
            error.to_string()
        }));
    }
    // Flush the entire file before making its final name visible.
    payload.sync_all()?;
    if cancelled.load(Ordering::Acquire) {
        return Err(TransferError::Cancelled);
    }
    if overwrite {
        fs::rename(&payload_path, destination)?;
    } else {
        // A hard link publishes atomically and refuses existing destinations,
        // including ones that appeared while this transfer was running.
        fs::hard_link(&payload_path, destination).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                TransferError::Failed(
                    "A file appeared at this destination during the download. It was preserved; choose another filename.".into(),
                )
            } else {
                TransferError::Failed(format!("Could not save the completed download: {error}"))
            }
        })?;
    }
    let bytes = payload.metadata()?.len();
    shared.update(id, |job| {
        job.received_bytes = bytes;
        if job.total_bytes.is_none() {
            job.total_bytes = Some(bytes);
        }
    });
    Ok(())
}

static NEXT_STAGING: AtomicU64 = AtomicU64::new(1);

// Child does not terminate its process on Drop. This guard also covers filesystem
// errors and unwinding, so no exit path leaves curl running or unreaped.
struct TransferProcess(Child);

impl Drop for TransferProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Staging {
    directory: PathBuf,
}

impl Staging {
    fn new(parent: &Path) -> io::Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..32 {
            let serial = NEXT_STAGING.fetch_add(1, Ordering::Relaxed);
            let directory = parent.join(format!(
                ".telorgon-download-{}-{stamp:x}-{serial:x}.part",
                std::process::id()
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            builder.mode(0o700);
            match builder.create(&directory) {
                Ok(()) => return Ok(Self { directory }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Could not allocate a private download staging directory",
        ))
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}

fn content_length(path: &Path) -> Option<u64> {
    let mut headers = String::new();
    // Bound header memory. Curl itself remains responsible for HTTP parsing.
    File::open(path)
        .ok()?
        .take(256 * 1024)
        .read_to_string(&mut headers)
        .ok()?;
    let mut total = None;
    for line in headers.lines() {
        if line.starts_with("HTTP/") {
            total = None;
        } else if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                total = value.trim().parse().ok();
            } else if name.eq_ignore_ascii_case("transfer-encoding")
                && value.trim().eq_ignore_ascii_case("chunked")
            {
                total = None;
            }
        }
    }
    total
}

fn validate_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    let (scheme, remainder) = url
        .split_once("://")
        .ok_or_else(|| "Enter a complete HTTP or HTTPS URL.".to_string())?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err("Downloads only support HTTP and HTTPS URLs.".into());
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("The URL contains invalid whitespace. Encode spaces as %20.".into());
    }
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.ends_with('@') {
        return Err("The URL needs a server hostname.".into());
    }
    Ok(url.to_string())
}

/// Derive one safe, human-readable filename from a URL path. Encoded directory
/// separators and dot-only names cannot escape the destination chosen by a user.
pub fn suggested_filename(url: &str) -> String {
    let without_suffix = url.split(['?', '#']).next().unwrap_or_default();
    let path = without_suffix
        .split_once("://")
        .map(|(_, remainder)| {
            remainder
                .find('/')
                .map(|index| &remainder[index..])
                .unwrap_or_default()
        })
        .unwrap_or(without_suffix);
    let segment = path.rsplit('/').next().unwrap_or_default();
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                decoded.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    let decoded = String::from_utf8_lossy(&decoded);
    let mut filename = String::new();
    for character in decoded.chars() {
        let character = if character.is_control() || matches!(character, '/' | '\\' | ':') {
            '_'
        } else {
            character
        };
        if filename.len() + character.len_utf8() > 240 {
            break;
        }
        filename.push(character);
    }
    let filename = filename.trim().trim_start_matches('.').trim();
    if filename.is_empty() {
        "download".into()
    } else {
        filename.into()
    }
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[path = "downloads/tests.rs"]
mod tests;
