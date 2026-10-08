use super::*;
use telorgon::ui::{Background, InteractionFlags};

fn idle(runtime: &telorgon::ComposedAppRuntime) {
    let node = named_button(runtime.ui(), "Location").unwrap();
    let style = runtime.ui().box_styles.get(node).unwrap();
    assert_eq!(style.decoration.background, Background::Color(PANEL));
    assert_eq!(style.decoration.border.top.color, LINE);
    assert!(descendant_text(runtime.ui(), node).contains("File System"));
}
fn active(runtime: &telorgon::ComposedAppRuntime) {
    let node = named_button(runtime.ui(), "Location").unwrap();
    assert_eq!(
        runtime
            .ui()
            .box_styles
            .get(node)
            .unwrap()
            .decoration
            .border
            .top
            .color,
        ACCENT
    );
}
#[test]
fn path_bar_returns_to_idle_after_submit_escape_and_clicking_blank_content() {
    let folder = Folder::new();
    let explorer = folder.explorer(None, false);
    let browser = explorer.browser().clone();
    let location = explorer.location.clone();
    let mut runtime = host_explorer(explorer);
    let mut tick = 0;
    idle(&runtime);
    host_click(&mut runtime, "Location", &mut tick);
    active(&runtime);
    host_type(&mut runtime, "folder", &mut tick);
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::Enter),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    wait_loaded(&browser);
    host_input(&mut runtime, [], &mut tick);
    idle(&runtime);
    assert_eq!(browser.snapshot().directory, folder.0.join("folder"));
    host_click(&mut runtime, "Location", &mut tick);
    active(&runtime);
    host_type(&mut runtime, "missing", &mut tick);
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::Escape),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    idle(&runtime);
    assert_eq!(location.text(), folder.0.join("folder").to_string_lossy());
    host_key(
        &mut runtime,
        LogicalKey::Character(KeyText::new("l").unwrap()),
        None,
        Modifiers::CONTROL,
        &mut tick,
    );
    active(&runtime);
    host_input(
        &mut runtime,
        [
            telorgon::InputEvent::mouse_moved(PointF {
                x: 1050.0,
                y: 600.0,
            }),
            telorgon::InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Pressed),
            telorgon::InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Released),
        ],
        &mut tick,
    );
    idle(&runtime);
    let node = named_button(runtime.ui(), "Location").unwrap();
    assert!(
        !runtime
            .ui()
            .interactions
            .get(node)
            .unwrap()
            .flags
            .contains(InteractionFlags::FOCUSED)
    );
}
