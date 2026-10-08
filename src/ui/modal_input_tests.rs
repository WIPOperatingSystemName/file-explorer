use super::*;
use telorgon::ui::InteractionFlags;

fn enter(runtime: &mut telorgon::ComposedAppRuntime, tick: &mut u64) {
    host_key(
        runtime,
        LogicalKey::Named(NamedKey::Enter),
        None,
        Modifiers::empty(),
        tick,
    );
}

fn escape(runtime: &mut telorgon::ComposedAppRuntime, tick: &mut u64) {
    host_key(
        runtime,
        LogicalKey::Named(NamedKey::Escape),
        None,
        Modifiers::empty(),
        tick,
    );
}

fn focused(runtime: &telorgon::ComposedAppRuntime) -> UiNodeId {
    runtime
        .ui()
        .interactions
        .iter()
        .find_map(|(node, interaction)| {
            interaction
                .flags
                .contains(InteractionFlags::FOCUSED)
                .then_some(node)
        })
        .expect("no focused control")
}

#[test]
fn save_picker_draft_survives_folder_dialog_and_enter_submits_the_filename() {
    let folder = Folder::new();
    let explorer = folder.explorer(
        Some(PickerRequest {
            mode: PickerMode::Save,
            current_name: Some("Report.txt".into()),
            ..Default::default()
        }),
        false,
    );
    let completion = explorer.completion.clone();
    let picker_name = explorer.picker_name.clone();
    let modal_name = explorer.name.clone();
    let workers = explorer.file_workers();
    let browser = explorer.browser().clone();
    let mut runtime = host_explorer(explorer);
    let mut tick = 0;
    host_click(&mut runtime, "New folder", &mut tick);
    host_type(&mut runtime, "Cancelled folder", &mut tick);
    let paste_target = modal_name.0.editor.lock().unwrap().target();
    escape(&mut runtime, &mut tick);
    assert!(named_button(runtime.ui(), "Name").is_none());
    assert_eq!(picker_name.text(), "Report.txt");
    assert!(!folder.0.join("Cancelled folder").exists());
    assert!(!modal_name.0.presentation.lock().unwrap().focused);
    assert!(matches!(
        modal_name
            .0
            .editor
            .lock()
            .unwrap()
            .paste(paste_target, "late clipboard"),
        Err(telorgon::services::clipboard::ClipboardError::Stale)
    ));
    host_click(&mut runtime, "New folder", &mut tick);
    host_type(&mut runtime, "Created folder", &mut tick);
    enter(&mut runtime, &mut tick);
    workers.finish();
    wait_loaded(&browser);
    host_input(&mut runtime, [], &mut tick);
    assert!(folder.0.join("Created folder").is_dir());
    assert_eq!(picker_name.text(), "Report.txt");
    host_click(&mut runtime, "File name", &mut tick);
    host_type(&mut runtime, "Final café report.txt", &mut tick);
    enter(&mut runtime, &mut tick);
    let response = completion
        .lock()
        .unwrap()
        .clone()
        .expect("filename Enter did not submit");
    assert!(!response.cancelled);
    assert_eq!(
        response.uris,
        vec![crate::protocol::path_to_uri(&folder.0.join("Final café report.txt")).unwrap()]
    );
}

#[test]
fn rename_shortcut_focuses_and_selects_name_then_enter_renames_the_file() {
    let folder = Folder::new();
    let explorer = folder.explorer(None, false);
    let name = explorer.name.clone();
    let workers = explorer.file_workers();
    let browser = explorer.browser().clone();
    let mut runtime = host_explorer(explorer);
    let mut tick = 0;
    host_click(&mut runtime, "alpha.txt file", &mut tick);
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::F2),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    assert_eq!(
        focused(&runtime),
        named_button(runtime.ui(), "Name").unwrap()
    );
    host_type(&mut runtime, "Renamed report.txt", &mut tick);
    assert_eq!(name.text(), "Renamed report.txt");
    enter(&mut runtime, &mut tick);
    workers.finish();
    wait_loaded(&browser);
    host_input(&mut runtime, [], &mut tick);
    assert!(!folder.0.join("alpha.txt").exists());
    assert_eq!(
        fs::read_to_string(folder.0.join("Renamed report.txt")).unwrap(),
        "First document\n"
    );
    assert!(named_button(runtime.ui(), "Name").is_none());
}

#[test]
fn modal_tab_focus_and_escape_cannot_route_shortcuts_to_background_files() {
    let folder = Folder::new();
    let explorer = folder.explorer(None, false);
    let browser = explorer.browser().clone();
    let mut runtime = host_explorer(explorer);
    let mut tick = 0;
    host_click(&mut runtime, "alpha.txt file", &mut tick);
    let trigger = host_click(&mut runtime, "New folder", &mut tick);
    let scope = runtime
        .ui()
        .interactions
        .iter()
        .find_map(|(node, interaction)| interaction.focus_scope.then_some(node))
        .unwrap();
    for _ in 0..12 {
        assert!(runtime.ui().is_descendant_or_self(focused(&runtime), scope));
        host_key(
            &mut runtime,
            LogicalKey::Named(NamedKey::Tab),
            None,
            Modifiers::empty(),
            &mut tick,
        );
    }
    let selected = browser.snapshot().selected;
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::Backspace),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    host_key(
        &mut runtime,
        LogicalKey::Character(KeyText::new("w").unwrap()),
        None,
        Modifiers::CONTROL,
        &mut tick,
    );
    assert_eq!(browser.snapshot().directory, folder.0);
    assert_eq!(browser.snapshot().selected, selected);
    escape(&mut runtime, &mut tick);
    assert!(named_button(runtime.ui(), "Name").is_none());
    assert_eq!(
        focused(&runtime),
        trigger,
        "closing the modal did not restore its trigger's focus"
    );
    host_click(&mut runtime, "More", &mut tick);
    escape(&mut runtime, &mut tick);
    assert!(named_button(runtime.ui(), "Dismiss dialog").is_none());
}

#[test]
fn url_and_download_filename_enter_validate_then_escape_closes_the_prompt() {
    let folder = Folder::new();
    let explorer = folder.explorer(None, false);
    let browser = explorer.browser().clone();
    let mut runtime = host_explorer(explorer);
    let mut tick = 0;
    host_click(&mut runtime, "More", &mut tick);
    host_click(&mut runtime, "Download URL", &mut tick);
    assert_eq!(
        focused(&runtime),
        named_button(runtime.ui(), "Download URL").unwrap()
    );
    host_type(&mut runtime, "not a URL", &mut tick);
    enter(&mut runtime, &mut tick);
    assert!(browser.snapshot().error);
    assert_eq!(
        browser.snapshot().message,
        "Enter a complete HTTP or HTTPS URL."
    );
    assert!(named_button(runtime.ui(), "Download URL").is_some());
    browser.message("", false);
    host_click(&mut runtime, "File name", &mut tick);
    host_type(&mut runtime, "download.txt", &mut tick);
    enter(&mut runtime, &mut tick);
    assert!(
        browser.snapshot().error,
        "download filename Enter did not submit"
    );
    assert_eq!(
        browser.snapshot().message,
        "Enter a complete HTTP or HTTPS URL."
    );
    escape(&mut runtime, &mut tick);
    assert!(named_button(runtime.ui(), "Download URL").is_none());
}

#[test]
fn relative_address_enter_keeps_resolved_location_and_invalid_draft_preserves_directory() {
    let folder = Folder::new();
    let explorer = folder.explorer(None, false);
    let browser = explorer.browser().clone();
    let location = explorer.location.clone();
    let mut runtime = host_explorer(explorer);
    let mut tick = 0;
    host_click(&mut runtime, "Location", &mut tick);
    host_type(&mut runtime, "folder", &mut tick);
    enter(&mut runtime, &mut tick);
    wait_loaded(&browser);
    assert_eq!(browser.snapshot().directory, folder.0.join("folder"));
    assert_eq!(location.text(), folder.0.join("folder").to_string_lossy());
    enter(&mut runtime, &mut tick);
    wait_loaded(&browser);
    assert_eq!(browser.snapshot().directory, folder.0.join("folder"));
    host_select_all(&mut runtime, &mut tick);
    host_type(&mut runtime, "missing-folder", &mut tick);
    enter(&mut runtime, &mut tick);
    assert!(browser.snapshot().error);
    assert_eq!(browser.snapshot().directory, folder.0.join("folder"));
    assert_eq!(location.text(), "missing-folder");
    escape(&mut runtime, &mut tick);
    assert_eq!(location.text(), folder.0.join("folder").to_string_lossy());
}

#[test]
fn control_l_focuses_and_selects_address_from_files_controls_and_text_fields() {
    let folder = Folder::new();
    for origin in ["alpha.txt file", "Refresh", "Search"] {
        let explorer = folder.explorer(None, false);
        let browser = explorer.browser().clone();
        let location = explorer.location.clone();
        let search = explorer.search.clone();
        let mut runtime = host_explorer(explorer);
        let mut tick = 0;
        host_click(&mut runtime, origin, &mut tick);
        wait_loaded(&browser);
        if origin == "Search" {
            host_type(&mut runtime, "retained search draft", &mut tick);
        }
        host_key(
            &mut runtime,
            LogicalKey::Character(KeyText::new("l").unwrap()),
            None,
            Modifiers::CONTROL,
            &mut tick,
        );
        assert_eq!(
            focused(&runtime),
            named_button(runtime.ui(), "Location").unwrap(),
            "Ctrl+L from {origin} did not focus the address"
        );
        if origin == "Search" {
            assert_eq!(search.text(), "retained search draft");
        }
        host_type(&mut runtime, "folder", &mut tick);
        assert_eq!(
            location.text(),
            "folder",
            "Ctrl+L did not select the old path"
        );
        enter(&mut runtime, &mut tick);
        wait_loaded(&browser);
        assert_eq!(browser.snapshot().directory, folder.0.join("folder"));
    }
}
