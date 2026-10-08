use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;

struct TestFolder(PathBuf);

impl TestFolder {
    fn new() -> Self {
        let staging = Staging::new(&std::env::temp_dir()).unwrap();
        let path = staging.directory.clone();
        // TestFolder takes ownership of this unique test directory.
        std::mem::forget(staging);
        Self(path)
    }

    fn assert_no_partial_files(&self) {
        assert!(!fs::read_dir(&self.0).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".telorgon-download-")
        }));
    }
}

impl Drop for TestFolder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn serve(body: Vec<u8>, slow: bool, status: &str) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let status = status.to_string();
    let server = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut connection = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "download never connected");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("local test server failed: {error}"),
            }
        };
        connection
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = [0; 4096];
        let _ = connection.read(&mut request);
        let header = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        if connection.write_all(header.as_bytes()).is_err() {
            return;
        }
        for chunk in body.chunks(1024) {
            if connection.write_all(chunk).is_err() {
                break;
            }
            if slow {
                thread::sleep(Duration::from_millis(25));
            }
        }
    });
    (format!("http://{address}/test.bin"), server)
}

fn await_job(manager: &DownloadManager, id: u64, finished: bool) -> DownloadJob {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(job) = manager.jobs().snapshot().iter().find(|job| job.id == id) {
            if if finished {
                job.status.is_finished()
            } else {
                job.received_bytes > 0
            } {
                return job.clone();
            }
        }
        assert!(
            Instant::now() < deadline,
            "download did not reach expected state"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn validates_protocol_and_derives_safe_filename() {
    assert!(validate_url("file:///etc/passwd").is_err());
    assert!(validate_url("ftp://example.com/file").is_err());
    assert!(validate_url("http:///missing-host").is_err());
    assert!(validate_url("http://example.com/a b").is_err());
    assert!(validate_url("HTTPS://example.com/archive%20one.zip").is_ok());
    assert_eq!(
        suggested_filename("https://example.com/archive%20one.zip?token=1#top"),
        "archive one.zip"
    );
    assert_eq!(suggested_filename("https://example.com/%2e%2e"), "download");
    assert_eq!(suggested_filename("https://example.com/"), "download");
    assert_eq!(suggested_filename("https://example.com"), "download");
    assert_eq!(
        suggested_filename("https://example.com/%2e%2e%2Fsecret%5cfile%00"),
        "_secret_file_"
    );
    assert!(suggested_filename(&format!("https://example.com/{}", "ö".repeat(200))).len() <= 240);
}

#[test]
fn successful_http_download_publishes_complete_file_and_progress() {
    let folder = TestFolder::new();
    let body = b"a real local HTTP transfer\n".repeat(512);
    let (url, server) = serve(body.clone(), false, "200 OK");
    let manager = DownloadManager::new();
    let destination = folder.0.join("complete.bin");
    let id = manager.start(url, destination.clone()).unwrap();
    let job = await_job(&manager, id, true);
    assert_eq!(job.status, DownloadStatus::Completed);
    assert_eq!(job.received_bytes, body.len() as u64);
    assert_eq!(job.total_bytes, Some(body.len() as u64));
    assert_eq!(fs::read(destination).unwrap(), body);
    folder.assert_no_partial_files();
    server.join().unwrap();
}

#[test]
fn protects_files_created_while_transfer_is_running() {
    let folder = TestFolder::new();
    let (url, server) = serve(vec![42; 64 * 1024], true, "200 OK");
    let manager = DownloadManager::new();
    let destination = folder.0.join("preserved.bin");
    let id = manager.start(url, destination.clone()).unwrap();
    let progress = await_job(&manager, id, false);
    assert_eq!(progress.status, DownloadStatus::Downloading);
    assert!(progress.received_bytes < progress.total_bytes.unwrap());
    assert!(
        !destination.exists(),
        "partial file became publicly visible"
    );
    fs::write(&destination, "keep this existing file").unwrap();
    let job = await_job(&manager, id, true);
    assert!(matches!(job.status, DownloadStatus::Failed(_)));
    assert_eq!(
        fs::read_to_string(destination).unwrap(),
        "keep this existing file"
    );
    folder.assert_no_partial_files();
    server.join().unwrap();
}

#[test]
fn cancellation_removes_partial_file_without_publishing_destination() {
    let folder = TestFolder::new();
    let (url, server) = serve(vec![42; 512 * 1024], true, "200 OK");
    let manager = DownloadManager::new();
    let destination = folder.0.join("cancelled.bin");
    let id = manager.start(url, destination.clone()).unwrap();
    await_job(&manager, id, false);
    assert!(manager.cancel(id));
    assert_eq!(
        await_job(&manager, id, true).status,
        DownloadStatus::Cancelled
    );
    assert!(!destination.exists());
    folder.assert_no_partial_files();
    server.join().unwrap();
}

#[test]
fn dropping_manager_cleans_up_running_transfer() {
    let folder = TestFolder::new();
    let (url, server) = serve(vec![42; 512 * 1024], true, "200 OK");
    let manager = DownloadManager::new();
    let destination = folder.0.join("stopped.bin");
    let id = manager.start(url, destination.clone()).unwrap();
    await_job(&manager, id, false);
    drop(manager);
    assert!(!destination.exists());
    folder.assert_no_partial_files();
    server.join().unwrap();
}

#[test]
fn existing_destination_requires_explicit_overwrite_and_http_errors_do_not_save() {
    let folder = TestFolder::new();
    let manager = DownloadManager::new();
    let destination = folder.0.join("existing.bin");
    fs::write(&destination, "original").unwrap();
    let id = manager
        .start("https://example.invalid/file".into(), destination.clone())
        .unwrap();
    assert!(matches!(
        await_job(&manager, id, true).status,
        DownloadStatus::Failed(_)
    ));
    assert_eq!(fs::read_to_string(&destination).unwrap(), "original");

    let (url, server) = serve(b"new contents".to_vec(), false, "200 OK");
    let id = manager
        .start_with_overwrite(url, destination.clone(), true)
        .unwrap();
    assert_eq!(
        await_job(&manager, id, true).status,
        DownloadStatus::Completed
    );
    assert_eq!(fs::read_to_string(&destination).unwrap(), "new contents");
    server.join().unwrap();

    let (url, server) = serve(b"server error".to_vec(), false, "404 Not Found");
    let missing = folder.0.join("missing.bin");
    let id = manager.start(url, missing.clone()).unwrap();
    assert!(matches!(
        await_job(&manager, id, true).status,
        DownloadStatus::Failed(_)
    ));
    assert!(!missing.exists());
    folder.assert_no_partial_files();
    server.join().unwrap();
}
