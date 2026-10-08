use super::*;
use std::path::Path;

impl Explorer {
    pub(super) fn sidebar(&self, state: &Snapshot) -> Container {
        let home = home_directory();
        let common = common_locations(&home);
        let mut displayed = vec![home.clone(), PathBuf::from("/")];
        displayed.extend(common.iter().map(|(_, _, path)| path.clone()));
        let mut bookmarks = Vec::new();
        for path in &self.preferences.bookmarks {
            if path.is_dir() && !displayed.contains(path) {
                displayed.push(path.clone());
                bookmarks.push(path.clone());
            }
        }

        let row_count = 1
            + common.len()
            + 1
            + usize::from(self.computer_expanded)
            + usize::from(!self.picker_mode())
            + bookmarks.len();
        let divider_count = 1 + usize::from(!bookmarks.is_empty());
        let mut navigation = content(row_count as f32 * 34.0 + divider_count as f32 * 17.0)
            .gap(0.0)
            .child(
                navigation_button(
                    "Home",
                    "home",
                    state.trash.is_none() && state.directory == home,
                    false,
                )
                .on_press(move |this: &mut Self| this.navigate(home.clone())),
            );
        for (name, icon, path) in common {
            let selected = state.trash.is_none() && state.directory == path;
            navigation = navigation.child(
                navigation_button(name, icon, selected, true)
                    .on_press(move |this: &mut Self| this.navigate(path.clone())),
            );
        }
        navigation = navigation.child(navigation_divider()).child(
            navigation_base("This PC", false)
                .child(
                    row()
                        .width(Dimension::FILL)
                        .height(20.0)
                        .gap(8.0)
                        .align_items(Alignment::Center)
                        .child(icon_view(
                            if self.computer_expanded {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            },
                            12.0,
                        ))
                        .child(icon_view("computer", 18.0))
                        .child(label("This PC", 13.0, TEXT).width(Dimension::FILL)),
                )
                .on_press(|this: &mut Self| this.computer_expanded = !this.computer_expanded),
        );
        if self.computer_expanded {
            let root = PathBuf::from("/");
            navigation = navigation.child(
                navigation_base(
                    "Local Disk (/)",
                    state.trash.is_none() && state.directory == root,
                )
                .child(
                    row()
                        .width(Dimension::FILL)
                        .height(20.0)
                        .gap(8.0)
                        .align_items(Alignment::Center)
                        .child(column().width(24.0).height(1.0))
                        .child(icon_view("drive", 18.0))
                        .child(label("Local Disk (/)", 13.0, TEXT).width(Dimension::FILL)),
                )
                .on_press(move |this: &mut Self| this.navigate(root.clone())),
            );
        }
        if !self.picker_mode() {
            navigation = navigation.child(
                navigation_button("Recycle Bin", "recycle", state.trash.is_some(), false)
                    .on_press(|this: &mut Self| this.browser().trash_location()),
            );
        }
        if !bookmarks.is_empty() {
            navigation = navigation.child(navigation_divider());
            for path in bookmarks {
                let name = path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .into_owned();
                let selected = state.trash.is_none() && state.directory == path;
                let remove = path.clone();
                navigation = navigation.child(
                    row()
                        .width(Dimension::FILL)
                        .height(34.0)
                        .background(if selected { SELECTED } else { PANEL })
                        .child(
                            navigation_button(&name, "folder", selected, false)
                                .on_press(move |this: &mut Self| this.navigate(path.clone())),
                        )
                        .child(
                            icon_button("Remove bookmark", "pin")
                                .width(28.0)
                                .height(34.0)
                                .padding((8.0, 5.0))
                                .background(if selected { SELECTED } else { PANEL })
                                .on_press(move |this: &mut Self| {
                                    this.preferences.bookmarks.retain(|path| path != &remove);
                                    this.save_preferences();
                                }),
                        ),
                );
            }
        }

        column()
            .width(220.0)
            .height(Dimension::FILL)
            .padding((8.0, 0.0))
            .background(PANEL)
            .border_sides(Border {
                right: BorderSide {
                    width: 1.0,
                    color: LINE,
                },
                ..Default::default()
            })
            .child(
                column()
                    .width(Dimension::FILL)
                    .height(Dimension::FILL)
                    .scrollable()
                    .child(navigation),
            )
    }
}

fn navigation_base(name: &str, selected: bool) -> Button {
    button()
        .accessible_label(name)
        .width(Dimension::FILL)
        .height(34.0)
        .padding((7.0, 12.0))
        .corner_radius(0.0)
        .background(if selected { SELECTED } else { PANEL })
        .hover_effect(InteractionEffect::Background(SELECTED))
        .on_input(|this: &mut Explorer, event| this.control_input(event))
}

fn navigation_button(name: &str, icon: &str, selected: bool, pinned: bool) -> Button {
    let mut contents = row()
        .width(Dimension::FILL)
        .height(20.0)
        .gap(8.0)
        .align_items(Alignment::Center)
        .child(column().width(12.0).height(1.0))
        .child(icon_view(icon, 18.0))
        .child(label(name, 13.0, TEXT).width(Dimension::FILL));
    if pinned {
        contents = contents.child(icon_view("pin", 12.0));
    }
    navigation_base(name, selected).child(contents)
}

fn navigation_divider() -> Container {
    column()
        .width(Dimension::FILL)
        .height(17.0)
        .padding((8.0, 20.0))
        .child(column().width(Dimension::FILL).height(1.0).background(LINE))
}

fn common_locations(home: &Path) -> Vec<(&'static str, &'static str, PathBuf)> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|path| Path::new(path).is_absolute())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let definitions = std::fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    let mut locations = Vec::new();
    for (key, name, icon) in [
        ("XDG_DESKTOP_DIR", "Desktop", "desktop"),
        ("XDG_DOCUMENTS_DIR", "Documents", "document"),
        ("XDG_DOWNLOAD_DIR", "Downloads", "download"),
        ("XDG_PICTURES_DIR", "Pictures", "pictures"),
        ("XDG_MUSIC_DIR", "Music", "music"),
        ("XDG_VIDEOS_DIR", "Videos", "videos"),
    ] {
        let configured = definitions.lines().find_map(|line| {
            let (setting, value) = line.split_once('=')?;
            if setting.trim() != key {
                return None;
            }
            let value = value.trim().strip_prefix('"')?.strip_suffix('"')?;
            let path = if let Some(relative) = value.strip_prefix("$HOME/") {
                home.join(relative)
            } else if value == "$HOME" {
                home.to_path_buf()
            } else {
                PathBuf::from(value)
            };
            path.is_absolute().then_some(path)
        });
        let path = configured.unwrap_or_else(|| home.join(name));
        if path != home
            && path.is_dir()
            && !locations.iter().any(|(_, _, existing)| existing == &path)
        {
            locations.push((name, icon, path));
        }
    }
    locations
}
