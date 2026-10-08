use super::*;
use telorgon::ui::{Background, InteractionFlags};

fn history_explorer(folder: &Folder) -> Explorer {
    fs::create_dir(folder.0.join("folder/child")).unwrap();
    let mut explorer = folder.explorer(None, false);
    explorer.navigate(folder.0.join("folder"));
    wait_loaded(explorer.browser());
    explorer.navigate(folder.0.join("folder/child"));
    wait_loaded(explorer.browser());
    explorer
}

fn leave_and_settle(runtime: &mut telorgon::ComposedAppRuntime, tick: &mut u64) {
    host_input(
        runtime,
        [telorgon::InputEvent::mouse_moved(PointF {
            x: 1050.0,
            y: 600.0,
        })],
        tick,
    );
    *tick += 150;
    host_input(runtime, [], tick);
}

fn flags(runtime: &telorgon::ComposedAppRuntime, name: &str) -> InteractionFlags {
    let node = named_button(runtime.ui(), name).unwrap();
    runtime.ui().interactions.get(node).unwrap().flags
}

fn neutral(runtime: &telorgon::ComposedAppRuntime, name: &str, enabled: bool) {
    let node = named_button(runtime.ui(), name).unwrap();
    let flags = flags(runtime, name);
    for unwanted in [
        InteractionFlags::HOVERED,
        InteractionFlags::PRESSED,
        InteractionFlags::FOCUS_VISIBLE,
        InteractionFlags::SELECTED,
        InteractionFlags::HIGHLIGHTED,
    ] {
        assert!(!flags.contains(unwanted), "{name} retained {unwanted:?}");
    }
    assert_eq!(flags.contains(InteractionFlags::DISABLED), !enabled);
    let style = runtime.ui().box_styles.get(node).unwrap();
    assert_eq!(style.decoration.background, Background::Color(PANEL));
    assert_eq!(style.decoration.outline.width, 0.0);
    assert_eq!(style.opacity, if enabled { 1.0 } else { 0.45 });
}

#[test]
fn history_buttons_release_press_and_return_to_neutral_at_each_end() {
    let folder = Folder::new();
    let explorer = history_explorer(&folder);
    let browser = explorer.browser().clone();
    let mut runtime = host_explorer(explorer);
    let mut tick = 0;
    let back = named_button(runtime.ui(), "Back").unwrap();
    let forward = named_button(runtime.ui(), "Forward").unwrap();
    neutral(&runtime, "Forward", false);

    let bounds = runtime.layout().computed(back).unwrap().border_rect;
    host_input(
        &mut runtime,
        [
            telorgon::InputEvent::mouse_moved(PointF {
                x: bounds.x + bounds.width / 2.0,
                y: bounds.y + bounds.height / 2.0,
            }),
            telorgon::InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Pressed),
        ],
        &mut tick,
    );
    assert!(flags(&runtime, "Back").contains(InteractionFlags::PRESSED));
    tick += 150;
    host_input(&mut runtime, [], &mut tick);
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(back)
            .unwrap()
            .decoration
            .background,
        Background::Color(ColorRgba8::rgba(61, 61, 61, 255))
    );
    assert_eq!(browser.snapshot().position, 2);
    host_input(
        &mut runtime,
        [telorgon::InputEvent::mouse_button(
            PointerButton::PRIMARY,
            ButtonState::Released,
        )],
        &mut tick,
    );
    wait_loaded(&browser);
    leave_and_settle(&mut runtime, &mut tick);
    assert_eq!(browser.snapshot().directory, folder.0.join("folder"));
    assert!(flags(&runtime, "Back").contains(InteractionFlags::FOCUSED));
    neutral(&runtime, "Back", true);
    neutral(&runtime, "Forward", true);

    host_click(&mut runtime, "Back", &mut tick);
    wait_loaded(&browser);
    leave_and_settle(&mut runtime, &mut tick);
    assert_eq!(browser.snapshot().directory, folder.0);
    neutral(&runtime, "Back", false);
    assert!(!flags(&runtime, "Back").contains(InteractionFlags::FOCUSED));

    host_click(&mut runtime, "Forward", &mut tick);
    wait_loaded(&browser);
    leave_and_settle(&mut runtime, &mut tick);
    assert_eq!(browser.snapshot().directory, folder.0.join("folder"));
    neutral(&runtime, "Forward", true);
    host_click(&mut runtime, "Forward", &mut tick);
    wait_loaded(&browser);
    leave_and_settle(&mut runtime, &mut tick);
    assert_eq!(browser.snapshot().directory, folder.0.join("folder/child"));
    neutral(&runtime, "Forward", false);
    assert!(!flags(&runtime, "Forward").contains(InteractionFlags::FOCUSED));
    assert_eq!(named_button(runtime.ui(), "Back"), Some(back));
    assert_eq!(named_button(runtime.ui(), "Forward"), Some(forward));
}

#[test]
fn new_tabs_keep_button_identity_and_do_not_focus_forward() {
    let folder = Folder::new();
    let explorer = history_explorer(&folder);
    let mut runtime = host_explorer(explorer);
    let mut tick = 0;
    let new_tab = named_button(runtime.ui(), "New tab").unwrap();
    let forward = named_button(runtime.ui(), "Forward").unwrap();
    for _ in 0..3 {
        assert_eq!(host_click(&mut runtime, "New tab", &mut tick), new_tab);
        leave_and_settle(&mut runtime, &mut tick);
        assert_eq!(named_button(runtime.ui(), "New tab"), Some(new_tab));
        assert_eq!(named_button(runtime.ui(), "Forward"), Some(forward));
        assert!(flags(&runtime, "New tab").contains(InteractionFlags::FOCUSED));
        for name in ["Back", "Forward"] {
            neutral(&runtime, name, false);
            assert!(!flags(&runtime, name).contains(InteractionFlags::FOCUSED));
        }
    }
}

#[test]
fn keyboard_navigation_retains_visible_focus_and_activates_history() {
    let folder = Folder::new();
    let explorer = history_explorer(&folder);
    let browser = explorer.browser().clone();
    let mut runtime = host_explorer(explorer);
    let mut tick = 0;
    host_click(&mut runtime, "Back", &mut tick);
    wait_loaded(&browser);
    leave_and_settle(&mut runtime, &mut tick);
    neutral(&runtime, "Back", true);
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::Tab),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    tick += 150;
    host_input(&mut runtime, [], &mut tick);
    let focus = flags(&runtime, "Forward");
    assert!(focus.contains(InteractionFlags::FOCUSED));
    assert!(focus.contains(InteractionFlags::FOCUS_VISIBLE));
    let node = named_button(runtime.ui(), "Forward").unwrap();
    let style = runtime.ui().box_styles.get(node).unwrap();
    assert_eq!(style.decoration.background, Background::Color(PANEL));
    assert_eq!(style.decoration.outline.width, 1.0);
    assert_eq!(style.decoration.outline.color, ACCENT);
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::Enter),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    wait_loaded(&browser);
    leave_and_settle(&mut runtime, &mut tick);
    assert_eq!(browser.snapshot().directory, folder.0.join("folder/child"));
    neutral(&runtime, "Forward", false);
}
