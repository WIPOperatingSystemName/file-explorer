use super::*;

#[test]
fn minimum_and_tiled_windows_keep_list_and_picker_controls_inside_visible_bounds() {
    let folder = Folder::new();
    for width in [800, 957, 1080] {
        for request in [
            None,
            Some(PickerRequest {
                mode: PickerMode::Save,
                current_name: Some("new.txt".into()),
                ..Default::default()
            }),
            Some(PickerRequest {
                multiple: true,
                accept_label: Some("Open selected documents and folders".into()),
                ..Default::default()
            }),
        ] {
            let picker = request.is_some();
            let save = request.as_ref().is_some_and(PickerRequest::is_save);
            let accept_label = request
                .as_ref()
                .and_then(|request| request.accept_label.as_deref())
                .unwrap_or("Save")
                .to_owned();
            let explorer = folder.explorer(request, false);
            let mut runtime = telorgon::ComposedAppRuntime::from_composed_with_extent(
                explorer,
                SizeI { width, height: 680 },
            )
            .unwrap();
            runtime.register_assets(crate::assets::bundle()).unwrap();
            runtime
                .prepare_frame(telorgon::MonotonicInstant::ZERO, true)
                .unwrap();
            let mut names = vec![
                "Location",
                "Search",
                "New folder",
                "Sort: name ↑",
                "Grid",
                "alpha.txt file",
            ];
            if picker {
                if save {
                    names.push("File name");
                }
                names.extend([accept_label.as_str(), "Cancel", "File type: All files"]);
            } else {
                names.extend([
                    "Cut",
                    "Copy",
                    "Paste",
                    "Rename",
                    "Trash",
                    "More",
                    "Properties",
                ]);
            }
            for name in names {
                let node = named_button(runtime.ui(), name)
                    .unwrap_or_else(|| panic!("missing control {name}"));
                let layout = runtime.layout().computed(node).unwrap();
                assert!(
                    layout.border_rect.x >= 0.0
                        && layout.border_rect.right() <= width as f32 + 0.5
                        && layout.border_rect.y >= 0.0
                        && layout.border_rect.bottom() <= 680.5,
                    "{name} is outside minimum window: {:?}",
                    layout.border_rect
                );
                assert!(
                    layout.visible_rect.width >= layout.border_rect.width - 0.5
                        && layout.visible_rect.height >= layout.border_rect.height - 0.5,
                    "{name} is clipped in minimum window: {:?} visible {:?}",
                    layout.border_rect,
                    layout.visible_rect
                );
            }
        }
    }
}

#[test]
fn maximum_picker_choices_keep_destination_and_accept_controls_visible() {
    let folder = Folder::new();
    let request = PickerRequest {
        mode: PickerMode::Save,
        current_name: Some("Report.txt".into()),
        choices: (0..64)
            .map(|index| crate::protocol::PickerChoice {
                id: format!("choice-{index}"),
                label: format!("Choice {index}"),
                options: vec![],
                selected: "false".into(),
            })
            .collect(),
        ..Default::default()
    };
    let mut runtime = telorgon::ComposedAppRuntime::from_composed_with_extent(
        folder.explorer(Some(request), false),
        SizeI {
            width: 800,
            height: 680,
        },
    )
    .unwrap();
    runtime.register_assets(crate::assets::bundle()).unwrap();
    runtime
        .prepare_frame(telorgon::MonotonicInstant::ZERO, true)
        .unwrap();
    for name in ["File name", "File type: All files", "Save", "Cancel"] {
        let node = named_button(runtime.ui(), name).unwrap();
        let layout = runtime.layout().computed(node).unwrap();
        assert!(
            layout.border_rect.bottom() <= 680.5,
            "{name} escaped window: {:?}",
            layout.border_rect
        );
        assert!(
            layout.visible_rect.height >= layout.border_rect.height - 0.5,
            "{name} was clipped"
        );
    }
    let node = named_button(runtime.ui(), "alpha.txt file").unwrap();
    assert!(runtime.layout().computed(node).unwrap().visible_rect.height > 20.0);
}
