use super::*;

impl Explorer {
    pub(super) fn tabbar(&self) -> Container {
        let tab_width =
            ((self.viewport_size().width - 96.0) / self.tabs.len() as f32).clamp(92.0, 224.0);
        let mut tabs = row()
            .width(Dimension::FILL)
            .height(42.0)
            .padding(Insets::new(6.0, 12.0, 0.0, 12.0))
            .gap(3.0)
            .align_items(Alignment::Center)
            .background(TAB_BAR);
        for (index, tab) in self.tabs.iter().enumerate() {
            let state = self.watch(tab.signal());
            let name = if state.trash.is_some() {
                "Trash".to_owned()
            } else if state.directory == home_directory() {
                "Home".to_owned()
            } else if state.directory.parent().is_none() {
                "File System".to_owned()
            } else {
                state
                    .directory
                    .file_name()
                    .unwrap_or(state.directory.as_os_str())
                    .to_string_lossy()
                    .into_owned()
            };
            let active = index == self.active;
            let background = if active { CARD } else { TAB_BAR };
            let name_limit = ((tab_width - 80.0) / 7.5).floor().max(3.0) as usize;
            let display_name = if name.chars().count() > name_limit {
                format!("{}…", name.chars().take(name_limit - 1).collect::<String>())
            } else {
                name.clone()
            };
            let close_label = if active {
                "Close tab".to_owned()
            } else {
                format!("Close {} tab {}", name, index + 1)
            };
            tabs = tabs.child(
                row()
                    .key(format!("tab-row-{index}"))
                    .width(tab_width)
                    .height(36.0)
                    .background(background)
                    .corner_radius(7.0)
                    .gap(0.0)
                    .align_items(Alignment::Center)
                    .child(
                        button()
                            .key(format!("tab-{index}"))
                            .accessible_label(&name)
                            .width(tab_width - 30.0)
                            .height(36.0)
                            .padding(Insets::symmetric(8.0, 10.0))
                            .corner_radius(7.0)
                            .background(background)
                            .overflow(telorgon::ui::Overflow::Clip)
                            .child(
                                row()
                                    .width(Dimension::FILL)
                                    .gap(9.0)
                                    .align_items(Alignment::Center)
                                    .child(icon_view(
                                        if state.trash.is_some() {
                                            "trash"
                                        } else {
                                            "folder"
                                        },
                                        17.0,
                                    ))
                                    .child(label(&display_name, 12.0, TEXT).width(Dimension::FILL)),
                            )
                            .on_press(move |this: &mut Self| {
                                this.active = index;
                                this.location
                                    .set(this.browser().snapshot().directory.to_string_lossy());
                                this.search.set(this.browser().snapshot().query);
                                this.bind_inputs();
                            })
                            .on_input(|this: &mut Self, event| this.control_input(event)),
                    )
                    .child(
                        icon_button(&close_label, "close")
                            .key(format!("close-tab-{index}"))
                            .width(28.0)
                            .height(28.0)
                            .padding(8.0)
                            .background(background)
                            .enabled(self.tabs.len() > 1)
                            .on_press(move |this: &mut Self| this.close_tab_at(index)),
                    ),
            );
        }
        tabs.child(
            icon_button("New tab", "plus")
                .key("new-tab")
                .background(TAB_BAR)
                .on_press(|this: &mut Self| this.new_tab()),
        )
        .child(spacer())
    }

    fn close_tab_at(&mut self, index: usize) {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return;
        }
        let previously_active = self.active;
        self.active = index;
        self.close_tab();
        self.active = if previously_active == index {
            index.min(self.tabs.len() - 1)
        } else if previously_active > index {
            previously_active - 1
        } else {
            previously_active
        };
        self.location
            .set(self.browser().snapshot().directory.to_string_lossy());
        self.search.set("");
        self.bind_inputs();
    }

    pub(super) fn toolbar(&self, state: &Snapshot) -> Container {
        let display = if state.trash.is_some() {
            "Recycle Bin".into()
        } else if let Ok(relative) = state.directory.strip_prefix(home_directory()) {
            std::iter::once("Home".to_owned())
                .chain(
                    relative
                        .components()
                        .map(|part| part.as_os_str().to_string_lossy().into_owned()),
                )
                .collect::<Vec<_>>()
                .join("  ›  ")
        } else {
            std::iter::once("File System".to_owned())
                .chain(
                    state
                        .directory
                        .components()
                        .filter(|part| !matches!(part, std::path::Component::RootDir))
                        .map(|part| part.as_os_str().to_string_lossy().into_owned()),
                )
                .collect::<Vec<_>>()
                .join("  ›  ")
        };
        let address = Field::new(
            "Location",
            "/path/to/folder",
            self.location.clone(),
            !state.busy && state.trash.is_none(),
        )
        .compact()
        .leading_icon("folder")
        .display_text(display)
        .select_on_focus()
        .on_escape(|this: &mut Self| {
            this.location
                .set(this.browser().snapshot().directory.to_string_lossy());
            this.location.finish_editing();
        });
        let search_width = (self.viewport_size().width * 0.245).clamp(230.0, 340.0);
        let search_placeholder = format!(
            "Search {}",
            if state.directory == home_directory() {
                "Home".into()
            } else if state.directory.parent().is_none() {
                "File System".into()
            } else {
                state
                    .directory
                    .file_name()
                    .unwrap_or(state.directory.as_os_str())
                    .to_string_lossy()
            }
        );
        row()
            .width(Dimension::FILL)
            .height(64.0)
            .padding(Insets::symmetric(14.0, 16.0))
            .gap(12.0)
            .align_items(Alignment::Center)
            .background(PANEL)
            .child(
                row()
                    .width(144.0)
                    .height(36.0)
                    .gap(0.0)
                    .align_items(Alignment::Center)
                    .child(
                        navigation_button("Back", "back")
                            .key("history-back")
                            .enabled(state.position > 0 && !state.busy)
                            .on_press(|this: &mut Self| this.go_history(false)),
                    )
                    .child(
                        navigation_button("Forward", "forward")
                            .key("history-forward")
                            .enabled(state.position + 1 < state.history.len() && !state.busy)
                            .on_press(|this: &mut Self| this.go_history(true)),
                    )
                    .child(
                        navigation_button("Parent folder", "up")
                            .key("parent-folder")
                            .enabled(state.directory.parent().is_some() && !state.busy)
                            .on_press(|this: &mut Self| {
                                if let Some(path) = this.browser().snapshot().directory.parent() {
                                    this.navigate(path.to_path_buf());
                                }
                            }),
                    )
                    .child(
                        navigation_button("Refresh", "refresh")
                            .key("refresh-folder")
                            .on_press(|this: &mut Self| this.browser().refresh()),
                    ),
            )
            .child(address)
            .child(
                row()
                    .width(search_width)
                    .height(36.0)
                    .gap(0.0)
                    .align_items(Alignment::Center)
                    .background(CARD)
                    .uniform_border(1.0, LINE)
                    .corner_radius(5.0)
                    .child(
                        Field::new(
                            "Search",
                            &search_placeholder,
                            self.search.clone(),
                            !state.busy && state.trash.is_none(),
                        )
                        .compact()
                        .borderless(),
                    )
                    .child(
                        icon_button("Search files", "search")
                            .enabled(!state.busy && state.trash.is_none())
                            .on_press(|this: &mut Self| {
                                this.browser()
                                    .search(this.search.text(), this.preferences.show_hidden);
                            }),
                    ),
            )
    }

    pub(super) fn commands(&self, state: &Snapshot) -> Container {
        let mut commands = row()
            .width(Dimension::FILL)
            .height(54.0)
            .padding(Insets::symmetric(10.0, 16.0))
            .gap(3.0)
            .align_items(Alignment::Center)
            .background(COMMAND_BAR);
        if state.trash.is_some() {
            commands = commands.child(
                command("Restore selected", "refresh")
                    .enabled(!state.selected.is_empty() && !state.busy)
                    .on_press(|this: &mut Self| this.browser().restore_selected()),
            );
        } else {
            commands = commands.child(
                command("New ▾", "plus")
                    .accessible_label("New folder")
                    .enabled(!state.busy)
                    .on_press(|this: &mut Self| {
                        this.name.set("");
                        this.dialog = Dialog::NewFolder;
                    }),
            );
            if !self.picker_mode() {
                commands = commands
                    .child(command_divider())
                    .child(
                        icon_button("Cut", "cut")
                            .enabled(!state.selected.is_empty())
                            .on_press(|this: &mut Self| this.copy(TransferMode::Move)),
                    )
                    .child(
                        icon_button("Copy", "copy")
                            .enabled(!state.selected.is_empty())
                            .on_press(|this: &mut Self| this.copy(TransferMode::Copy)),
                    )
                    .child(
                        icon_button("Paste", "paste")
                            .enabled(self.clipboard.is_some() && !state.busy)
                            .on_press(|this: &mut Self| this.paste()),
                    )
                    .child(
                        icon_button("Rename", "rename")
                            .enabled(state.selected.len() == 1 && !state.busy)
                            .on_press(|this: &mut Self| this.rename_prompt()),
                    )
                    .child(
                        icon_button("Trash", "trash")
                            .enabled(!state.selected.is_empty() && !state.busy)
                            .on_press(|this: &mut Self| this.dialog = Dialog::Trash),
                    );
            }
        }
        let target_view = if self.preferences.view_mode == "grid" {
            "List"
        } else {
            "Grid"
        };
        commands
            .child(command_divider())
            .child(
                command("Sort ▾", "sort")
                    .accessible_label(format!(
                        "Sort: {} {}",
                        self.preferences.sort_by,
                        if self.preferences.descending {
                            "↓"
                        } else {
                            "↑"
                        }
                    ))
                    .on_press(|this: &mut Self| this.dialog = Dialog::Sort),
            )
            .child(
                command("View ▾", "list")
                    .accessible_label(target_view)
                    .on_press(|this: &mut Self| this.dialog = Dialog::View),
            )
            .child(
                icon_button("More", "more").on_press(|this: &mut Self| this.dialog = Dialog::More),
            )
            .child(spacer())
            .child(
                command("Details", "details")
                    .accessible_label("Properties")
                    .enabled(state.selected.len() == 1 || state.preview.is_some())
                    .on_press(|this: &mut Self| {
                        let state = this.browser().snapshot();
                        if state.preview.is_some() {
                            this.browser().close_preview();
                        } else if let Some(path) = state.selected.first() {
                            this.browser().preview(path.clone());
                        }
                    }),
            )
    }
}

fn command_divider() -> Container {
    separator().margin(Insets::symmetric(0.0, 4.0))
}
