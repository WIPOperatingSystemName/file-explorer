mod model;
mod ui;
use protocol::{PickerMode, PickerRequest, PickerResponse};
use std::{
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use telorgon::app::*;
use telorgon_file_explorer::{downloads, fs, preferences, protocol};

telorgon::asset_catalog! { pub mod assets = "assets"; }

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("telorgon-file-explorer: {error}");
            std::process::exit(2);
        }
    }
}
fn run() -> std::result::Result<i32, Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let mut path = None;
    let mut select = None;
    let mut properties = false;
    let mut request = None;
    let mut custom_title = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") => {
                println!(
                    "Telorgon Files\n\nUsage: telorgon-file-explorer [PATH]\n  --show PATH         Browse a folder\n  --select PATH       Select a file in its folder\n  --properties PATH   Show file properties\n  --pick              Open file picker (JSON response on stdout)\n  --save              Save file picker\n  --folder            Folder picker\n  --multiple          Allow multiple picker selections\n  --title TITLE       Picker title\n  --name NAME         Suggested save file name\n  --download URL      Choose a destination and download HTTP(S) file\n  --request-stdin     Read a bounded JSON picker request from stdin\n  --portal            Serve XDG FileChooser and FileManager1 on D-Bus\n\nRun the app with cargo run --release -- [PATH]."
                );
                return Ok(0);
            }
            Some("--portal") => {
                telorgon_file_explorer::portal::run()?;
                return Ok(0);
            }
            Some("--request-stdin") => {
                request = Some(protocol::read_request(io::stdin().lock())?);
                custom_title = true;
            }
            Some("--pick") => {
                let request = request.get_or_insert_with(PickerRequest::default);
                request.mode = PickerMode::Open;
                if !custom_title {
                    request.title = "Open a file".into();
                }
            }
            Some("--save") => {
                let request = request.get_or_insert_with(PickerRequest::default);
                request.mode = PickerMode::Save;
                if !custom_title {
                    request.title = "Save a file".into();
                }
            }
            Some("--folder") => {
                let request = request.get_or_insert_with(PickerRequest::default);
                request.mode = PickerMode::Folder;
                if !custom_title {
                    request.title = "Choose a folder".into();
                }
            }
            Some("--download") => {
                let url = args
                    .next()
                    .ok_or("--download requires a URL")?
                    .into_string()
                    .map_err(|_| "Download URL must be UTF-8")?;
                let name = downloads::suggested_filename(&url);
                let home = preferences::home_directory();
                let downloads = home.join("Downloads");
                if path.is_none() {
                    path = Some(if downloads.is_dir() { downloads } else { home });
                }
                let request = request.get_or_insert_with(PickerRequest::default);
                request.mode = PickerMode::Download;
                if !custom_title {
                    request.title = "Download a file".into();
                }
                request.source_url = Some(url);
                request.current_name.get_or_insert(name);
            }
            Some("--multiple") => {
                request.get_or_insert_with(PickerRequest::default).multiple = true;
            }
            Some("--title") => {
                custom_title = true;
                request.get_or_insert_with(PickerRequest::default).title = args
                    .next()
                    .ok_or("--title requires text")?
                    .into_string()
                    .map_err(|_| "Title must be UTF-8")?;
            }
            Some("--name") => {
                request
                    .get_or_insert_with(PickerRequest::default)
                    .current_name = Some(
                    args.next()
                        .ok_or("--name requires a file name")?
                        .into_string()
                        .map_err(|_| "File name must be UTF-8")?,
                );
            }
            Some("--show" | "--select" | "--properties") => {
                let target = PathBuf::from(args.next().ok_or("The option requires a path")?);
                properties = arg == "--properties";
                if arg == "--show" {
                    path = Some(target);
                } else {
                    path = Some(
                        target
                            .parent()
                            .unwrap_or(std::path::Path::new("/"))
                            .to_path_buf(),
                    );
                    select = Some(target);
                }
            }
            Some(value) if value.starts_with('-') => {
                return Err(format!("Unknown option: {value}").into());
            }
            _ => {
                if path.is_none() {
                    path = Some(
                        if let Some(uri) = arg.to_str().filter(|s| s.starts_with("file:")) {
                            protocol::uri_to_path(uri)?
                        } else {
                            PathBuf::from(arg)
                        },
                    );
                }
            }
        }
    }
    if let Some(request) = &request {
        request.validate()?;
        if path.is_none() {
            path = request.initial_path();
        }
    }
    let path = model::initial_directory(path);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    let completion: Arc<Mutex<Option<PickerResponse>>> = Arc::new(Mutex::new(None));
    let mut explorer = ui::Explorer::new(path, request.clone(), completion.clone());
    if let Some(path) = select.or_else(|| request.as_ref().and_then(|r| r.file_path())) {
        explorer.initial_selection(path, properties);
    }
    let title = request
        .as_ref()
        .map(|r| r.title.as_str())
        .unwrap_or("Telorgon Files");
    let workers = explorer.file_workers();
    let result = Application::gui("org.telorgon.FileExplorer", title)
        .renderer(Renderer::Auto)
        .assets(assets::bundle())
        .window(
            Window::new(title)
                .size(1240, 800)
                .minimum_size(800, 680)
                .content(explorer),
        )
        .run();
    // File writes already admitted by the UI finish before the process exits.
    workers.finish();
    result?;
    if request.is_some() {
        let response = completion
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(PickerResponse::cancelled);
        protocol::write_response(io::stdout().lock(), &response)?;
        return Ok(if response.cancelled { 1 } else { 0 });
    }
    Ok(0)
}
