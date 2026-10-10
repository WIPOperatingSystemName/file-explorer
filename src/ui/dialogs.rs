use super::*;
use crate::downloads::{DownloadJob, DownloadStatus};

impl Explorer {
    pub(super) fn dialog_view(&self, state: &Snapshot) -> Container {
        match &self.dialog {
            Dialog::Context => return self.context_menu(state),
            Dialog::More => return self.more_menu(state),
            Dialog::Sort => return self.sort_menu(),
            Dialog::View => return self.view_menu(),
            Dialog::Filters => return self.filter_menu(),
            _ => {}
        }
        let (title, subtitle, header_height, body_height) = match &self.dialog {
            Dialog::NewFolder => ("Create a folder", "The folder will be created in the current location.".into(), 94.0, 88.0),
            Dialog::Rename => ("Rename", "Enter a new name for the selected item.".into(), 94.0, 88.0),
            Dialog::Trash => ("Move to Trash?", format!("{} selected items. You can restore them from Trash.", state.selected.len()), 94.0, 0.0),
            Dialog::Download => ("Download a file", format!("Save to {}", state.directory.display()), 110.0, 156.0),
            Dialog::Overwrite(paths) => ("Replace the existing file?", format!("{} already exists. The requesting application may replace its contents.", paths.first().map(|p| p.display().to_string()).unwrap_or_default()), 126.0, 0.0),
            Dialog::Help => ("Keyboard shortcuts", "Focus a file to use shortcuts. Double-click opens folders and files. Ctrl-click toggles a selection; Shift-click selects a range.".into(), 110.0, 108.0),
            _ => ("", String::new(), 0.0, 0.0),
        };
        let mut pane = column()
            .width(560.0)
            .height(header_height + body_height + 63.0)
            .background(CARD)
            .uniform_border(1.0, LINE)
            .corner_radius(8.0)
            .overflow(telorgon::ui::Overflow::Clip)
            .child(
                column()
                    .width(Dimension::FILL)
                    .height(header_height)
                    .padding(16.0)
                    .gap(8.0)
                    .child(label(title, 18.0, TEXT).height(24.0))
                    .child(label(subtitle, 13.0, MUTED).height(header_height - 64.0)),
            );
        let mut actions = dialog_footer();
        match &self.dialog {
            Dialog::NewFolder | Dialog::Rename => {
                pane = pane.child(
                    column()
                        .width(Dimension::FILL)
                        .height(body_height)
                        .padding(16.0)
                        .child(
                            Field::new("Name", "Enter a name", self.name.clone(), !state.busy)
                                .select_on_focus()
                                .autofocus()
                                .on_submit(|this: &mut Self| this.submit_dialog())
                                .on_escape(|this: &mut Self| this.dismiss_dialog()),
                        ),
                );
                actions = actions
                    .child(spacer())
                    .child(control("Cancel").on_press(|this: &mut Self| this.dismiss_dialog()))
                    .child(primary("Save").on_press(|this: &mut Self| this.submit_dialog()));
            }
            Dialog::Trash => {
                actions = actions
                    .child(spacer())
                    .child(control("Cancel").on_press(|this: &mut Self| this.dismiss_dialog()))
                    .child(primary("Move to Trash").enabled(!state.busy).on_press(
                        |this: &mut Self| {
                            this.browser().trash_selected();
                            this.dismiss_dialog();
                        },
                    ));
            }
            Dialog::Download => {
                pane = pane.child(
                    column()
                        .width(Dimension::FILL)
                        .height(body_height)
                        .padding(16.0)
                        .gap(12.0)
                        .child(
                            Field::new(
                                "Download URL",
                                "https://example.com/file.zip",
                                self.url.clone(),
                                true,
                            )
                            .select_on_focus()
                            .autofocus()
                            .on_submit(|this: &mut Self| this.submit_dialog())
                            .on_escape(|this: &mut Self| this.dismiss_dialog()),
                        )
                        .child(
                            Field::new(
                                "File name",
                                "File name (or infer from URL)",
                                self.name.clone(),
                                true,
                            )
                            .select_on_focus()
                            .on_submit(|this: &mut Self| this.submit_dialog())
                            .on_escape(|this: &mut Self| this.dismiss_dialog()),
                        ),
                );
                actions = actions
                    .child(spacer())
                    .child(control("Cancel").on_press(|this: &mut Self| this.dismiss_dialog()))
                    .child(primary("Download").on_press(|this: &mut Self| this.submit_dialog()));
            }
            Dialog::Overwrite(_) => {
                actions = actions
                    .child(spacer())
                    .child(control("Cancel").on_press(|this: &mut Self| this.dismiss_dialog()))
                    .child(primary("Replace").on_press(|this: &mut Self| this.accept(true)));
            }
            Dialog::Help => {
                pane = pane.child(
                    column()
                        .width(Dimension::FILL)
                        .height(body_height)
                        .padding((8.0, 16.0))
                        .child(
                            label(
                                "Enter: open · F2: rename · F5: refresh · Delete: trash
Backspace: parent folder · Ctrl+L: location · Ctrl+A: select all
Ctrl+C/X/V: copy/cut/paste · Ctrl+H: hidden files · Ctrl+Z: undo trash
Ctrl+T/W: new/close tab · Ctrl+Shift+N: new folder · Esc: cancel",
                                12.0,
                                TEXT,
                            )
                            .height(80.0),
                        ),
                );
                actions = actions
                    .child(spacer())
                    .child(control("Close").on_press(|this: &mut Self| this.dismiss_dialog()));
            }
            _ => {}
        }
        pane.child(column().width(Dimension::FILL).height(1.0).background(LINE))
            .child(actions)
    }

    fn context_menu(&self, state: &Snapshot) -> Container {
        let file_actions = !self.picker_mode() && state.trash.is_none();
        let selected = !state.selected.is_empty();
        let mut menu = menu_base(if file_actions { 311.0 } else { 89.0 });
        if file_actions {
            menu =
                menu.child(menu_item("Open", "folder").enabled(selected).on_press(
                    |this: &mut Self| {
                        this.dismiss_dialog();
                        this.open_selected();
                    },
                ))
                .child(
                    menu_item("Open in new tab", "plus")
                        .enabled(state.selected.len() == 1)
                        .on_press(|this: &mut Self| {
                            if let Some(path) = this.browser().snapshot().selected.first() {
                                let path = if path.is_dir() {
                                    path.clone()
                                } else {
                                    path.parent()
                                        .unwrap_or(std::path::Path::new("/"))
                                        .to_path_buf()
                                };
                                this.tabs
                                    .push(Browser::new(path.clone(), this.workers.clone()));
                                this.active = this.tabs.len() - 1;
                                this.location.set(path.to_string_lossy());
                                this.search.set("");
                                this.bind_inputs();
                            }
                            this.dismiss_dialog();
                        }),
                )
                .child(menu_line())
                .child(
                    menu_item("Cut", "cut")
                        .enabled(selected)
                        .on_press(|this: &mut Self| {
                            this.copy(TransferMode::Move);
                            this.dismiss_dialog();
                        }),
                )
                .child(
                    menu_item("Copy", "copy")
                        .enabled(selected)
                        .on_press(|this: &mut Self| {
                            this.copy(TransferMode::Copy);
                            this.dismiss_dialog();
                        }),
                )
                .child(
                    menu_item("Rename", "rename")
                        .enabled(state.selected.len() == 1 && !state.busy)
                        .on_press(|this: &mut Self| this.rename_prompt()),
                )
                .child(
                    menu_item("Trash", "trash")
                        .enabled(selected && !state.busy)
                        .on_press(|this: &mut Self| this.dialog = Dialog::Trash),
                )
                .child(menu_line());
        }
        menu.child(
            menu_item("Properties", "details")
                .enabled(state.selected.len() == 1)
                .on_press(|this: &mut Self| {
                    if let Some(path) = this.browser().snapshot().selected.first() {
                        this.browser().preview(path.clone());
                    }
                    this.dismiss_dialog();
                }),
        )
        .child(menu_line())
        .child(menu_item("Close", "close").on_press(|this: &mut Self| this.dismiss_dialog()))
    }

    fn more_menu(&self, state: &Snapshot) -> Container {
        let mut menu = menu_base(if self.picker_mode() { 80.0 } else { 302.0 });
        if !self.picker_mode() {
            let pinned = self.preferences.bookmarks.contains(&state.directory);
            let jobs = self.downloads.jobs().snapshot();
            menu = menu
                .child(
                    menu_item("Open", "folder")
                        .enabled(!state.selected.is_empty() && state.trash.is_none())
                        .on_press(|this: &mut Self| {
                            this.dismiss_dialog();
                            this.open_selected();
                        }),
                )
                .child(
                    menu_item(
                        if pinned {
                            "Unpin this folder"
                        } else {
                            "Pin this folder"
                        },
                        "pin",
                    )
                    .enabled(state.trash.is_none())
                    .on_press(|this: &mut Self| {
                        let path = this.browser().snapshot().directory;
                        if this.preferences.bookmarks.contains(&path) {
                            this.preferences
                                .bookmarks
                                .retain(|bookmark| bookmark != &path);
                        } else {
                            this.preferences.bookmarks.push(path);
                        }
                        this.save_preferences();
                        this.dismiss_dialog();
                    }),
                )
                .child(
                    menu_item("Select all", "check")
                        .enabled(!state.loading)
                        .on_press(|this: &mut Self| {
                            let state = this.browser().snapshot();
                            let paths = this
                                .entries(&state)
                                .into_iter()
                                .map(|entry| entry.path)
                                .collect();
                            this.browser().select_all(paths);
                            this.dismiss_dialog();
                        }),
                )
                .child(
                    menu_item("Undo trash", "refresh")
                        .enabled(!state.undo_trash.is_empty() && !state.busy)
                        .on_press(|this: &mut Self| {
                            this.browser().undo_trash();
                            this.dismiss_dialog();
                        }),
                )
                .child(menu_line())
                .child(
                    menu_item("Download URL", "download")
                        .enabled(state.trash.is_none())
                        .on_press(|this: &mut Self| {
                            this.url.set("");
                            this.name.set("");
                            this.dialog = Dialog::Download;
                        }),
                )
                .child(
                    menu_item(&format!("Downloads ({})", jobs.len()), "download").on_press(
                        |this: &mut Self| {
                            this.downloads_open = !this.downloads_open;
                            this.dismiss_dialog();
                        },
                    ),
                )
                .child(menu_line());
        }
        menu.child(
            menu_item("Keyboard shortcuts", "help")
                .on_press(|this: &mut Self| this.dialog = Dialog::Help),
        )
        .child(menu_item("Close", "close").on_press(|this: &mut Self| this.dismiss_dialog()))
    }

    fn sort_menu(&self) -> Container {
        let mut menu = menu_base(268.0);
        for (value, title) in [
            ("name", "Name"),
            ("modified", "Date modified"),
            ("type", "Type"),
            ("size", "Size"),
        ] {
            menu = menu.child(
                menu_item(
                    title,
                    if self.preferences.sort_by == value {
                        "check"
                    } else {
                        ""
                    },
                )
                .on_press(move |this: &mut Self| {
                    this.preferences.sort_by = value.into();
                    this.save_preferences();
                    this.dismiss_dialog();
                }),
            );
        }
        menu = menu.child(menu_line());
        for (descending, title) in [(false, "Ascending"), (true, "Descending")] {
            menu = menu.child(
                menu_item(
                    title,
                    if self.preferences.descending == descending {
                        "check"
                    } else {
                        ""
                    },
                )
                .on_press(move |this: &mut Self| {
                    this.preferences.descending = descending;
                    this.save_preferences();
                    this.dismiss_dialog();
                }),
            );
        }
        menu.child(menu_line())
            .child(menu_item("Close", "close").on_press(|this: &mut Self| this.dismiss_dialog()))
    }

    fn view_menu(&self) -> Container {
        let multiple = self.request.as_ref().is_none_or(|request| request.multiple);
        let mut menu = menu_base(if multiple { 200.0 } else { 166.0 });
        for (value, title) in [("list", "Details"), ("grid", "Large icons")] {
            menu = menu.child(
                menu_item(
                    title,
                    if self.preferences.view_mode == value {
                        "check"
                    } else {
                        ""
                    },
                )
                .on_press(move |this: &mut Self| {
                    this.preferences.view_mode = value.into();
                    this.save_preferences();
                    this.dismiss_dialog();
                }),
            );
        }
        menu = menu.child(menu_line()).child(
            menu_item(
                "Show hidden files",
                if self.preferences.show_hidden {
                    "check"
                } else {
                    ""
                },
            )
            .accessible_label(if self.preferences.show_hidden {
                "Hidden: on"
            } else {
                "Hidden: off"
            })
            .on_press(|this: &mut Self| {
                this.preferences.show_hidden = !this.preferences.show_hidden;
                this.save_preferences();
                this.bind_inputs();
                this.dismiss_dialog();
            }),
        );
        if multiple {
            menu = menu.child(
                menu_item(
                    "Select items",
                    if self.selection_mode { "check" } else { "" },
                )
                .accessible_label(if self.selection_mode {
                    "Select: on"
                } else {
                    "Select"
                })
                .on_press(|this: &mut Self| {
                    this.selection_mode = !this.selection_mode;
                    this.dismiss_dialog();
                }),
            );
        }
        menu.child(menu_line())
            .child(menu_item("Close", "close").on_press(|this: &mut Self| this.dismiss_dialog()))
    }

    fn filter_menu(&self) -> Container {
        let Some(request) = &self.request else {
            return menu_base(46.0);
        };
        let content_height = request.filters.len() as f32 * 34.0 + 43.0;
        let height = (content_height + 12.0).min(440.0);
        let mut rows = content(content_height).gap(0.0);
        for (index, filter) in request.filters.iter().enumerate() {
            rows = rows.child(
                menu_item(
                    &filter.name,
                    if self.filter_index == index {
                        "check"
                    } else {
                        ""
                    },
                )
                .on_press(move |this: &mut Self| {
                    this.filter_index = index;
                    this.dismiss_dialog();
                }),
            );
        }
        rows = rows
            .child(menu_line())
            .child(menu_item("Close", "close").on_press(|this: &mut Self| this.dismiss_dialog()));
        menu_base(height).child(
            column()
                .width(Dimension::FILL)
                .height(Dimension::FILL)
                .scrollable()
                .child(rows),
        )
    }

    pub(super) fn picker_footer(&self, state: &Snapshot) -> Container {
        let request = self.request.as_ref().unwrap();
        let jobs = self.downloads.jobs().snapshot();
        let downloaded = request.mode == PickerMode::Download
            && jobs
                .iter()
                .any(|job| matches!(job.status, DownloadStatus::Completed));
        let downloading = request.mode == PickerMode::Download
            && jobs.iter().any(|job| !job.status.is_finished());
        let choices_content_height = (request.choices.len() as f32 * 42.0 - 8.0).max(0.0);
        let choices_height = choices_content_height.min(if self.viewport_size().height < 600.0 {
            42.0
        } else {
            118.0
        });
        let choices_extra = if request.choices.is_empty() {
            0.0
        } else {
            choices_height + 8.0
        };
        let mut footer = column()
            .width(Dimension::FILL)
            .gap(8.0)
            .height(if request.is_save() { 108.0 } else { 66.0 } + choices_extra)
            .padding(16.0)
            .background(PANEL);
        if request.is_save() {
            footer = footer.child(
                row()
                    .width(Dimension::FILL)
                    .height(34.0)
                    .gap(12.0)
                    .align_items(Alignment::Center)
                    .child(label("File name:", 13.0, TEXT).width(72.0))
                    .child(
                        Field::new(
                            "File name",
                            "File name",
                            self.picker_name.clone(),
                            !state.busy,
                        )
                        .compact()
                        .select_on_focus()
                        .autofocus()
                        .on_submit(|this: &mut Self| this.submit_picker()),
                    ),
            );
        }
        let mut choices = content(choices_content_height).gap(8.0);
        for (index, choice) in request.choices.iter().enumerate() {
            let value = choice
                .options
                .iter()
                .find(|(id, _)| id == &choice.selected)
                .map(|(_, label)| label.as_str())
                .unwrap_or(&choice.selected);
            choices = choices.child(
                row()
                    .width(Dimension::FILL)
                    .height(34.0)
                    .gap(12.0)
                    .align_items(Alignment::Center)
                    .child(label(&choice.label, 13.0, TEXT).width(144.0))
                    .child(
                        control(if choice.options.is_empty() {
                            if choice.selected == "true" {
                                "Yes"
                            } else {
                                "No"
                            }
                        } else {
                            value
                        })
                        .background(CARD)
                        .uniform_border(1.0, LINE)
                        .on_press(move |this: &mut Self| {
                            if let Some(choice) = this
                                .request
                                .as_mut()
                                .and_then(|request| request.choices.get_mut(index))
                            {
                                if choice.options.is_empty() {
                                    choice.selected = if choice.selected == "true" {
                                        "false"
                                    } else {
                                        "true"
                                    }
                                    .into();
                                } else {
                                    let index = choice
                                        .options
                                        .iter()
                                        .position(|(id, _)| id == &choice.selected)
                                        .unwrap_or(0);
                                    choice.selected = choice.options
                                        [(index + 1) % choice.options.len()]
                                    .0
                                    .clone();
                                }
                            }
                        }),
                    ),
            );
        }
        if !request.choices.is_empty() {
            footer = footer.child(
                column()
                    .width(Dimension::FILL)
                    .height(choices_height)
                    .scrollable()
                    .child(choices),
            );
        }
        let filter = request
            .filters
            .get(self.filter_index)
            .map(|filter| filter.name.as_str())
            .unwrap_or("All files");
        let accept_label = if downloaded {
            "Done"
        } else if downloading {
            "Downloading…"
        } else {
            request.accept_label.as_deref().unwrap_or(match request.mode {
                PickerMode::Save => "Save",
                PickerMode::SaveFiles | PickerMode::Folder => "Choose folder",
                PickerMode::Download => "Download",
                _ => "Open",
            })
        };
        let accept_width = (accept_label.chars().count() as f32 * 7.0 + 28.0).clamp(76.0, 240.0);
        let filter_width = (self.viewport_size().width * 0.32)
            .clamp(100.0, 360.0)
            .min((self.viewport_size().width - 236.0 - accept_width).max(80.0));
        footer.child(
            row()
                .width(Dimension::FILL)
                .height(34.0)
                .gap(12.0)
                .align_items(Alignment::Center)
                .child(label("File type:", 13.0, TEXT).width(72.0))
                .child(
                    button()
                        .accessible_label(format!("File type: {filter}"))
                        .width(filter_width)
                        .height(34.0)
                        .padding(8.0)
                        .corner_radius(4.0)
                        .background(CARD)
                        .uniform_border(1.0, LINE)
                        .enabled(!request.filters.is_empty())
                        .child(
                            row()
                                .width(Dimension::FILL)
                                .height(18.0)
                                .align_items(Alignment::Center)
                                .child(label(filter, 13.0, TEXT))
                                .child(spacer())
                                .child(icon_view("chevron-down", 12.0)),
                        )
                        .on_input(|this: &mut Self, event| this.control_input(event))
                        .on_press(|this: &mut Self| this.dialog = Dialog::Filters),
                )
                .children(
                    (request.multiple && self.viewport_size().width >= 960.0)
                        .then(|| label("Multiple selections allowed", 11.0, MUTED)),
                )
                .child(spacer())
                .child(
                    control("Cancel")
                        .width(84.0)
                        .background(CARD)
                        .uniform_border(1.0, LINE)
                        .on_press(|this: &mut Self| this.cancel()),
                )
                .child(
                    primary(accept_label)
                    .enabled(
                        !downloading
                            && !state.loading
                            && !state.busy
                            && (request.is_save()
                                || request.selects_folders()
                                || !state.selected.is_empty()),
                    )
                    .on_press(|this: &mut Self| this.accept(false)),
                ),
        )
    }
    pub(super) fn downloads_panel(&self, jobs: &[DownloadJob]) -> Container {
        let visible_jobs = if self.viewport_size().height < 600.0 {
            1
        } else {
            3
        };
        let height = (jobs.len().min(visible_jobs) as f32 * 52.0 + 52.0).min(210.0);
        let mut rows = content(jobs.len() as f32 * 52.0).gap(4.0);
        for job in jobs.iter().rev() {
            let id = job.id;
            let destination = job.destination.clone();
            let status = match &job.status {
                DownloadStatus::Queued => "Queued".into(),
                DownloadStatus::Downloading => format!(
                    "{}{} · {}/s",
                    size(job.received_bytes),
                    job.total_bytes
                        .map(|total| format!(" / {}", size(total)))
                        .unwrap_or_default(),
                    size(job.bytes_per_second)
                ),
                DownloadStatus::Completed => "Complete".into(),
                DownloadStatus::Cancelled => "Cancelled".into(),
                DownloadStatus::Failed(error) => format!("Failed: {error}"),
            };
            rows = rows.child(
                row()
                    .height(48.0)
                    .gap(10.0)
                    .align_items(Alignment::Center)
                    .child(icon_view("download", 20.0))
                    .child(
                        column()
                            .width(Dimension::FILL)
                            .gap(3.0)
                            .child(label(&job.filename, 12.0, TEXT).height(18.0))
                            .child(label(status, 11.0, MUTED).height(18.0)),
                    )
                    .child(if job.status.is_finished() {
                        control("Show")
                            .enabled(matches!(job.status, DownloadStatus::Completed))
                            .on_press(move |this: &mut Self| {
                                if let Some(parent) = destination.parent() {
                                    this.navigate(parent.to_path_buf());
                                    this.browser().select_initial(destination.clone());
                                }
                            })
                    } else {
                        control("Cancel").on_press(move |this: &mut Self| {
                            this.downloads.cancel(id);
                        })
                    }),
            );
        }
        column()
            .width(Dimension::FILL)
            .height(height)
            .padding(8.0)
            .gap(4.0)
            .background(PANEL)
            .corner_radius(8.0)
            .child(
                row()
                    .height(32.0)
                    .align_items(Alignment::Center)
                    .child(label("Downloads", 13.0, TEXT))
                    .child(spacer())
                    .child(
                        control("Clear finished")
                            .height(32.0)
                            .on_press(|this: &mut Self| this.downloads.clear_finished()),
                    )
                    .child(
                        icon_button("Close downloads", "close")
                            .width(32.0)
                            .height(32.0)
                            .padding(7.0)
                            .on_press(|this: &mut Self| this.downloads_open = false),
                    ),
            )
            .child(
                column()
                    .width(Dimension::FILL)
                    .height(Dimension::FILL)
                    .scrollable()
                    .child(rows),
            )
    }
}

fn menu_base(height: f32) -> Container {
    column()
        .width(260.0)
        .height(height)
        .padding(6.0)
        .gap(0.0)
        .background(CARD)
        .uniform_border(1.0, LINE)
        .corner_radius(6.0)
}

fn menu_item(value: &str, icon: &str) -> Button {
    button()
        .accessible_label(value)
        .width(Dimension::FILL)
        .height(34.0)
        .padding(8.0)
        .background(CARD)
        .corner_radius(4.0)
        .hover_effect(InteractionEffect::Background(SELECTED))
        .press_effect(InteractionEffect::Background(PANEL))
        .child(
            row()
                .width(Dimension::FILL)
                .height(18.0)
                .gap(10.0)
                .align_items(Alignment::Center)
                .child(
                    column()
                        .width(18.0)
                        .height(18.0)
                        .children((!icon.is_empty()).then(|| icon_view(icon, 18.0))),
                )
                .child(label(value, 13.0, TEXT)),
        )
        .on_input(|this: &mut Explorer, event| this.control_input(event))
}

fn menu_line() -> Container {
    column()
        .width(Dimension::FILL)
        .height(9.0)
        .padding((4.0, 8.0))
        .child(column().width(Dimension::FILL).height(1.0).background(LINE))
}

fn dialog_footer() -> Container {
    row()
        .width(Dimension::FILL)
        .height(62.0)
        .padding((14.0, 16.0))
        .gap(8.0)
        .align_items(Alignment::Center)
        .background(PANEL)
}
