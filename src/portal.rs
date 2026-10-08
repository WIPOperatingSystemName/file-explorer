//! Freedesktop FileChooser backend and FileManager1 service.
//! Each dialog is an isolated child process; the D-Bus executor never runs GUI code.
use crate::protocol::{
    self, FileFilter, FilterRule, PickerChoice, PickerMode, PickerRequest, PickerResponse,
};
use std::collections::HashMap;
use std::io::{self, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use zbus::message::Header;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.telorgon.FileExplorer";
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const MAX_DIALOGS: usize = 16;
type Options = HashMap<String, OwnedValue>;
type PortalResult = (u32, Options);
type WireFilter = (String, Vec<(u32, String)>);
type WireChoice = (String, String, Vec<(String, String)>, String);

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let connection = zbus::blocking::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(
            PORTAL_PATH,
            FileChooser {
                active: Arc::new(AtomicUsize::new(0)),
            },
        )?
        .serve_at(
            "/org/freedesktop/FileManager1",
            FileManager {
                active: Arc::new(AtomicUsize::new(0)),
            },
        )?
        .build()?;
    if let Err(error) = connection.request_name("org.freedesktop.FileManager1") {
        eprintln!("FileManager1 is already owned by another file manager: {error}");
    }
    eprintln!("Telorgon File Explorer portal ready on {BUS_NAME}");
    // zbus owns the background executor. Keep the connection alive for activation.
    loop {
        std::thread::park_timeout(Duration::from_secs(60));
        let _ = &connection;
    }
}

struct FileChooser {
    active: Arc<AtomicUsize>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooser {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }

    #[zbus(out_args("response", "results"))]
    async fn open_file(
        &self,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: Options,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<(u32, Options)> {
        self.choose(
            PickerMode::Open,
            handle,
            app_id,
            parent_window,
            title,
            options,
            connection,
            header,
        )
        .await
    }

    #[zbus(out_args("response", "results"))]
    async fn save_file(
        &self,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: Options,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<(u32, Options)> {
        self.choose(
            PickerMode::Save,
            handle,
            app_id,
            parent_window,
            title,
            options,
            connection,
            header,
        )
        .await
    }

    #[zbus(out_args("response", "results"))]
    async fn save_files(
        &self,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: Options,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<(u32, Options)> {
        self.choose(
            PickerMode::SaveFiles,
            handle,
            app_id,
            parent_window,
            title,
            options,
            connection,
            header,
        )
        .await
    }
}

impl FileChooser {
    #[allow(clippy::too_many_arguments)]
    async fn choose(
        &self,
        mode: PickerMode,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: Options,
        connection: &zbus::Connection,
        header: Header<'_>,
    ) -> zbus::fdo::Result<PortalResult> {
        let owner = authorize_portal(connection, &header).await?;
        if !handle
            .as_str()
            .starts_with("/org/freedesktop/portal/desktop/request/")
        {
            return Err(zbus::fdo::Error::InvalidArgs(
                "Invalid portal request object path.".into(),
            ));
        }
        let request = decode_request(mode, app_id, parent_window, title, &options)?;
        let payload = serde_json::to_vec(&request).map_err(failed)?;
        if payload.len() > protocol::MAX_PAYLOAD {
            return Err(zbus::fdo::Error::LimitsExceeded(
                "Picker request exceeds 256 KiB.".into(),
            ));
        }
        let _slot = ActiveSlot::acquire(self.active.clone())?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel_on_drop = CancelOnDrop(cancelled.clone());
        if !connection
            .object_server()
            .at(
                handle.clone(),
                Request {
                    owner: owner.clone(),
                    cancelled: cancelled.clone(),
                },
            )
            .await
            .map_err(failed)?
        {
            return Err(zbus::fdo::Error::InvalidArgs(
                "A request already exists at this handle.".into(),
            ));
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let worker_cancelled = cancelled.clone();
        let spawned = std::thread::Builder::new()
            .name("telorgon-file-dialog".into())
            .spawn(move || {
                let result = run_picker_child(payload, worker_cancelled);
                let _ = tx.send(result);
            });
        let result = if let Err(error) = spawned {
            Err(error)
        } else {
            let mut last_owner_check = Instant::now();
            loop {
                match rx.try_recv() {
                    Ok(result) => break result,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        break Err(io::Error::other("Picker worker stopped unexpectedly."));
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
                if last_owner_check.elapsed() >= Duration::from_secs(1) {
                    // An abandoned dialog must not outlive the frontend that owns it.
                    if !has_owner(connection, &owner).await {
                        cancelled.store(true, Ordering::Release);
                    }
                    last_owner_check = Instant::now();
                }
                async_io::Timer::after(Duration::from_millis(25)).await;
            }
        };
        let _ = connection
            .object_server()
            .remove::<Request, _>(handle)
            .await;
        if cancelled.load(Ordering::Acquire) {
            return Ok((1, Options::new()));
        }
        match result {
            Ok(response) => encode_response(&request, response),
            Err(error) => {
                eprintln!("File chooser failed: {error}");
                Ok((2, Options::new()))
            }
        }
    }
}

struct Request {
    owner: String,
    cancelled: Arc<AtomicBool>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    fn close(&self, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        if header
            .sender()
            .is_none_or(|sender| sender.as_str() != self.owner)
        {
            return Err(zbus::fdo::Error::AccessDenied(
                "Only the request owner can close this dialog.".into(),
            ));
        }
        self.cancelled.store(true, Ordering::Release);
        Ok(())
    }
}

async fn authorize_portal(
    connection: &zbus::Connection,
    header: &Header<'_>,
) -> zbus::fdo::Result<String> {
    let sender = header
        .sender()
        .ok_or_else(|| zbus::fdo::Error::AccessDenied("Missing D-Bus sender.".into()))?;
    let proxy = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(failed)?;
    let portal_name =
        zbus::names::BusName::try_from("org.freedesktop.portal.Desktop").map_err(failed)?;
    let owner = proxy.get_name_owner(portal_name).await.map_err(|_| {
        zbus::fdo::Error::AccessDenied(
            "FileChooser backend requires the desktop portal frontend.".into(),
        )
    })?;
    if owner.as_str() != sender.as_str() {
        return Err(zbus::fdo::Error::AccessDenied(
            "FileChooser backend calls must come from the desktop portal frontend.".into(),
        ));
    }
    Ok(sender.to_string())
}

async fn has_owner(connection: &zbus::Connection, owner: &str) -> bool {
    let Ok(proxy) = zbus::fdo::DBusProxy::new(connection).await else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from(owner) else {
        return false;
    };
    proxy.name_has_owner(name).await.unwrap_or(false)
}

struct ActiveSlot(Arc<AtomicUsize>);
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
impl ActiveSlot {
    fn acquire(active: Arc<AtomicUsize>) -> zbus::fdo::Result<Self> {
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_DIALOGS).then_some(count + 1)
            })
            .map_err(|_| {
                zbus::fdo::Error::LimitsExceeded("Too many active file dialogs.".into())
            })?;
        Ok(Self(active))
    }
}
impl Drop for ActiveSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn decode_request(
    mode: PickerMode,
    app_id: String,
    parent_window: String,
    title: String,
    options: &Options,
) -> zbus::fdo::Result<PickerRequest> {
    let mut request = PickerRequest {
        mode,
        app_id,
        parent_window,
        title,
        accept_label: option(options, "accept_label")?,
        modal: option(options, "modal")?.unwrap_or(true),
        multiple: if mode == PickerMode::Open {
            option(options, "multiple")?.unwrap_or(false)
        } else {
            false
        },
        directory: if mode == PickerMode::Open {
            option(options, "directory")?.unwrap_or(false)
        } else {
            mode == PickerMode::SaveFiles
        },
        current_name: option(options, "current_name")?,
        current_folder_bytes: option::<Vec<u8>>(options, "current_folder")?
            .map(strip_nul)
            .transpose()?,
        current_file_bytes: option::<Vec<u8>>(options, "current_file")?
            .map(strip_nul)
            .transpose()?,
        ..PickerRequest::default()
    };
    request.filters = option::<Vec<WireFilter>>(options, "filters")?
        .unwrap_or_default()
        .into_iter()
        .map(filter_from_wire)
        .collect();
    request.current_filter = option::<WireFilter>(options, "current_filter")?.map(filter_from_wire);
    request.choices = option::<Vec<WireChoice>>(options, "choices")?
        .unwrap_or_default()
        .into_iter()
        .map(|(id, label, options, selected)| PickerChoice {
            id,
            label,
            options,
            selected,
        })
        .collect();
    request.files_bytes = option::<Vec<Vec<u8>>>(options, "files")?
        .unwrap_or_default()
        .into_iter()
        .map(strip_nul)
        .collect::<zbus::fdo::Result<_>>()?;
    if let Some(folder) = request.folder_path() {
        request.current_folder = Some(folder.to_string_lossy().into_owned());
    }
    if let Some(file) = request.file_path() {
        request.current_file = Some(file.to_string_lossy().into_owned());
    }
    request
        .validate()
        .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
    Ok(request)
}

fn filter_from_wire((name, rules): WireFilter) -> FileFilter {
    FileFilter {
        name,
        rules: rules
            .into_iter()
            .map(|(kind, pattern)| FilterRule { kind, pattern })
            .collect(),
    }
}

fn strip_nul(mut bytes: Vec<u8>) -> zbus::fdo::Result<Vec<u8>> {
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    if bytes.contains(&0) {
        return Err(zbus::fdo::Error::InvalidArgs(
            "Paths and names cannot contain embedded NUL bytes.".into(),
        ));
    }
    Ok(bytes)
}

fn option<T>(options: &Options, name: &str) -> zbus::fdo::Result<Option<T>>
where
    T: TryFrom<OwnedValue>,
    T::Error: std::fmt::Display,
{
    options
        .get(name)
        .map(|value| {
            T::try_from(value.try_clone().map_err(failed)?)
                .map_err(|e| zbus::fdo::Error::InvalidArgs(format!("Invalid {name}: {e}")))
        })
        .transpose()
}

fn encode_response(
    request: &PickerRequest,
    response: PickerResponse,
) -> zbus::fdo::Result<PortalResult> {
    if response.cancelled {
        return Ok((1, Options::new()));
    }
    if response.current_filter.as_ref().is_some_and(|filter| {
        !request.filters.contains(filter) && request.current_filter.as_ref() != Some(filter)
    }) {
        return Err(failed("Picker returned an unknown selected filter."));
    }
    if response.choices.len() != request.choices.len() {
        return Err(failed("Picker returned incomplete choice values."));
    }
    let mut choice_ids = std::collections::HashSet::new();
    for (id, value) in &response.choices {
        let Some(choice) = request.choices.iter().find(|choice| &choice.id == id) else {
            return Err(failed("Picker returned an unknown choice."));
        };
        if !choice_ids.insert(id)
            || (choice.options.is_empty() && !matches!(value.as_str(), "true" | "false"))
            || (!choice.options.is_empty()
                && !choice.options.iter().any(|(option, _)| option == value))
        {
            return Err(failed("Picker returned an invalid choice value."));
        }
    }
    let mut validation_request = request.clone();
    validation_request.current_filter = response
        .current_filter
        .clone()
        .or_else(|| request.current_filter.clone());
    let paths = response
        .uris
        .iter()
        .map(|uri| protocol::uri_to_path(uri))
        .collect::<io::Result<Vec<_>>>()
        .map_err(failed)?;
    if request.mode == PickerMode::SaveFiles {
        if paths.len() != request.files_bytes.len().max(request.files.len())
            || paths.is_empty()
            || paths
                .iter()
                .any(|path| !path.parent().is_some_and(std::path::Path::is_dir) || path.is_dir())
        {
            return Err(failed("SaveFiles result has invalid destinations."));
        }
    } else {
        validation_request
            .validate_selection(&paths)
            .map_err(failed)?;
    }
    let uris = paths
        .iter()
        .map(|p| protocol::path_to_uri(p))
        .collect::<io::Result<Vec<_>>>()
        .map_err(failed)?;
    let mut results = Options::new();
    results.insert(
        "uris".into(),
        OwnedValue::try_from(Value::new(uris)).map_err(failed)?,
    );
    results.insert("writable".into(), OwnedValue::from(response.writable));
    results.insert(
        "choices".into(),
        OwnedValue::try_from(Value::new(response.choices)).map_err(failed)?,
    );
    if let Some(filter) = response.current_filter {
        let wire = (
            filter.name,
            filter
                .rules
                .into_iter()
                .map(|rule| (rule.kind, rule.pattern))
                .collect::<Vec<_>>(),
        );
        results.insert(
            "current_filter".into(),
            OwnedValue::try_from(Value::new(wire)).map_err(failed)?,
        );
    }
    Ok((0, results))
}

struct ReapChild {
    child: Child,
    reaped: bool,
}
impl ReapChild {
    fn wait(&mut self) -> io::Result<std::process::ExitStatus> {
        let result = self.child.wait();
        self.reaped = result.is_ok();
        result
    }
    fn kill(&mut self) {
        // Every launched process owns an isolated group. Close any transfer or
        // other helpers along with the dialog rather than leaving orphaned work.
        unsafe {
            libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = self.child.kill();
    }
}
impl Drop for ReapChild {
    fn drop(&mut self) {
        if !self.reaped {
            self.kill();
            let _ = self.child.wait();
        }
    }
}

fn run_picker_child(payload: Vec<u8>, cancelled: Arc<AtomicBool>) -> io::Result<PickerResponse> {
    run_picker_program(&std::env::current_exe()?, payload, cancelled)
}

fn run_picker_program(
    executable: &std::path::Path,
    payload: Vec<u8>,
    cancelled: Arc<AtomicBool>,
) -> io::Result<PickerResponse> {
    let child = Command::new(executable)
        .arg("--request-stdin")
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut process = ReapChild {
        child,
        reaped: false,
    };
    let stdout = process
        .child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Missing picker stdout."))?;
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("telorgon-picker-response".into())
        .spawn(move || {
            let _ = tx.send(protocol::read_response(stdout));
        })?;
    let mut stdin = process
        .child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("Missing picker stdin."))?;
    stdin.write_all(&payload)?;
    drop(stdin);
    loop {
        if cancelled.load(Ordering::Acquire) {
            process.kill();
            let _ = process.wait();
            return Ok(PickerResponse::cancelled());
        }
        if let Some(status) = process.child.try_wait()? {
            process.reaped = true;
            if status.code() == Some(1) {
                return Ok(PickerResponse::cancelled());
            }
            if !status.success() {
                return Err(io::Error::other(format!("Picker exited with {status}.")));
            }
            return rx
                .recv_timeout(Duration::from_secs(2))
                .map_err(|e| io::Error::other(format!("Missing picker result: {e}")))?;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

struct FileManager {
    active: Arc<AtomicUsize>,
}
#[zbus::interface(name = "org.freedesktop.FileManager1")]
impl FileManager {
    fn show_items(&self, uris: Vec<String>, startup_id: String) -> zbus::fdo::Result<()> {
        launch_explorer("--select", uris, startup_id, &self.active)
    }
    fn show_folders(&self, uris: Vec<String>, startup_id: String) -> zbus::fdo::Result<()> {
        launch_explorer("--show", uris, startup_id, &self.active)
    }
    fn show_item_properties(&self, uris: Vec<String>, startup_id: String) -> zbus::fdo::Result<()> {
        launch_explorer("--properties", uris, startup_id, &self.active)
    }
}

fn launch_explorer(
    action: &str,
    uris: Vec<String>,
    startup_id: String,
    active: &Arc<AtomicUsize>,
) -> zbus::fdo::Result<()> {
    if uris.is_empty() || uris.len() > 32 {
        return Err(zbus::fdo::Error::InvalidArgs(
            "Provide between one and 32 local file URIs.".into(),
        ));
    }
    let paths = uris
        .iter()
        .map(|uri| protocol::uri_to_path(uri))
        .collect::<io::Result<Vec<_>>>()
        .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
    if paths
        .iter()
        .any(|p| std::fs::symlink_metadata(p).is_err() || (action == "--show" && !p.is_dir()))
    {
        return Err(zbus::fdo::Error::InvalidArgs(
            "The requested file or folder does not exist.".into(),
        ));
    }
    let executable = std::env::current_exe().map_err(failed)?;
    for path in paths {
        let slot = ActiveSlot::acquire(active.clone())?;
        let mut command = Command::new(&executable);
        command
            .arg(action)
            .arg(path)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        if !startup_id.is_empty() {
            command.env("DESKTOP_STARTUP_ID", &startup_id);
        }
        let child = command.spawn().map_err(failed)?;
        let mut process = ReapChild {
            child,
            reaped: false,
        };
        // Reap after the explorer window closes, without blocking the D-Bus method.
        std::thread::Builder::new()
            .name("telorgon-file-manager-window".into())
            .spawn(move || {
                let _slot = slot;
                let _ = process.wait();
            })
            .map_err(failed)?;
    }
    Ok(())
}

fn failed(error: impl std::fmt::Display) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portal_options_decode_nul_paths_filters_and_choices() {
        let mut options = Options::new();
        options.insert(
            "current_folder".into(),
            OwnedValue::try_from(Value::new(b"/tmp/\xff\0".to_vec())).unwrap(),
        );
        options.insert("multiple".into(), OwnedValue::from(true));
        options.insert(
            "filters".into(),
            OwnedValue::try_from(Value::new(vec![(
                "Images".to_string(),
                vec![(0_u32, "*.png".to_string())],
            )]))
            .unwrap(),
        );
        options.insert(
            "choices".into(),
            OwnedValue::try_from(Value::new(vec![(
                "readonly".to_string(),
                "Read only".to_string(),
                Vec::<(String, String)>::new(),
                "true".to_string(),
            )]))
            .unwrap(),
        );
        let request = decode_request(
            PickerMode::Open,
            "app.test".into(),
            String::new(),
            "Pick".into(),
            &options,
        )
        .unwrap();
        assert_eq!(request.current_folder_bytes.unwrap(), b"/tmp/\xff");
        assert!(request.multiple);
        assert_eq!(request.filters[0].rules[0].pattern, "*.png");
        assert_eq!(request.choices[0].selected, "true");
    }
    #[test]
    fn malformed_option_types_and_unsafe_names_fail() {
        let mut options = Options::new();
        options.insert("multiple".into(), OwnedValue::from(1_u32));
        assert!(
            decode_request(
                PickerMode::Open,
                String::new(),
                String::new(),
                String::new(),
                &options
            )
            .is_err()
        );
        options.clear();
        options.insert(
            "files".into(),
            OwnedValue::try_from(Value::new(vec![b"../secret\0".to_vec()])).unwrap(),
        );
        assert!(
            decode_request(
                PickerMode::SaveFiles,
                String::new(),
                String::new(),
                String::new(),
                &options
            )
            .is_err()
        );
    }
    #[test]
    fn active_slots_are_bounded_and_released() {
        let active = Arc::new(AtomicUsize::new(0));
        let slots: Vec<_> = (0..MAX_DIALOGS)
            .map(|_| ActiveSlot::acquire(active.clone()).unwrap())
            .collect();
        assert!(ActiveSlot::acquire(active.clone()).is_err());
        drop(slots);
        assert_eq!(active.load(Ordering::Acquire), 0);
    }
    #[test]
    fn only_request_owner_can_cancel() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let request = Request {
            owner: ":1.42".into(),
            cancelled: cancelled.clone(),
        };
        let outsider = zbus::Message::method_call("/request", "Close")
            .unwrap()
            .sender(":1.43")
            .unwrap()
            .build(&())
            .unwrap();
        assert!(request.close(outsider.header()).is_err());
        assert!(!cancelled.load(Ordering::Acquire));
        let owner = zbus::Message::method_call("/request", "Close")
            .unwrap()
            .sender(":1.42")
            .unwrap()
            .build(&())
            .unwrap();
        request.close(owner.header()).unwrap();
        assert!(cancelled.load(Ordering::Acquire));
    }
    #[test]
    fn switched_filter_and_choice_values_are_validated() {
        let folder =
            std::env::temp_dir().join(format!("telorgon-portal-response-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("image.png");
        std::fs::write(&path, b"png").unwrap();
        let text = FileFilter {
            name: "Text".into(),
            rules: vec![FilterRule {
                kind: 0,
                pattern: "*.txt".into(),
            }],
        };
        let image = FileFilter {
            name: "Image".into(),
            rules: vec![FilterRule {
                kind: 0,
                pattern: "*.png".into(),
            }],
        };
        let request = PickerRequest {
            filters: vec![text.clone(), image.clone()],
            current_filter: Some(text),
            choices: vec![PickerChoice {
                id: "readonly".into(),
                label: "Read only".into(),
                options: Vec::new(),
                selected: "false".into(),
            }],
            ..Default::default()
        };
        let mut selection_request = request.clone();
        selection_request.current_filter = Some(image);
        let response = protocol::selected_response(&selection_request, &[path]).unwrap();
        assert_eq!(encode_response(&request, response.clone()).unwrap().0, 0);
        let mut bad = response;
        bad.choices[0].1 = "invalid".into();
        assert!(encode_response(&request, bad).is_err());
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn dbus_method_signatures_match_the_portal_contract() {
        use zbus::object_server::Interface;
        let chooser = FileChooser {
            active: Arc::new(AtomicUsize::new(0)),
        };
        let mut xml = String::new();
        chooser.introspect_to_writer(&mut xml, 0);
        for name in ["OpenFile", "SaveFile", "SaveFiles"] {
            assert!(xml.contains(&format!("<method name=\"{name}\">")), "{xml}");
        }
        assert!(xml.contains("type=\"a{sv}\" direction=\"out\""), "{xml}");
        assert!(xml.contains("<property name=\"version\""), "{xml}");
        let request = Request {
            owner: ":1.1".into(),
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        xml.clear();
        request.introspect_to_writer(&mut xml, 0);
        assert!(xml.contains("org.freedesktop.impl.portal.Request"));
        assert!(xml.contains("<method name=\"Close\">"));
    }
    #[test]
    fn cancelled_child_is_killed_and_reaped_promptly() {
        use std::os::unix::fs::PermissionsExt;
        let script =
            std::env::temp_dir().join(format!("telorgon-picker-cancel-{}.sh", std::process::id()));
        std::fs::write(&script, b"#!/bin/sh\nexec sleep 10\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let cancel_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            flag.store(true, Ordering::Release);
        });
        let start = Instant::now();
        let result = run_picker_program(&script, b"{}".to_vec(), cancelled).unwrap();
        assert!(result.cancelled);
        assert!(start.elapsed() < Duration::from_secs(2));
        cancel_thread.join().unwrap();
        std::fs::remove_file(script).unwrap();
    }
}
