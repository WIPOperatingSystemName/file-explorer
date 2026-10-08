use super::*;
use telorgon::ui::text::{ResolvedTextStyle, TextEngine};

fn point(runtime: &telorgon::ComposedAppRuntime, name: &str, x: f32) -> PointF {
    let node = named_button(runtime.ui(), name).unwrap();
    let bounds = runtime.layout().computed(node).unwrap().content_rect;
    PointF {
        x: bounds.x + x,
        y: bounds.y + bounds.height / 2.0,
    }
}
fn click_at(runtime: &mut telorgon::ComposedAppRuntime, point: PointF, tick: &mut u64) {
    host_input(
        runtime,
        [
            telorgon::InputEvent::mouse_moved(point),
            telorgon::InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Pressed),
            telorgon::InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Released),
        ],
        tick,
    );
}
fn caret_x(text: &str, offset: usize) -> f32 {
    TextEngine::new()
        .unwrap()
        .line_layout(text, &ResolvedTextStyle::new(TEXT, 14))
        .unwrap()
        .caret_x(offset)
}
#[test]
fn pointer_positions_caret_shift_selects_and_drag_selects_text() {
    let folder = Folder::new();
    let explorer = folder.explorer(None, false);
    let value = explorer.search.clone();
    let mut runtime = host_explorer(explorer);
    let mut tick = 0;
    host_click(&mut runtime, "Search", &mut tick);
    value.set("alpha beta");
    runtime
        .prepare_frame(telorgon::MonotonicInstant::from_nanos(100_000_000), false)
        .unwrap();
    let position = point(&runtime, "Search", caret_x("alpha beta", 5));
    click_at(&mut runtime, position, &mut tick);
    assert_eq!(value.0.editor.lock().unwrap().cursor, 5);
    host_type(&mut runtime, "!", &mut tick);
    assert_eq!(value.text(), "alpha! beta");
    value.set("alpha beta");
    runtime
        .prepare_frame(telorgon::MonotonicInstant::from_nanos(101_000_000), false)
        .unwrap();
    let start = point(&runtime, "Search", caret_x("alpha beta", 0));
    let end = point(&runtime, "Search", caret_x("alpha beta", 5));
    host_input(
        &mut runtime,
        [
            telorgon::InputEvent::mouse_moved(start),
            telorgon::InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Pressed),
            telorgon::InputEvent::mouse_moved(end),
            telorgon::InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Released),
        ],
        &mut tick,
    );
    assert_eq!(
        value.0.editor.lock().unwrap().selected_text(),
        Some("alpha")
    );
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::ArrowRight),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    host_input(
        &mut runtime,
        [telorgon::InputEvent::ModifiersChanged(Modifiers::SHIFT)],
        &mut tick,
    );
    let end = point(&runtime, "Search", caret_x("alpha beta", 10));
    click_at(&mut runtime, end, &mut tick);
    assert_eq!(
        value.0.editor.lock().unwrap().selected_text(),
        Some(" beta")
    );
}
fn caret_bounds(runtime: &telorgon::ComposedAppRuntime) -> telorgon::RectF {
    let field = named_button(runtime.ui(), "Search").unwrap();
    let node = runtime
        .ui()
        .box_styles
        .iter()
        .find_map(|(node, style)| {
            (runtime.ui().is_descendant_or_self(node, field)
                && style.width == SizeRule::Logical(1.0)
                && style.height == SizeRule::Logical(16.0))
            .then_some(node)
        })
        .expect("focused editor has a caret");
    runtime.layout().computed(node).unwrap().border_rect
}
#[test]
fn long_input_keeps_caret_visible_and_unicode_edits_keep_graphemes_whole() {
    let folder = Folder::new();
    let explorer = folder.explorer(None, false);
    let value = explorer.search.clone();
    let mut runtime = host_explorer(explorer);
    let mut tick = 0;
    host_click(&mut runtime, "Search", &mut tick);
    value.set("a very long folder name / ".repeat(30));
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::End),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    let field = named_button(runtime.ui(), "Search").unwrap();
    let viewport = runtime.layout().computed(field).unwrap().content_rect;
    let caret = caret_bounds(&runtime);
    assert!(caret.x > viewport.x && caret.x + caret.width <= viewport.x + viewport.width + 0.1);
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::Home),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    let caret = caret_bounds(&runtime);
    assert!((caret.x - viewport.x).abs() < 0.1);
    value.set("e\u{301}👨‍👩‍👧‍👦");
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::Backspace),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    assert_eq!(value.text(), "e\u{301}");
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::Backspace),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    assert_eq!(value.text(), "");
}
#[test]
fn resetting_field_history_and_blurring_reject_late_pastes() {
    let value = FieldValue::with_keyboard("old", false, keyboard::Keyboard::default());
    value.focus(true, false);
    let pending = value.0.editor.lock().unwrap().target();
    value.focus(false, false);
    assert_eq!(
        value.0.editor.lock().unwrap().paste(pending, "late"),
        Err(telorgon::services::clipboard::ClipboardError::Stale)
    );
    value
        .0
        .editor
        .lock()
        .unwrap()
        .replace_selection(" edited")
        .unwrap();
    value.set("new\r\npath\tname\0");
    assert_eq!(value.text(), "new path name");
    assert!(!value.0.editor.lock().unwrap().undo(false));
}
