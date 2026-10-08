use super::*;
use telorgon::{
    input::{ButtonState, LogicalKey, Modifiers, NamedKey},
    ui::{UiEvent, UiEventKind},
};
impl Explorer {
    pub(super) fn dismiss_dialog(&mut self) {
        self.dialog = Dialog::None;
        self.name.blur();
        self.url.blur();
    }

    pub(super) fn control_input(&mut self, event: &UiEvent) -> bool {
        self.keyboard.set(event.modifiers);
        let UiEventKind::Input(telorgon::InputEvent::Key(key)) = &event.kind else {
            return false;
        };
        let key = self.keyboard.update(key);
        if key.state == ButtonState::Pressed
            && matches!(self.dialog, Dialog::None)
            && key.modifiers.contains(Modifiers::CONTROL)
            && !key
                .modifiers
                .intersects(Modifiers::ALT.union(Modifiers::SUPER))
            && matches!(&key.logical_key, LogicalKey::Character(value) if value.as_str().eq_ignore_ascii_case("l"))
        {
            if !key.repeat {
                self.location.request_focus();
            }
            return true;
        }
        if key.state == ButtonState::Pressed
            && key.logical_key == LogicalKey::Named(NamedKey::Escape)
        {
            if !matches!(self.dialog, Dialog::None) {
                self.dismiss_dialog();
                return true;
            }
            if self.picker_mode() {
                self.cancel();
                return true;
            }
        }
        false
    }

    pub(super) fn submit_dialog(&mut self) {
        if self.browser().snapshot().busy {
            return;
        }
        match self.dialog {
            Dialog::NewFolder | Dialog::Rename => {
                let name = self.name.text();
                if let Err(error) = crate::fs::validate_name(&name) {
                    self.browser().message(error, true);
                    return;
                }
                if matches!(self.dialog, Dialog::NewFolder) {
                    self.browser().create_folder(name);
                } else {
                    self.browser().rename(name);
                }
                self.dismiss_dialog();
            }
            Dialog::Download => {
                let url = self.url.text();
                let name = if self.name.text().is_empty() {
                    crate::downloads::suggested_filename(&url)
                } else {
                    self.name.text()
                };
                if let Err(error) = crate::fs::validate_name(&name) {
                    self.browser().message(error, true);
                    return;
                }
                match self
                    .downloads
                    .start(url, self.browser().snapshot().directory.join(name))
                {
                    Ok(_) => {
                        self.dismiss_dialog();
                        self.downloads_open = true;
                    }
                    Err(error) => self.browser().message(error, true),
                }
            }
            _ => {}
        }
    }

    pub(super) fn submit_picker(&mut self) {
        if self.picker_mode() && matches!(self.dialog, Dialog::None) {
            self.accept(false);
        }
    }

    pub(super) fn bind_inputs(&self) {
        for value in [
            &self.location,
            &self.search,
            &self.name,
            &self.picker_name,
            &self.url,
        ] {
            let browser = self.browser().clone();
            value.on_error(move |error| browser.message(error, true));
        }
        let browser = self.browser().clone();
        let location = Arc::downgrade(&self.location.0);
        let clear_search = self.search.clone();
        self.location.on_submit(move |value| {
            match location_path(&value, &browser.snapshot().directory) {
                Ok(path) => match std::fs::metadata(&path) {
                    Ok(metadata) if metadata.is_dir() => {
                        browser.navigate(path);
                        if let Some(location) = location.upgrade() {
                            let location = FieldValue(location);
                            location.set(browser.snapshot().directory.to_string_lossy());
                            location.finish_editing();
                        }
                        clear_search.set("");
                    }
                    Ok(_) => browser.message("Choose a folder for the location.", true),
                    Err(error) => browser.message(format!("Cannot open location: {error}"), true),
                },
                Err(error) => browser.message(error, true),
            }
        });
        let browser = self.browser().clone();
        let include_hidden = self.preferences.show_hidden;
        self.search
            .on_submit(move |query| browser.search(query, include_hidden));
    }
    pub(super) fn new_tab(&mut self) {
        let path = self.browser().snapshot().directory;
        self.tabs.push(Browser::new(path, self.workers.clone()));
        self.active = self.tabs.len() - 1;
        self.location
            .set(self.browser().snapshot().directory.to_string_lossy());
        self.search.set("");
        self.bind_inputs();
    }
    pub(super) fn close_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.tabs.remove(self.active);
            self.active = self.active.min(self.tabs.len() - 1);
            self.location
                .set(self.browser().snapshot().directory.to_string_lossy());
            self.search.set(self.browser().snapshot().query);
            self.bind_inputs();
        }
    }
    pub(super) fn go_history(&mut self, forward: bool) {
        self.browser().history(forward);
        self.location
            .set(self.browser().snapshot().directory.to_string_lossy());
        self.search.set("");
        self.bind_inputs();
    }
    pub(super) fn copy(&mut self, mode: TransferMode) {
        let paths = self.browser().snapshot().selected;
        if !paths.is_empty() {
            self.browser().message(
                format!(
                    "{} items ready to {}",
                    paths.len(),
                    if mode == TransferMode::Copy {
                        "copy"
                    } else {
                        "move"
                    }
                ),
                false,
            );
            self.clipboard = Some((paths, mode));
        }
    }
    pub(super) fn paste(&mut self) {
        if let Some((paths, mode)) = self.clipboard.clone() {
            self.browser().transfer(paths, mode);
            if mode == TransferMode::Move {
                self.clipboard = None;
            }
        }
    }
    pub(super) fn rename_prompt(&mut self) {
        let state = self.browser().snapshot();
        if state.selected.len() == 1 {
            self.name.set(
                state.selected[0]
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
            );
            self.dialog = Dialog::Rename;
        }
    }
    pub(super) fn open_selected(&mut self) {
        let state = self.browser().snapshot();
        if state.trash.is_some() {
            return;
        }
        if let Some(path) = state.selected.first() {
            if path.is_dir() {
                self.navigate(path.clone());
            } else if self.picker_mode() {
                if self.request.as_ref().is_some_and(|r| r.is_save()) {
                    self.picker_name
                        .set(path.file_name().unwrap_or_default().to_string_lossy());
                } else {
                    self.accept(false);
                }
            } else {
                self.browser().open(path.clone());
            }
        }
    }
    pub(super) fn activate(&mut self, path: PathBuf, ordered: &[PathBuf]) {
        if !matches!(self.dialog, Dialog::None) {
            return;
        }
        let multiple = self.request.as_ref().is_none_or(|r| r.multiple);
        let modifiers = self.keyboard.current();
        self.browser().select(
            path.clone(),
            (self.selection_mode || modifiers.contains(Modifiers::CONTROL)) && multiple,
            modifiers.contains(Modifiers::SHIFT) && multiple,
            ordered,
        );
        let double = self
            .last_click
            .as_ref()
            .is_some_and(|(previous, time)| previous == &path && time.elapsed().as_millis() < 420);
        self.last_click = Some((path.clone(), Instant::now()));
        if self.request.as_ref().is_some_and(|r| r.is_save()) && !path.is_dir() {
            self.picker_name
                .set(path.file_name().unwrap_or_default().to_string_lossy());
        }
        if double {
            self.open_selected();
            self.last_click = None;
        }
    }
    pub(super) fn input(&mut self, event: &UiEvent, ordered: &[PathBuf]) -> bool {
        if self.control_input(event) {
            return true;
        }
        let UiEventKind::Input(telorgon::InputEvent::Key(key)) = &event.kind else {
            return false;
        };
        let key = self.keyboard.update(key);
        self.control = key.modifiers.contains(Modifiers::CONTROL);
        self.shift = key.modifiers.contains(Modifiers::SHIFT);
        if key.state != ButtonState::Pressed {
            return false;
        }
        if !matches!(self.dialog, Dialog::None) {
            if key.logical_key == LogicalKey::Named(NamedKey::Escape) {
                self.dismiss_dialog();
            }
            return true;
        }
        let ctrl = self.control;
        let shift = self.shift;
        match &key.logical_key {
            LogicalKey::Named(NamedKey::Enter) => self.open_selected(),
            LogicalKey::Named(NamedKey::F2) if !self.picker_mode() => self.rename_prompt(),
            LogicalKey::Named(NamedKey::F5) => self.browser().refresh(),
            LogicalKey::Named(NamedKey::Delete)
                if !self.picker_mode() && self.browser().snapshot().trash.is_none() =>
            {
                self.dialog = Dialog::Trash
            }
            LogicalKey::Named(NamedKey::Backspace) => {
                if let Some(path) = self.browser().snapshot().directory.parent() {
                    self.navigate(path.to_path_buf());
                }
            }
            LogicalKey::Named(NamedKey::Escape) => {
                self.browser().select_all(vec![]);
                self.browser().close_preview();
            }
            LogicalKey::Named(NamedKey::ArrowDown | NamedKey::ArrowUp) => {
                let state = self.browser().snapshot();
                let current = state
                    .cursor
                    .as_ref()
                    .and_then(|path| ordered.iter().position(|p| p == path));
                let current = current.unwrap_or_else(|| {
                    if key.logical_key == LogicalKey::Named(NamedKey::ArrowDown) {
                        usize::MAX
                    } else {
                        0
                    }
                });
                let index = if key.logical_key == LogicalKey::Named(NamedKey::ArrowUp) {
                    current.saturating_sub(1)
                } else {
                    current.wrapping_add(1).min(ordered.len().saturating_sub(1))
                };
                if let Some(path) = ordered.get(index) {
                    self.browser().select(path.clone(), false, shift, ordered);
                }
            }
            LogicalKey::Character(value) if ctrl => {
                match value.as_str().to_ascii_lowercase().as_str() {
                    "a" if self.request.as_ref().is_none_or(|r| r.multiple) => {
                        self.browser().select_all(ordered.to_vec())
                    }
                    "c" if !self.picker_mode() => self.copy(TransferMode::Copy),
                    "x" if !self.picker_mode() => self.copy(TransferMode::Move),
                    "v" if !self.picker_mode() => self.paste(),
                    "t" if !self.picker_mode() => self.new_tab(),
                    "w" if !self.picker_mode() => self.close_tab(),
                    "h" => {
                        self.preferences.show_hidden = !self.preferences.show_hidden;
                        self.save_preferences();
                        self.bind_inputs();
                    }
                    "n" if shift => {
                        self.name.set("");
                        self.dialog = Dialog::NewFolder;
                    }
                    "z" if !self.picker_mode() => self.browser().undo_trash(),
                    _ => return false,
                }
            }
            _ => return false,
        }
        true
    }
    pub(super) fn accept(&mut self, overwrite_confirmed: bool) {
        let Some(request) = self.selected_request() else {
            return;
        };
        if request.mode == PickerMode::Download {
            let jobs = self.downloads.jobs().snapshot();
            if let Some(job) = jobs
                .iter()
                .find(|job| matches!(job.status, crate::downloads::DownloadStatus::Completed))
            {
                match crate::protocol::selected_response(&request, &[job.destination.clone()]) {
                    Ok(response) => {
                        *self.completion.lock().unwrap() = Some(response);
                        telorgon::request_exit();
                    }
                    Err(error) => self.browser().message(error.to_string(), true),
                }
                return;
            }
            if jobs.iter().any(|job| !job.status.is_finished()) {
                return;
            }
        }
        let state = self.browser().snapshot();
        let paths = if request.is_save() {
            let name = self.picker_name.text();
            if let Err(error) = crate::fs::validate_name(&name) {
                self.browser().message(error, true);
                return;
            }
            vec![state.directory.join(name)]
        } else if request.selects_folders() && state.selected.is_empty() {
            vec![state.directory.clone()]
        } else {
            state.selected
        };
        if overwrite_confirmed
            && !matches!(&self.dialog, Dialog::Overwrite(confirmed) if confirmed == &paths)
        {
            self.dismiss_dialog();
            self.browser().message(
                "The destination changed. Choose Save again to confirm the current file.",
                true,
            );
            return;
        }
        let mut existing = false;
        if request.is_save() {
            for path in &paths {
                match std::fs::symlink_metadata(path) {
                    Ok(_) => existing = true,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        self.browser()
                            .message(format!("Cannot inspect destination: {error}"), true);
                        return;
                    }
                }
            }
        }
        if existing && !overwrite_confirmed {
            self.dialog = Dialog::Overwrite(paths);
            return;
        }
        match crate::protocol::selected_response(&request, &paths) {
            Ok(response) => {
                if request.mode == PickerMode::Download {
                    if let Some(url) = &request.source_url {
                        match self.downloads.start_with_overwrite(
                            url.clone(),
                            paths[0].clone(),
                            overwrite_confirmed,
                        ) {
                            Ok(_) => {
                                self.downloads_open = true;
                                self.dismiss_dialog();
                                self.browser().message(
                                    "Download started. Keep this window open until it finishes.",
                                    false,
                                );
                            }
                            Err(error) => self.browser().message(error, true),
                        }
                    } else {
                        self.browser()
                            .message("No download URL was provided.", true);
                    }
                } else {
                    *self.completion.lock().unwrap() = Some(response);
                    telorgon::request_exit();
                }
            }
            Err(error) => self.browser().message(error.to_string(), true),
        }
    }
}

fn location_path(value: &str, current: &std::path::Path) -> std::result::Result<PathBuf, String> {
    if value.starts_with("file://") {
        crate::protocol::uri_to_path(value).map_err(|error| error.to_string())
    } else if value == "~" {
        Ok(home_directory())
    } else if let Some(relative) = value.strip_prefix("~/") {
        Ok(home_directory().join(relative))
    } else {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            Ok(path)
        } else {
            Ok(current.join(path))
        }
    }
}
