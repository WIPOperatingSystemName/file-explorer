use super::*;
use crate::{fs::Entry, model::Preview};
impl Explorer {
    pub(super) fn listing(&self, state: &Snapshot, entries: &[Entry]) -> Container {
        let grid = self.preferences.view_mode == "grid";
        let ordered = Arc::new(
            entries
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>(),
        );
        let mut list = if grid {
            column().width(Dimension::FILL).grid(112, 116).gap(8.0)
        } else {
            content((entries.len() * 30) as f32).gap(0.0)
        };
        for entry in entries {
            let path = entry.path.clone();
            let input_path = path.clone();
            let paths = ordered.clone();
            let keys = ordered.clone();
            let selected = state.selected.contains(&path);
            let icon = entry_icon(entry);
            let mut item = button()
                .key(hashed_key(&entry.path))
                .width(Dimension::FILL)
                .height(if grid { 112.0 } else { 30.0 })
                .padding(if grid { 6.0 } else { 4.0 })
                .corner_radius(2.0)
                .accessible_label(format!(
                    "{} {}",
                    entry.name,
                    if entry.is_dir { "folder" } else { "file" }
                ))
                .background(if selected { SELECTED } else { BG })
                .uniform_border(1.0, if selected { ACCENT.with_alpha(75) } else { BG });
            if grid {
                item = item.child(
                    column()
                        .width(Dimension::FILL)
                        .gap(5.0)
                        .align_items(Alignment::Center)
                        .child(icon_view(icon, 60.0))
                        .child(
                            label(&entry.name, 12.0, TEXT)
                                .height(34.0)
                                .width(Dimension::FILL)
                                .overflow(telorgon::ui::Overflow::Clip)
                                .text_align(Alignment::Center),
                        ),
                );
            } else {
                item = item.child(
                    row()
                        .width(Dimension::FILL)
                        .height(Dimension::FILL)
                        .align_items(Alignment::Center)
                        .child(
                            row()
                                .width(Dimension::FILL)
                                .height(Dimension::FILL)
                                .gap(8.0)
                                .align_items(Alignment::Center)
                                .child(icon_view(icon, 20.0))
                                .child(
                                    label(
                                        format!(
                                            "{}{}",
                                            entry.name,
                                            if entry.is_symlink { " ↗" } else { "" }
                                        ),
                                        12.0,
                                        TEXT,
                                    )
                                    .width(Dimension::FILL)
                                    .overflow(telorgon::ui::Overflow::Clip),
                                ),
                        )
                        .child(
                            label(
                                entry.modified.map(date).unwrap_or_else(|| "—".into()),
                                12.0,
                                MUTED,
                            )
                            .width(160.0),
                        )
                        .child(label(&entry.kind, 12.0, MUTED).width(140.0))
                        .child(
                            label(
                                if entry.is_dir {
                                    String::new()
                                } else {
                                    size(entry.size)
                                },
                                12.0,
                                MUTED,
                            )
                            .width(85.0),
                        ),
                );
            }
            list = list.child(
                item.on_press_event(move |this: &mut Self, event| {
                    if event.source() != telorgon::input::ChangeSource::Keyboard {
                        this.activate(path.clone(), &paths);
                    }
                })
                .on_input(move |this: &mut Self, event| {
                    this.keyboard.set(event.modifiers);
                    if !matches!(this.dialog, Dialog::None) {
                        return this.input(event, &keys);
                    }
                    if matches!(&event.kind, telorgon::ui::UiEventKind::Input(telorgon::InputEvent::PointerButton {
                        button: telorgon::input::PointerButton::SECONDARY, state: telorgon::input::ButtonState::Released, ..
                    })) {
                        if !this.browser().snapshot().selected.contains(&input_path) {
                            this.browser().select(input_path.clone(), false, false, &keys);
                        }
                        this.dialog = Dialog::Context;
                        return true;
                    }
                    if matches!(&event.kind, telorgon::ui::UiEventKind::Input(telorgon::InputEvent::Key(key))
                        if key.state == telorgon::input::ButtonState::Pressed && key.logical_key == telorgon::input::LogicalKey::Named(telorgon::input::NamedKey::Space)) {
                        this.browser().select(input_path.clone(), true, false, &keys); return true;
                    }
                    this.input(event, &keys)
                }),
            );
        }
        let mut pane = column()
            .width(Dimension::FILL)
            .height(Dimension::FILL)
            .padding((8.0, 16.0))
            .gap(4.0)
            .background(BG);
        if !grid {
            pane = pane.child(
                row()
                    .width(Dimension::FILL)
                    .height(30.0)
                    .padding((0.0, 4.0))
                    .border_sides(Border {
                        bottom: BorderSide {
                            width: 1.0,
                            color: LINE,
                        },
                        ..Default::default()
                    })
                    .child(self.column_header("Name", "name", Dimension::FILL))
                    .child(self.column_header(
                        "Date modified",
                        "modified",
                        Dimension::Logical(160.0),
                    ))
                    .child(self.column_header("Type", "type", Dimension::Logical(140.0)))
                    .child(self.column_header("Size", "size", Dimension::Logical(85.0))),
            );
        }
        if entries.is_empty() {
            pane = pane.child(
                column()
                    .width(Dimension::FILL)
                    .height(Dimension::FILL)
                    .padding((36.0, 12.0))
                    .gap(12.0)
                    .align_items(Alignment::Center)
                    .child(label(
                        if state.loading {
                            "Reading files…"
                        } else if state.error {
                            "Unable to read this location"
                        } else if !state.query.is_empty() {
                            "No matching files"
                        } else {
                            "This folder is empty"
                        },
                        13.0,
                        MUTED,
                    )),
            );
        } else {
            pane = pane.child(
                column()
                    .width(Dimension::FILL)
                    .height(Dimension::FILL)
                    .scrollable()
                    .child(list),
            );
        }
        pane
    }
    fn column_header(&self, title: &'static str, sort: &'static str, width: Dimension) -> Button {
        let active = self.preferences.sort_by == sort;
        let caption = if active {
            format!(
                "{title} {}",
                if self.preferences.descending {
                    "⌄"
                } else {
                    "⌃"
                }
            )
        } else {
            title.into()
        };
        button()
            .width(width)
            .height(Dimension::FILL)
            .padding((4.0, 8.0))
            .background(BG)
            .border_sides(Border {
                right: BorderSide {
                    width: 1.0,
                    color: LINE,
                },
                ..Default::default()
            })
            .accessible_label(format!("Sort by {title}"))
            .child(label(caption, 12.0, MUTED).width(Dimension::FILL))
            .on_input(|this: &mut Self, event| this.control_input(event))
            .on_press(move |this: &mut Self| {
                if this.preferences.sort_by == sort {
                    this.preferences.descending = !this.preferences.descending;
                } else {
                    this.preferences.sort_by = sort.into();
                    this.preferences.descending = false;
                }
                this.save_preferences();
            })
    }

    pub(super) fn preview_panel(&self, preview: &Preview) -> Container {
        let filename = preview
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        // This SDK requires an explicit height for scroll contents. Estimate wrapped
        // lines conservatively so previews can reach their final line at any viewport.
        let text_height = |value: &str, font_size: f32| {
            let lines = value
                .lines()
                .map(|line| line.chars().count().div_ceil(30).max(1))
                .sum::<usize>()
                .max(1);
            lines as f32 * font_size * 1.25 + 4.0
        };
        let filename_height = text_height(&filename, 14.0).max(40.0);
        let detail_height = text_height(&preview.detail, 12.0);
        let preview_text = preview
            .text
            .as_ref()
            .map(|value| value.chars().take(3500).collect::<String>());
        let preview_text_height = preview_text
            .as_ref()
            .map(|value| text_height(value, 12.0))
            .unwrap_or(0.0);
        let image_height = if preview.image.is_some() { 160.0 } else { 0.0 };
        let open_height = if self.picker_mode() { 0.0 } else { 34.0 };
        let element_count = 2
            + usize::from(preview.image.is_some())
            + usize::from(preview_text.is_some())
            + usize::from(!self.picker_mode());
        let body_height = filename_height
            + detail_height
            + preview_text_height
            + image_height
            + open_height
            + (element_count - 1) as f32 * 12.0;
        let mut details = content(body_height)
            .gap(12.0)
            .children(preview.image.as_ref().map(|resource| {
                Image::resource(resource.clone())
                    .width(Dimension::FILL)
                    .height(image_height)
            }))
            .child(
                label(filename, 14.0, TEXT)
                    .height(filename_height)
                    .width(Dimension::FILL)
                    .overflow(telorgon::ui::Overflow::Clip),
            )
            .child(
                label(&preview.detail, 12.0, MUTED)
                    .height(detail_height)
                    .width(Dimension::FILL)
                    .overflow(telorgon::ui::Overflow::Clip),
            )
            .children(preview_text.as_ref().map(|value| {
                label(value, 12.0, TEXT)
                    .height(preview_text_height)
                    .width(Dimension::FILL)
                    .overflow(telorgon::ui::Overflow::Clip)
            }));
        if !self.picker_mode() {
            let path = preview.path.clone();
            details = details.child(
                control("Open in default application")
                    .width(Dimension::FILL)
                    .height(open_height)
                    .on_press(move |this: &mut Self| this.browser().open(path.clone())),
            );
        }
        column()
            .width(280.0)
            .height(Dimension::FILL)
            .padding(16.0)
            .gap(12.0)
            .background(BG)
            .border_sides(Border {
                left: BorderSide {
                    width: 1.0,
                    color: LINE,
                },
                ..Default::default()
            })
            .child(
                row()
                    .height(28.0)
                    .align_items(Alignment::Center)
                    .child(label("Details", 14.0, TEXT).width(Dimension::FILL))
                    .child(
                        status_control("×", "")
                            .accessible_label("Close preview")
                            .on_press(|this: &mut Self| this.browser().close_preview()),
                    ),
            )
            .child(
                column()
                    .width(Dimension::FILL)
                    .height(Dimension::FILL)
                    .scrollable()
                    .child(details),
            )
    }
    pub(super) fn footer(
        &self,
        state: &Snapshot,
        count: usize,
        jobs: &[crate::downloads::DownloadJob],
    ) -> Container {
        let selected_size = state
            .entries
            .iter()
            .filter(|entry| !entry.is_dir && state.selected.contains(&entry.path))
            .map(|entry| entry.size)
            .sum::<u64>();
        let selection = if state.selected.is_empty() {
            String::new()
        } else if selected_size == 0 {
            format!("   {} selected", state.selected.len())
        } else {
            format!(
                "   {} selected · {}",
                state.selected.len(),
                size(selected_size)
            )
        };
        let mut footer = row()
            .width(Dimension::FILL)
            .height(28.0)
            .padding((0.0, 12.0))
            .gap(8.0)
            .background(PANEL)
            .border_sides(Border {
                top: BorderSide {
                    width: 1.0,
                    color: LINE,
                },
                ..Default::default()
            })
            .align_items(Alignment::Center)
            .child(label(
                format!(
                    "{count} items{}{selection}",
                    if state.loading { " · loading" } else { "" }
                ),
                11.0,
                MUTED,
            ));
        if !state.message.is_empty() {
            footer = footer.child(
                label(&state.message, 11.0, if state.error { RED } else { GREEN })
                    .width(Dimension::FILL),
            );
        } else {
            footer = footer.child(spacer());
        }
        if !state.undo_trash.is_empty() && !self.picker_mode() {
            footer = footer.child(
                status_control("Undo trash", "")
                    .enabled(!state.busy)
                    .on_press(|this: &mut Self| this.browser().undo_trash()),
            );
        }
        if !self.picker_mode() && !jobs.is_empty() {
            footer = footer.child(
                status_control(format!("Downloads ({})", jobs.len()), "download")
                    .on_press(|this: &mut Self| this.downloads_open = !this.downloads_open),
            );
        }
        footer
    }
}

fn status_control(value: impl ToString, icon: &str) -> Button {
    let value = value.to_string();
    let label_width = value.chars().count() as f32 * 6.0;
    let mut contents = row()
        .height(Dimension::FILL)
        .gap(5.0)
        .align_items(Alignment::Center);
    if !icon.is_empty() {
        contents = contents.child(icon_view(icon, 14.0));
    }
    contents = contents.child(label(&value, 11.0, TEXT));
    button()
        .width(label_width + if icon.is_empty() { 12.0 } else { 31.0 })
        .height(24.0)
        .padding((2.0, 6.0))
        .corner_radius(2.0)
        .background(PANEL)
        .accessible_label(value)
        .child(contents)
        .on_input(|this: &mut Explorer, event| this.control_input(event))
}

fn entry_icon(entry: &Entry) -> &'static str {
    if entry.is_dir {
        return "folder";
    }
    match entry
        .path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "svg" | "bmp" | "avif" => "image",
        "pdf" => "pdf",
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst" => "archive",
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" => "audio",
        "mp4" | "mkv" | "mov" | "webm" | "avi" => "video",
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "html" | "css" | "json" | "toml" | "sh" => {
            "code"
        }
        "txt" | "md" | "doc" | "docx" | "odt" | "rtf" => "document",
        _ => "file",
    }
}
