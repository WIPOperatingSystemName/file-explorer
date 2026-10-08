use super::*;
use telorgon::input::TextInputEvent;
#[test]
fn ime_preedit_is_temporary_enter_waits_for_commit_and_commit_is_undoable_once() {
    let folder = Folder::new();
    let explorer = folder.explorer(None, false);
    let browser = explorer.browser().clone();
    let value = explorer.search.clone();
    let mut runtime = host_explorer(explorer);
    let mut tick = 0;
    host_click(&mut runtime, "Search", &mut tick);
    value.set("old");
    host_select_all(&mut runtime, &mut tick);
    host_input(
        &mut runtime,
        [telorgon::InputEvent::TextInput(TextInputEvent::Preedit {
            text: "にほん".into(),
            selection: Some((0, 9)),
        })],
        &mut tick,
    );
    assert_eq!(value.text(), "old");
    assert!(
        descendant_text(runtime.ui(), named_button(runtime.ui(), "Search").unwrap())
            .contains("にほん")
    );
    host_key(
        &mut runtime,
        LogicalKey::Named(NamedKey::Enter),
        None,
        Modifiers::empty(),
        &mut tick,
    );
    assert!(browser.snapshot().query.is_empty());
    host_input(
        &mut runtime,
        [
            telorgon::InputEvent::TextInput(TextInputEvent::Preedit {
                text: "".into(),
                selection: None,
            }),
            telorgon::InputEvent::TextInput(TextInputEvent::Commit("日本".into())),
        ],
        &mut tick,
    );
    assert_eq!(value.text(), "日本");
    host_key(
        &mut runtime,
        LogicalKey::Character(KeyText::new("z").unwrap()),
        None,
        Modifiers::CONTROL,
        &mut tick,
    );
    assert_eq!(value.text(), "old");
}
#[test]
fn ime_cancel_and_programmatic_reset_cannot_apply_old_composition_to_new_value() {
    let value = FieldValue::with_keyboard("old", false, keyboard::Keyboard::default());
    value.composition(&TextInputEvent::Preedit {
        text: "draft".into(),
        selection: None,
    });
    value.composition(&TextInputEvent::Cancel);
    assert_eq!(value.text(), "old");
    value.composition(&TextInputEvent::Preedit {
        text: "pending".into(),
        selection: None,
    });
    value.set("new path");
    value.composition(&TextInputEvent::Commit("late".into()));
    assert_eq!(value.text(), "new path");
}
