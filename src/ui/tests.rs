//! Mount the real components and dispatch their real owner-bound callbacks.
//! Fixtures never change the user's preferences or launch external applications.
use super::*;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};
use telorgon::{
    ChangeSource, CompositionDriver, NodeKind, PointF, SemanticName, SemanticRole, SizeI,
    ViewRuntime,
    input::{
        ButtonState, KeyEvent, KeyText, LogicalKey, Modifiers, NamedKey, PhysicalKey, PointerButton,
    },
    ui::{MountedUi, UiEventKind, UiNodeId},
};

type MountedExplorer = ViewRuntime<CompositionDriver>;
static NEXT_FOLDER: AtomicU64 = AtomicU64::new(1);

#[path = "modal_input_tests.rs"]
mod modal_input_tests;

struct Folder(PathBuf);
impl Folder {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "telorgon-files-ui-{}-{}",
            std::process::id(),
            NEXT_FOLDER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        fs::create_dir(directory.join("folder")).unwrap();
        fs::write(directory.join("alpha.txt"), "First document\n").unwrap();
        fs::write(directory.join("beta.png"), "Image fixture").unwrap();
        fs::write(directory.join("gamma.txt"), "Third document\n").unwrap();
        fs::write(directory.join(".hidden"), "Hidden fixture").unwrap();
        Self(directory)
    }
    fn explorer(&self, request: Option<PickerRequest>, grid: bool) -> Explorer {
        let mut explorer = Explorer::new(self.0.clone(), request, Arc::new(Mutex::new(None)));
        explorer.preferences = Preferences::default();
        explorer.preferences.show_hidden = false;
        explorer.preferences.view_mode = if grid { "grid" } else { "list" }.into();
        explorer.preferences.sort_by = "name".into();
        explorer.preferences.descending = false;
        wait_loaded(explorer.browser());
        explorer
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn wait_loaded(browser: &Browser) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while browser.snapshot().loading {
        assert!(Instant::now() < deadline, "fixture listing did not finish");
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!browser.snapshot().error, "fixture listing failed");
}

fn descendant_text(ui: &MountedUi, node: UiNodeId) -> String {
    let mut value = String::new();
    if let Some(text) = ui.texts.get(node) {
        value.push_str(ui.string(text.content).unwrap_or_default());
    }
    for child in ui.nodes.children(node) {
        value.push_str(&descendant_text(ui, child));
    }
    value
}

fn named_button(ui: &MountedUi, name: &str) -> Option<UiNodeId> {
    ui.semantics.iter().find_map(|(node, semantics)| {
        if !matches!(
            semantics.role,
            SemanticRole::Button | SemanticRole::TextInput | SemanticRole::SearchBox
        ) {
            return None;
        }
        let value = match semantics.name {
            SemanticName::Text(value) => ui.string(value).unwrap_or_default().to_owned(),
            SemanticName::Contents => descendant_text(ui, node),
            _ => String::new(),
        };
        (value == name).then_some(node)
    })
}

fn button(runtime: &MountedExplorer, name: &str) -> UiNodeId {
    named_button(runtime.ui(), name).unwrap_or_else(|| panic!("missing mounted button: {name}"))
}

fn press(runtime: &mut MountedExplorer, name: &str) {
    let node = button(runtime, name);
    assert!(runtime.dispatch_activation(node, ChangeSource::Programmatic));
}

fn key(
    runtime: &mut MountedExplorer,
    target: UiNodeId,
    logical: LogicalKey,
    text: Option<&str>,
    modifiers: Modifiers,
) {
    let event = KeyEvent::new(PhysicalKey::UNIDENTIFIED, ButtonState::Pressed)
        .with_logical_key(logical)
        .with_text(text.map(|text| KeyText::new(text).unwrap()))
        .with_modifiers(modifiers);
    runtime.dispatch_ui(
        target,
        UiEventKind::Input(telorgon::InputEvent::Key(event)),
        u16::MAX,
        1,
    );
}

#[test]
fn mounts_list_grid_and_filtered_picker_with_valid_callback_owners() {
    let folder = Folder::new();
    for grid in [false, true] {
        let explorer = folder.explorer(None, grid);
        let runtime = ViewRuntime::from_composed(explorer).unwrap();
        assert!(named_button(runtime.ui(), "alpha.txt file").is_some());
        assert!(named_button(runtime.ui(), "folder folder").is_some());
        assert!(named_button(runtime.ui(), ".hidden file").is_none());
        assert!(named_button(runtime.ui(), "New tab").is_some());
        assert_eq!(
            runtime.composition_diagnostics().input_mutations_restored,
            0
        );
    }
    let request = PickerRequest {
        filters: vec![crate::protocol::FileFilter {
            name: "Text documents".into(),
            rules: vec![crate::protocol::FilterRule {
                kind: 0,
                pattern: "*.txt".into(),
            }],
        }],
        ..Default::default()
    };
    let runtime = ViewRuntime::from_composed(folder.explorer(Some(request), false)).unwrap();
    assert!(named_button(runtime.ui(), "alpha.txt file").is_some());
    assert!(named_button(runtime.ui(), "beta.png file").is_none());
    assert!(named_button(runtime.ui(), "folder folder").is_some());
    assert!(named_button(runtime.ui(), "New tab").is_none());
    assert!(named_button(runtime.ui(), "File type: Text documents").is_some());
}

#[test]
fn field_keyboard_input_edits_shared_value_and_enter_updates_browser() {
    let folder = Folder::new();
    let explorer = folder.explorer(None, false);
    let browser = explorer.browser().clone();
    let location = explorer.location.clone();
    let mut runtime = ViewRuntime::from_composed(explorer).unwrap();
    let target = button(&runtime, "Location");
    runtime.dispatch_ui(target, UiEventKind::Focus(true), u16::MAX, 0);
    key(
        &mut runtime,
        target,
        LogicalKey::Character(KeyText::new("a").unwrap()),
        None,
        Modifiers::CONTROL,
    );
    let release = KeyEvent::new(PhysicalKey::UNIDENTIFIED, ButtonState::Released)
        .with_logical_key(LogicalKey::Named(NamedKey::Control));
    runtime.dispatch_ui(
        target,
        UiEventKind::Input(telorgon::InputEvent::Key(release)),
        u16::MAX,
        2,
    );
    for character in folder.0.join("folder").to_string_lossy().chars() {
        let text = character.to_string();
        key(
            &mut runtime,
            target,
            LogicalKey::Character(KeyText::new(&text).unwrap()),
            Some(&text),
            Modifiers::empty(),
        );
    }
    assert_eq!(location.text(), folder.0.join("folder").to_string_lossy());
    assert!(
        browser.snapshot().selected.is_empty(),
        "field shortcut escaped into the listing owner"
    );
    key(
        &mut runtime,
        target,
        LogicalKey::Named(NamedKey::Enter),
        None,
        Modifiers::empty(),
    );
    wait_loaded(&browser);
    runtime.process_external_updates();
    assert_eq!(browser.snapshot().directory, folder.0.join("folder"));
    assert!(browser.snapshot().entries.is_empty());
    assert_eq!(
        runtime.composition_diagnostics().input_mutations_restored,
        0
    );
}

#[test]
fn enter_submits_location_and_search_in_the_active_tab() {
    let folder = Folder::new();
    let mut explorer = folder.explorer(None, false);
    let original = explorer.browser().clone();
    explorer.new_tab();
    let active = explorer.browser().clone();
    let location = explorer.location.clone();
    let search = explorer.search.clone();
    let mut runtime = ViewRuntime::from_composed(explorer).unwrap();
    let target = button(&runtime, "Location");
    runtime.dispatch_ui(target, UiEventKind::Focus(true), u16::MAX, 0);
    location.set(folder.0.join("folder").to_string_lossy());
    key(
        &mut runtime,
        target,
        LogicalKey::Named(NamedKey::Enter),
        None,
        Modifiers::empty(),
    );
    wait_loaded(&active);
    runtime.process_external_updates();
    assert_eq!(active.snapshot().directory, folder.0.join("folder"));
    assert_eq!(original.snapshot().directory, folder.0);
    assert!(named_button(runtime.ui(), "Go").is_none());
    search.set("needle");
    let target = button(&runtime, "Search");
    key(
        &mut runtime,
        target,
        LogicalKey::Named(NamedKey::Enter),
        None,
        Modifiers::empty(),
    );
    assert_eq!(active.snapshot().query, "needle");
    assert!(original.snapshot().query.is_empty());
}

fn host_input(
    runtime: &mut telorgon::ComposedAppRuntime,
    events: impl IntoIterator<Item = telorgon::InputEvent>,
    tick: &mut u64,
) {
    for event in events {
        runtime.queue_input(event);
    }
    *tick += 1;
    let time = telorgon::MonotonicInstant::from_nanos(*tick * 1_000_000);
    runtime.flush_input(time);
    runtime.prepare_frame(time, false).unwrap();
}

fn host_click(runtime: &mut telorgon::ComposedAppRuntime, name: &str, tick: &mut u64) -> UiNodeId {
    let target = named_button(runtime.ui(), name)
        .unwrap_or_else(|| panic!("missing mounted button: {name}"));
    let bounds = runtime.layout().computed(target).unwrap().border_rect;
    let center = PointF {
        x: bounds.x + bounds.width / 2.0,
        y: bounds.y + bounds.height / 2.0,
    };
    host_input(
        runtime,
        [
            telorgon::InputEvent::mouse_moved(center),
            telorgon::InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Pressed),
            telorgon::InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Released),
        ],
        tick,
    );
    target
}

fn host_key(
    runtime: &mut telorgon::ComposedAppRuntime,
    logical: LogicalKey,
    text: Option<&str>,
    modifiers: Modifiers,
    tick: &mut u64,
) {
    let press = KeyEvent::new(PhysicalKey::UNIDENTIFIED, ButtonState::Pressed)
        .with_logical_key(logical.clone())
        .with_text(text.map(|value| KeyText::new(value).unwrap()))
        .with_modifiers(modifiers);
    let release = KeyEvent::new(PhysicalKey::UNIDENTIFIED, ButtonState::Released)
        .with_logical_key(logical)
        .with_modifiers(modifiers);
    host_input(
        runtime,
        [
            telorgon::InputEvent::Key(press),
            telorgon::InputEvent::Key(release),
        ],
        tick,
    );
}

fn host_type(runtime: &mut telorgon::ComposedAppRuntime, value: &str, tick: &mut u64) {
    for character in value.chars() {
        let text = character.to_string();
        let logical = if character == ' ' {
            LogicalKey::Named(NamedKey::Space)
        } else {
            LogicalKey::Character(KeyText::new(&text).unwrap())
        };
        host_key(runtime, logical, Some(&text), Modifiers::empty(), tick);
    }
}

fn host_select_all(runtime: &mut telorgon::ComposedAppRuntime, tick: &mut u64) {
    host_input(
        runtime,
        [telorgon::InputEvent::Key(
            KeyEvent::new(PhysicalKey::UNIDENTIFIED, ButtonState::Pressed)
                .with_logical_key(LogicalKey::Named(NamedKey::Control))
                .with_modifiers(Modifiers::CONTROL),
        )],
        tick,
    );
    host_key(
        runtime,
        LogicalKey::Character(KeyText::new("a").unwrap()),
        None,
        Modifiers::CONTROL,
        tick,
    );
    host_input(
        runtime,
        [telorgon::InputEvent::Key(
            KeyEvent::new(PhysicalKey::UNIDENTIFIED, ButtonState::Released)
                .with_logical_key(LogicalKey::Named(NamedKey::Control)),
        )],
        tick,
    );
}

fn host_explorer(explorer: Explorer) -> telorgon::ComposedAppRuntime {
    let mut runtime = telorgon::ComposedAppRuntime::from_composed_with_extent(
        explorer,
        SizeI {
            width: 1240,
            height: 800,
        },
    )
    .unwrap();
    runtime.register_assets(crate::assets::bundle()).unwrap();
    runtime
        .prepare_frame(telorgon::MonotonicInstant::ZERO, false)
        .unwrap();
    runtime
}

#[test]
fn pointer_focused_search_edits_unicode_spaces_and_submits_without_listing_shortcuts() {
    let folder = Folder::new();
    let matching = folder.0.join("café report.txt");
    fs::write(&matching, "Search fixture").unwrap();
    let explorer = folder.explorer(None, false);
    let browser = explorer.browser().clone();
    let search = explorer.search.clone();
    browser.select_initial(folder.0.join("alpha.txt"));
    let selected = browser.snapshot().selected;
    let mut runtime = host_explorer(explorer);
    let mut tick = 0;
    let target = host_click(&mut runtime, "Search", &mut tick);
    assert!(
        runtime.ui().box_styles.iter().any(|(node, style)| {
            runtime.ui().is_descendant_or_self(node, target)
                && style.decoration.background == telorgon::ui::Background::Color(TEXT)
                && runtime.layout().computed(node).is_some_and(|layout| {
                    layout.visible_rect.width > 0.0
                        && layout.visible_rect.width <= 2.0
                        && layout.visible_rect.height >= 12.0
                })
        }),
        "focused empty search has no visible caret"
    );
    host_type(&mut runtime, "temporary α β", &mut tick);
    assert_eq!(search.text(), "temporary α β");
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::Backspace),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    assert_eq!(search.text(), "temporary α ");
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::ArrowLeft),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    host_type(&mut runtime, "Ω", &mut tick);
    assert_eq!(search.text(), "temporary αΩ ");
    host_select_all(&mut runtime, &mut tick);
    host_type(&mut runtime, "café report", &mut tick);
    assert_eq!(search.text(), "café report");
    assert_eq!(
        browser.snapshot().selected,
        selected,
        "text editing changed the file selection"
    );
    assert_eq!(
        named_button(runtime.ui(), "Search"),
        Some(target),
        "typing replaced the focused field"
    );
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::Enter),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    wait_loaded(&browser);
    assert_eq!(browser.snapshot().query, "café report");
    assert_eq!(
        browser
            .snapshot()
            .entries
            .iter()
            .map(|entry| &entry.path)
            .collect::<Vec<_>>(),
        vec![&matching]
    );
}

#[test]
fn first_pointer_click_address_replaces_path_and_accepts_encoded_file_uri_with_retained_focus() {
    let folder = Folder::new();
    let destination = folder.0.join("Project café");
    fs::create_dir(&destination).unwrap();
    for encoded_uri in [false, true] {
        let explorer = folder.explorer(None, false);
        let browser = explorer.browser().clone();
        let location = explorer.location.clone();
        let mut runtime = host_explorer(explorer);
        let mut tick = 0;
        let target = host_click(&mut runtime, "Location", &mut tick);
        let address = if encoded_uri {
            crate::protocol::path_to_uri(&destination).unwrap()
        } else {
            destination.to_string_lossy().into_owned()
        };
        host_type(&mut runtime, &address, &mut tick);
        assert_eq!(
            location.text(),
            address,
            "the first click failed to replace the old path"
        );
        assert!(browser.snapshot().selected.is_empty());
        assert_eq!(named_button(runtime.ui(), "Location"), Some(target));
        host_key(
            &mut runtime,
            LogicalKey::Named(NamedKey::Enter),
            None,
            Modifiers::empty(),
            &mut tick,
        );
        wait_loaded(&browser);
        host_input(&mut runtime, [], &mut tick);
        assert_eq!(browser.snapshot().directory, destination);
        assert_eq!(
            named_button(runtime.ui(), "Location"),
            Some(target),
            "navigation replaced the focused address control"
        );
        // Continue editing without another click; Enter must retain the same field's focus.
        host_select_all(&mut runtime, &mut tick);
        host_type(&mut runtime, &folder.0.to_string_lossy(), &mut tick);
        host_key(
            &mut runtime,
            LogicalKey::Named(NamedKey::Enter),
            None,
            Modifiers::empty(),
            &mut tick,
        );
        wait_loaded(&browser);
        assert_eq!(browser.snapshot().directory, folder.0);
    }
}

#[test]
fn listing_activation_and_targeted_selection_shortcut_update_external_model() {
    let folder = Folder::new();
    let explorer = folder.explorer(None, false);
    let browser = explorer.browser().clone();
    let mut runtime = ViewRuntime::from_composed(explorer).unwrap();
    press(&mut runtime, "alpha.txt file");
    assert_eq!(
        browser.snapshot().selected,
        vec![folder.0.join("alpha.txt")]
    );
    let target = button(&runtime, "alpha.txt file");
    key(
        &mut runtime,
        target,
        LogicalKey::Character(KeyText::new("a").unwrap()),
        None,
        Modifiers::CONTROL,
    );
    assert_eq!(browser.snapshot().selected.len(), 4);
    assert!(
        !browser
            .snapshot()
            .selected
            .contains(&folder.0.join(".hidden"))
    );
    runtime.process_external_updates();
    assert!(runtime.ui().texts.iter().any(|(_, text)| {
        runtime
            .ui()
            .string(text.content)
            .is_some_and(|value| value.starts_with("4 items") && value.contains("4 selected"))
    }));
}

#[test]
fn keyboard_selection_starts_at_first_item_and_preserves_shift_range_anchor() {
    let folder = Folder::new();
    let explorer = folder.explorer(None, false);
    let browser = explorer.browser().clone();
    let mut runtime = ViewRuntime::from_composed(explorer).unwrap();
    let target = button(&runtime, "alpha.txt file");
    key(
        &mut runtime,
        target,
        LogicalKey::Named(NamedKey::ArrowDown),
        None,
        Modifiers::empty(),
    );
    assert_eq!(browser.snapshot().selected, vec![folder.0.join("folder")]);
    press(&mut runtime, "alpha.txt file");
    for _ in 0..2 {
        key(
            &mut runtime,
            target,
            LogicalKey::Named(NamedKey::ArrowDown),
            None,
            Modifiers::SHIFT,
        );
    }
    assert_eq!(
        browser.snapshot().selected,
        vec![
            folder.0.join("alpha.txt"),
            folder.0.join("beta.png"),
            folder.0.join("gamma.txt"),
        ]
    );
    // The SDK also emits normal button activation after targeted Enter input.
    // Keyboard activation must not replace the cursor selection with this old focus.
    assert!(runtime.dispatch_activation(target, ChangeSource::Keyboard));
    assert_eq!(browser.snapshot().selected.len(), 3);
}

#[test]
fn secondary_pointer_context_routes_properties_and_new_tab_and_space_only_selects() {
    use telorgon::input::PointerButton;
    let folder = Folder::new();
    let explorer = folder.explorer(None, false);
    let browser = explorer.browser().clone();
    let location = explorer.location.clone();
    let mut runtime = ViewRuntime::from_composed(explorer).unwrap();
    let target = button(&runtime, "folder folder");
    for expected in [vec![folder.0.join("folder")], vec![]] {
        key(
            &mut runtime,
            target,
            LogicalKey::Named(NamedKey::Space),
            None,
            Modifiers::empty(),
        );
        // A real Space release also triggers the button's keyboard activation.
        assert!(runtime.dispatch_activation(target, ChangeSource::Keyboard));
        assert_eq!(browser.snapshot().selected, expected);
        assert_eq!(
            browser.snapshot().directory,
            folder.0,
            "Space opened the selected folder"
        );
    }
    runtime.dispatch_ui(
        target,
        UiEventKind::Input(telorgon::InputEvent::mouse_button(
            PointerButton::SECONDARY,
            ButtonState::Released,
        )),
        u16::MAX,
        3,
    );
    assert_eq!(browser.snapshot().selected, vec![folder.0.join("folder")]);
    assert!(named_button(runtime.ui(), "Open in new tab").is_some());
    let close = button(&runtime, "Close");
    let context_actions = runtime.ui().nodes.core(close).unwrap().parent;
    // Dispatch the context-owned Properties entry rather than the toolbar control.
    let properties = runtime
        .ui()
        .semantics
        .iter()
        .find_map(|(node, semantics)| {
            (semantics.role == SemanticRole::Button
                && descendant_text(runtime.ui(), node) == "Properties"
                && runtime.ui().nodes.core(node).unwrap().parent == context_actions)
                .then_some(node)
        })
        .unwrap();
    assert!(runtime.dispatch_activation(properties, ChangeSource::Programmatic));
    assert!(named_button(runtime.ui(), "Close").is_none());
    let deadline = Instant::now() + Duration::from_secs(5);
    while browser.snapshot().preview.is_none() {
        assert!(
            Instant::now() < deadline,
            "Properties did not publish its preview"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        browser.snapshot().preview.unwrap().path,
        folder.0.join("folder")
    );
    runtime.process_external_updates();
    let target = button(&runtime, "folder folder");
    runtime.dispatch_ui(
        target,
        UiEventKind::Input(telorgon::InputEvent::mouse_button(
            PointerButton::SECONDARY,
            ButtonState::Released,
        )),
        u16::MAX,
        4,
    );
    press(&mut runtime, "Open in new tab");
    assert_eq!(location.text(), folder.0.join("folder").to_string_lossy());
    assert_eq!(
        browser.snapshot().directory,
        folder.0,
        "new-tab action navigated the old tab"
    );
    assert!(named_button(runtime.ui(), "Close").is_none());
    assert!(
        runtime
            .ui()
            .interactions
            .get(button(&runtime, "Close tab"))
            .unwrap()
            .enabled
    );
}

#[test]
fn open_picker_returns_selected_uri_through_mounted_accept_callback() {
    let folder = Folder::new();
    let explorer = folder.explorer(Some(PickerRequest::default()), false);
    let completion = explorer.completion.clone();
    let mut runtime = ViewRuntime::from_composed(explorer).unwrap();
    press(&mut runtime, "alpha.txt file");
    runtime.process_external_updates();
    press(&mut runtime, "Open");
    let response = completion
        .lock()
        .unwrap()
        .clone()
        .expect("picker did not complete");
    assert!(!response.cancelled);
    assert_eq!(
        response.uris,
        vec![crate::protocol::path_to_uri(&folder.0.join("alpha.txt")).unwrap()]
    );
}

#[test]
fn save_picker_requires_confirmation_without_modifying_existing_file() {
    let folder = Folder::new();
    let explorer = folder.explorer(
        Some(PickerRequest {
            mode: PickerMode::Save,
            current_name: Some("alpha.txt".into()),
            ..Default::default()
        }),
        false,
    );
    let completion = explorer.completion.clone();
    let name = explorer.picker_name.clone();
    let mut runtime = ViewRuntime::from_composed(explorer).unwrap();
    press(&mut runtime, "Save");
    assert!(named_button(runtime.ui(), "Replace").is_some());
    assert!(completion.lock().unwrap().is_none());
    assert_eq!(
        fs::read_to_string(folder.0.join("alpha.txt")).unwrap(),
        "First document\n"
    );
    // A destination may change while confirmation is visible.
    // Confirming alpha.txt must never authorize replacing this new destination.
    name.set("gamma.txt");
    runtime.process_external_updates();
    press(&mut runtime, "Replace");
    assert!(completion.lock().unwrap().is_none());
    assert_eq!(
        fs::read_to_string(folder.0.join("gamma.txt")).unwrap(),
        "Third document\n"
    );
    press(&mut runtime, "Save");
    press(&mut runtime, "Replace");
    assert_eq!(
        completion.lock().unwrap().as_ref().unwrap().uris,
        vec![crate::protocol::path_to_uri(&folder.0.join("gamma.txt")).unwrap()]
    );
    #[cfg(unix)]
    {
        let dangling = folder.0.join("dangling.txt");
        std::os::unix::fs::symlink(folder.0.join("missing-target.txt"), &dangling).unwrap();
        assert!(!dangling.exists());
        *completion.lock().unwrap() = None;
        name.set("dangling.txt");
        runtime.process_external_updates();
        press(&mut runtime, "Save");
        assert!(
            named_button(runtime.ui(), "Replace").is_some(),
            "existing dangling symlink skipped confirmation"
        );
        assert!(completion.lock().unwrap().is_none());
        assert!(fs::symlink_metadata(dangling).unwrap().is_symlink());
        assert!(!folder.0.join("missing-target.txt").exists());
    }
}

#[test]
fn software_renderer_draws_real_explorer_without_opening_a_window() {
    use telorgon::graphics::{
        render::{
            ReadbackFormat, ReadbackRequest, RenderBackend, RenderRequest, RenderTargetInfo,
            TargetLoad, TargetStore,
        },
        renderers::software::{SoftwareRenderer, SoftwareScene, SoftwareSurface, SoftwareTarget},
    };
    let folder = Folder::new();
    fs::create_dir(folder.0.join("Projects")).unwrap();
    fs::create_dir(folder.0.join("Reports")).unwrap();
    fs::write(folder.0.join("Quarterly budget.xlsx"), "Preview fixture").unwrap();
    fs::write(folder.0.join("Meeting notes.docx"), "Preview fixture").unwrap();
    fs::write(folder.0.join("Project overview.pdf"), "Preview fixture").unwrap();
    let explorer = folder.explorer(None, false);
    explorer
        .browser()
        .select_initial(folder.0.join("alpha.txt"));
    let extent = SizeI {
        width: 1240,
        height: 800,
    };
    for (name, request, grid) in [
        ("explorer", None, false),
        (
            "picker",
            Some(PickerRequest {
                mode: PickerMode::Save,
                title: "Save As".into(),
                current_name: Some("Report.txt".into()),
                ..Default::default()
            }),
            false,
        ),
        ("grid", None, true),
    ] {
        let explorer = folder.explorer(request, grid);
        explorer
            .browser()
            .select_initial(folder.0.join("alpha.txt"));
        let mut runtime =
            telorgon::ComposedAppRuntime::from_composed_with_extent(explorer, extent).unwrap();
        runtime.register_assets(crate::assets::bundle()).unwrap();
        runtime
            .prepare_frame(telorgon::MonotonicInstant::ZERO, true)
            .unwrap();
        let renderer = SoftwareRenderer;
        let mut scene = SoftwareScene::default();
        let mut surface = SoftwareSurface::default();
        while let Some(delta) = runtime.pop_scene_delta() {
            renderer.apply_scene_delta(&mut scene, &delta).unwrap();
        }
        let target = SoftwareTarget::new(RenderTargetInfo::full(extent));
        let background = scene.background();
        {
            let mut frame = surface.begin_frame();
            renderer
                .render(
                    &mut scene,
                    &mut frame,
                    &target,
                    &RenderRequest {
                        force: true,
                        load: TargetLoad::Clear(background),
                        store: TargetStore::Store,
                        region: None,
                    },
                )
                .unwrap();
        }
        let readback = surface
            .readback(&ReadbackRequest {
                region: telorgon::RectI {
                    x: 0,
                    y: 0,
                    width: extent.width,
                    height: extent.height,
                },
                format: ReadbackFormat::Rgba8,
            })
            .unwrap();
        assert_eq!(readback.pixels.len(), 1240 * 800 * 4);
        assert!(
            readback
                .pixels
                .chunks_exact(4)
                .any(|pixel| pixel[..3] != [BG.r, BG.g, BG.b]),
            "render contained only a flat background"
        );
        assert!(
            runtime
                .ui()
                .nodes
                .alive()
                .iter()
                .any(|node| runtime.ui().kinds.get(*node) == Some(&NodeKind::Button))
        );
        // A reviewable screenshot from the exact app tree, without external assets.
        let mut ppm = format!("P6\n{} {}\n255\n", extent.width, extent.height).into_bytes();
        for pixel in readback.pixels.chunks_exact(4) {
            ppm.extend_from_slice(&pixel[..3]);
        }
        let path = if name == "explorer" {
            "/tmp/telorgon-file-explorer.ppm".into()
        } else {
            format!("/tmp/telorgon-file-{name}.ppm")
        };
        fs::write(path, ppm).unwrap();
    }
}

#[test]
fn non_utf8_filenames_with_identical_display_labels_keep_distinct_keys() {
    use std::os::unix::ffi::OsStringExt;
    let folder = Folder::new();
    for byte in [0xfe, 0xff] {
        let name = std::ffi::OsString::from_vec(vec![b'f', byte]);
        fs::write(folder.0.join(name), b"raw filename").unwrap();
    }
    let explorer = folder.explorer(None, false);
    let browser = explorer.browser().clone();
    let runtime = ViewRuntime::from_composed(explorer).unwrap();
    assert!(named_button(runtime.ui(), "alpha.txt file").is_some());
    assert_eq!(browser.snapshot().entries.len(), 7);
}

#[test]
fn shutdown_finishes_file_writes_after_browser_is_dropped() {
    let folder = Folder::new();
    let data = vec![0x5a; 2 * 1024 * 1024];
    let source = folder.0.join("payload.bin");
    fs::write(&source, &data).unwrap();
    let workers = crate::model::FileWorkers::default();
    let browser = Browser::new(folder.0.join("folder"), workers.clone());
    wait_loaded(&browser);
    browser.transfer(vec![source.clone()], TransferMode::Copy);
    drop(browser);
    workers.finish();
    assert_eq!(fs::read(folder.0.join("folder/payload.bin")).unwrap(), data);
    assert_eq!(fs::read(source).unwrap(), data);
}

#[path = "field_composition_tests.rs"]
mod field_composition_tests;
#[path = "field_tests.rs"]
mod field_tests;
#[path = "navigation_button_tests.rs"]
mod navigation_button_tests;
#[path = "path_bar_tests.rs"]
mod path_bar_tests;

#[path = "window_layout_tests.rs"]
mod window_layout_tests;
