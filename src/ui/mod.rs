mod actions;
mod chrome;
mod clipboard;
mod dialogs;
mod field;
mod field_composition;
mod field_layout;
mod icons;
mod keyboard;
mod listing;
mod navigation;
mod overlay;
#[cfg(test)]
mod overlay_tests;
#[cfg(test)]
mod tests;
pub mod theme;

use crate::{
    downloads::DownloadManager,
    fs::TransferMode,
    model::{Browser, Snapshot, visible_entries},
    preferences::{Preferences, home_directory},
    protocol::{PickerMode, PickerRequest, PickerResponse},
};
use field::{Field, FieldValue};
use icons::icon_view;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime},
};
use telorgon::app::*;
use theme::*;

#[derive(Clone)]
enum Dialog {
    None,
    Context,
    More,
    Sort,
    View,
    Filters,
    NewFolder,
    Rename,
    Trash,
    Download,
    Overwrite(Vec<PathBuf>),
    Help,
}

#[component(no_default)]
pub struct Explorer {
    #[state]
    tabs: Vec<Browser>,
    #[state]
    workers: crate::model::FileWorkers,
    #[state]
    active: usize,
    #[state]
    preferences: Preferences,
    #[state]
    location: FieldValue,
    #[state]
    search: FieldValue,
    #[state]
    name: FieldValue,
    #[state]
    picker_name: FieldValue,
    #[state]
    url: FieldValue,
    #[state]
    dialog: Dialog,
    #[state]
    clipboard: Option<(Vec<PathBuf>, TransferMode)>,
    #[state]
    downloads: Arc<DownloadManager>,
    #[state]
    downloads_open: bool,
    #[state]
    request: Option<PickerRequest>,
    #[state]
    completion: Arc<Mutex<Option<PickerResponse>>>,
    #[state]
    last_click: Option<(PathBuf, Instant)>,
    #[state]
    control: bool,
    #[state]
    shift: bool,
    #[state]
    filter_index: usize,
    #[state]
    keyboard: keyboard::Keyboard,
    #[state]
    selection_mode: bool,
    #[state]
    computer_expanded: bool,
}
impl Explorer {
    pub fn new(
        path: PathBuf,
        request: Option<PickerRequest>,
        completion: Arc<Mutex<Option<PickerResponse>>>,
    ) -> Self {
        let name = request
            .as_ref()
            .and_then(|r| r.current_name.clone())
            .unwrap_or_default();
        let url = request
            .as_ref()
            .and_then(|r| r.source_url.clone())
            .unwrap_or_default();
        let filter_index = request
            .as_ref()
            .and_then(|r| {
                r.current_filter
                    .as_ref()
                    .and_then(|f| r.filters.iter().position(|item| item.name == f.name))
            })
            .unwrap_or(0);
        let keyboard = keyboard::Keyboard::default();
        let workers = crate::model::FileWorkers::default();
        let explorer = Self {
            tabs: vec![Browser::new(path.clone(), workers.clone())],
            workers,
            active: 0,
            preferences: Preferences::load(),
            location: FieldValue::with_keyboard(path.to_string_lossy(), false, keyboard.clone()),
            search: FieldValue::with_keyboard("", false, keyboard.clone()),
            name: FieldValue::with_keyboard("", false, keyboard.clone()),
            picker_name: FieldValue::with_keyboard(name, false, keyboard.clone()),
            url: FieldValue::with_keyboard(url, false, keyboard.clone()),
            dialog: Dialog::None,
            clipboard: None,
            downloads: Arc::new(DownloadManager::new()),
            downloads_open: false,
            request,
            completion,
            last_click: None,
            control: false,
            shift: false,
            filter_index,
            keyboard,
            selection_mode: false,
            computer_expanded: true,
        };
        explorer.bind_inputs();
        explorer
    }
    pub fn initial_selection(&mut self, path: PathBuf, properties: bool) {
        self.browser().select_initial(path.clone());
        if properties {
            self.browser().preview(path);
        }
    }
    pub fn file_workers(&self) -> crate::model::FileWorkers {
        self.workers.clone()
    }
    fn browser(&self) -> &Browser {
        &self.tabs[self.active]
    }
    fn navigate(&mut self, path: PathBuf) {
        self.browser().navigate(path.clone());
        self.location.set(path.to_string_lossy());
        self.search.set("");
        self.last_click = None;
        self.bind_inputs();
    }
    fn save_preferences(&self) {
        if let Err(error) = self.preferences.save() {
            self.browser().message(error, true);
        }
    }
    fn picker_mode(&self) -> bool {
        self.request.is_some()
    }
    fn selected_request(&self) -> Option<PickerRequest> {
        self.request.clone().map(|mut request| {
            request.current_filter = request.filters.get(self.filter_index).cloned();
            request
        })
    }
    fn cancel(&mut self) {
        if self.picker_mode() {
            *self.completion.lock().unwrap() = Some(PickerResponse::cancelled());
        }
        telorgon::request_exit();
    }
    fn entries(&self, state: &Snapshot) -> Vec<crate::fs::Entry> {
        let request = self.selected_request();
        visible_entries(state, &self.preferences)
            .into_iter()
            .filter(|entry| {
                request.as_ref().is_none_or(|r| {
                    if r.mode == PickerMode::Folder
                        || r.directory
                        || r.mode == PickerMode::SaveFiles
                    {
                        entry.is_dir
                    } else {
                        entry.is_dir || r.matches_path(&entry.path)
                    }
                })
            })
            .collect()
    }
}
impl Component for Explorer {
    fn view(&self) -> impl View {
        self.location.watch(self);
        self.search.watch(self);
        self.name.watch(self);
        self.picker_name.watch(self);
        self.url.watch(self);
        let state = self.watch(self.browser().signal());
        let jobs = self.watch(&self.downloads.jobs());
        let entries = self.entries(&state);
        let mut main = column()
            .width(Dimension::FILL)
            .height(Dimension::FILL)
            .background(BG)
            .gap(0.0);
        if !self.picker_mode() {
            main = main.child(self.tabbar());
        } else if let Some(request) = &self.request {
            main = main.child(
                row()
                    .width(Dimension::FILL)
                    .height(36.0)
                    .padding((6.0, 16.0))
                    .background(PANEL)
                    .align_items(Alignment::Center)
                    .gap(8.0)
                    .child(icon_view("folder", 18.0))
                    .child(label(
                        if request.title.is_empty() {
                            "Choose a file"
                        } else {
                            &request.title
                        },
                        13.0,
                        TEXT,
                    ))
                    .child(spacer())
                    .children(
                        (!request.app_id.is_empty()).then(|| label(&request.app_id, 11.0, MUTED)),
                    ),
            );
        }
        main = main
            .child(self.toolbar(&state))
            .child(self.commands(&state))
            .child(column().width(Dimension::FILL).height(1.0).background(LINE))
            .child(
                row()
                    .width(Dimension::FILL)
                    .height(Dimension::FILL)
                    .gap(0.0)
                    .child(self.sidebar(&state))
                    .child(column().width(1.0).height(Dimension::FILL).background(LINE))
                    .child(self.listing(&state, &entries))
                    .children(
                        state
                            .preview
                            .as_ref()
                            .filter(|_| self.viewport_size().width >= 1080.0)
                            .map(|preview| self.preview_panel(preview)),
                    ),
            );
        if self.downloads_open || jobs.iter().any(|job| !job.status.is_finished()) {
            main = main.child(self.downloads_panel(&jobs));
        }
        main = main.child(self.footer(&state, entries.len(), &jobs));
        if self.picker_mode() {
            main = main.child(self.picker_footer(&state));
        }
        let menu = matches!(
            self.dialog,
            Dialog::Context | Dialog::More | Dialog::Sort | Dialog::View | Dialog::Filters
        );
        let menu_left: f32 = match self.dialog {
            Dialog::Sort => {
                if self.picker_mode() {
                    115.0
                } else if state.trash.is_some() {
                    192.0
                } else {
                    322.0
                }
            }
            Dialog::View => {
                if self.picker_mode() {
                    209.0
                } else if state.trash.is_some() {
                    286.0
                } else {
                    416.0
                }
            }
            Dialog::More => {
                if self.picker_mode() {
                    303.0
                } else if state.trash.is_some() {
                    380.0
                } else {
                    510.0
                }
            }
            Dialog::Context => 246.0,
            _ => 24.0,
        };
        let filter_menu = matches!(self.dialog, Dialog::Filters);
        let menu_left = menu_left.min((self.viewport_size().width - 284.0).max(24.0));
        stack()
            .width(Dimension::FILL)
            .height(Dimension::FILL)
            .child(main)
            .children((!matches!(self.dialog, Dialog::None)).then(|| self.dismiss_layer(menu)))
            .children((!matches!(self.dialog, Dialog::None)).then(|| {
                column()
                    .width(Dimension::FILL)
                    .height(Dimension::FILL)
                    .focus_scope(true)
                    .padding(if menu {
                        Insets::new(
                            if filter_menu {
                                24.0
                            } else if self.viewport_size().height < 600.0 {
                                100.0
                            } else if self.picker_mode() {
                                150.0
                            } else {
                                156.0
                            },
                            24.0,
                            if filter_menu { 114.0 } else { 28.0 },
                            menu_left,
                        )
                    } else {
                        Insets::all(24.0)
                    })
                    .justify_content(if filter_menu {
                        Alignment::End
                    } else if menu {
                        Alignment::Start
                    } else {
                        Alignment::Center
                    })
                    .align_items(if menu {
                        Alignment::Start
                    } else {
                        Alignment::Center
                    })
                    .child(if menu {
                        column()
                            .width(260.0)
                            .height(Dimension::FILL)
                            .scrollable()
                            .child(self.dialog_view(&state))
                            .into_element()
                    } else {
                        self.dialog_view(&state).into_element()
                    })
            }))
    }
}
pub fn date(time: SystemTime) -> String {
    #[cfg(unix)]
    {
        let seconds = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as libc::time_t;
        let mut broken: libc::tm = unsafe { std::mem::zeroed() };
        if unsafe { libc::localtime_r(&seconds, &mut broken) }.is_null() {
            return "Unknown".into();
        }
        return format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            broken.tm_year + 1900,
            broken.tm_mon + 1,
            broken.tm_mday,
            broken.tm_hour,
            broken.tm_min
        );
    }
    #[cfg(not(unix))]
    {
        format!(
            "{} seconds since Unix epoch",
            time.duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        )
    }
}
