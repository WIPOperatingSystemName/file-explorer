use crate::ui::theme::{ACCENT, CARD, LINE, MUTED, PANEL};
use std::sync::{Arc, Mutex};
use telorgon::{
    app::*,
    input::{ButtonState, LogicalKey, Modifiers, NamedKey},
    services::clipboard::{ClipboardEditAction, ClipboardText},
    ui::{UiEvent, UiEventKind},
};

#[derive(Clone)]
pub(crate) struct FieldValue(pub(super) Arc<Value>);
pub(super) struct Value {
    pub(super) editor: Mutex<ClipboardText>,
    revision: Signal<()>,
    writer: SignalWriter<()>,
    keyboard: super::keyboard::Keyboard,
    submit: Mutex<Option<Arc<dyn Fn(String) + Send + Sync>>>,
    error: Mutex<Option<Arc<dyn Fn(String) + Send + Sync>>>,
    pub(super) presentation: Mutex<Presentation>,
    pub(super) layout: Mutex<super::field_layout::FieldLayout>,
}
#[derive(Default)]
pub(super) struct Presentation {
    pub(super) focused: bool,
    pub(super) editing: bool,
    pub(super) select_focus_pending: bool,
    pub(super) focus_requested: bool,
    pub(super) composition: Option<super::field_composition::Composition>,
}
impl PartialEq for FieldValue {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl FieldValue {
    pub fn set(&self, text: impl Into<String>) {
        self.0
            .editor
            .lock()
            .unwrap()
            .reset(super::clipboard::single_line(&text.into()));
        self.0.layout.lock().unwrap().reset();
        self.changed();
    }
    pub fn with_keyboard(
        text: impl Into<String>,
        secure: bool,
        keyboard: super::keyboard::Keyboard,
    ) -> Self {
        let (revision, writer) = Signal::new(());
        let mut editor = ClipboardText::default();
        editor.secure = secure;
        editor.reset(super::clipboard::single_line(&text.into()));
        Self(Arc::new(Value {
            editor: Mutex::new(editor),
            revision,
            writer,
            keyboard,
            submit: Mutex::new(None),
            error: Mutex::new(None),
            presentation: Mutex::new(Presentation::default()),
            layout: Mutex::new(Default::default()),
        }))
    }
    pub fn watch<C: Component>(&self, owner: &C) {
        owner.watch(&self.0.revision);
    }
    pub fn text(&self) -> String {
        self.0.editor.lock().unwrap().text.clone()
    }
    pub fn on_submit(&self, done: impl Fn(String) + Send + Sync + 'static) {
        *self.0.submit.lock().unwrap() = Some(Arc::new(done));
    }
    pub fn on_error(&self, done: impl Fn(String) + Send + Sync + 'static) {
        *self.0.error.lock().unwrap() = Some(Arc::new(done));
    }
    pub(super) fn error(&self, error: impl ToString) {
        let done = self.0.error.lock().unwrap().clone();
        if let Some(done) = done {
            done(error.to_string());
        }
    }
    pub(super) fn changed(&self) {
        self.0.writer.publish(());
    }
    pub(super) fn focus(&self, focused: bool, select_on_focus: bool) -> bool {
        let mut presentation = self.0.presentation.lock().unwrap();
        presentation.focus_requested = false;
        if presentation.focused == focused {
            return false;
        }
        presentation.focused = focused;
        presentation.editing = focused;
        if !focused {
            presentation.composition = None;
        }
        presentation.select_focus_pending = focused && select_on_focus;
        drop(presentation);
        if !focused {
            self.0.layout.lock().unwrap().blur();
        }
        let mut editor = self.0.editor.lock().unwrap();
        editor.invalidate_paste();
        if focused && select_on_focus {
            editor.select_all();
        }
        drop(editor);
        self.changed();
        true
    }
    pub(super) fn blur(&self) {
        self.focus(false, false);
    }
    pub(super) fn finish_editing(&self) {
        self.0.presentation.lock().unwrap().editing = false;
        let mut editor = self.0.editor.lock().unwrap();
        editor.anchor = editor.cursor;
        editor.invalidate_paste();
        drop(editor);
        self.changed();
    }
    fn begin_editing(&self, select_all: bool) -> bool {
        let mut presentation = self.0.presentation.lock().unwrap();
        if presentation.editing {
            return false;
        }
        presentation.editing = true;
        presentation.select_focus_pending = select_all;
        drop(presentation);
        if select_all {
            self.0.editor.lock().unwrap().select_all();
        }
        self.changed();
        true
    }
    pub(super) fn request_focus(&self) {
        let mut presentation = self.0.presentation.lock().unwrap();
        presentation.focus_requested = true;
        presentation.editing = true;
        drop(presentation);
        self.0.editor.lock().unwrap().select_all();
        self.changed();
    }
    fn key(&self, key: &telorgon::input::KeyEvent) -> bool {
        let mut key = self.0.keyboard.update(key);
        if let Some(text) = &key.text {
            let text = super::clipboard::single_line(text.as_str());
            key.text = (!text.is_empty())
                .then(|| telorgon::input::KeyText::new(&text).expect("normalized key text fits"));
        }
        if is_submit(&key) {
            let submit = self.0.submit.lock().unwrap().clone();
            if let Some(submit) = submit {
                submit(self.text());
                return true;
            }
            return false;
        }
        self.0.presentation.lock().unwrap().select_focus_pending = false;
        let mut editor = self.0.editor.lock().unwrap();
        let action = editor.key(&key);
        let changed = action == ClipboardEditAction::Changed;
        match action {
            ClipboardEditAction::Copy | ClipboardEditAction::Cut => {
                if let Some(text) = editor.selected_text().map(str::to_owned) {
                    let target = editor.target();
                    let value = self.clone();
                    crate::ui::clipboard::copy(text, move |result| match result {
                        Ok(()) if action == ClipboardEditAction::Cut => {
                            let result = value.0.editor.lock().unwrap().paste(target, "");
                            match result {
                                Ok(()) => value.changed(),
                                Err(telorgon::services::clipboard::ClipboardError::Stale) => {}
                                Err(error) => value.error(error),
                            }
                        }
                        Err(error) => value.error(error),
                        _ => {}
                    });
                }
            }
            ClipboardEditAction::Paste if !editor.read_only => {
                let target = editor.target();
                let value = self.clone();
                crate::ui::clipboard::read(move |result| match result {
                    Ok(text) => {
                        let result = value
                            .0
                            .editor
                            .lock()
                            .unwrap()
                            .paste(target, &super::clipboard::single_line(&text));
                        match result {
                            Ok(()) => value.changed(),
                            Err(telorgon::services::clipboard::ClipboardError::Stale) => {}
                            Err(error) => value.error(error),
                        }
                    }
                    Err(error) => value.error(error),
                });
            }
            _ => {}
        }
        drop(editor);
        if changed {
            self.changed();
        }
        changed
    }
}
fn is_submit(key: &telorgon::input::KeyEvent) -> bool {
    key.state == ButtonState::Pressed
        && !key.repeat
        && key.logical_key == LogicalKey::Named(NamedKey::Enter)
        && !key.modifiers.intersects(
            Modifiers::CONTROL
                .union(Modifiers::ALT)
                .union(Modifiers::SUPER),
        )
}

type OwnerAction = Box<dyn Fn(&mut super::Explorer)>;
/// A view builder keeps callbacks bound to the owning Explorer while the editor persists in FieldValue.
pub(crate) struct Field {
    label: String,
    placeholder: String,
    value: FieldValue,
    enabled: bool,
    compact: bool,
    borderless: bool,
    select_on_focus: bool,
    autofocus: bool,
    display_text: Option<String>,
    leading_icon: Option<String>,
    submit: Option<OwnerAction>,
    escape: Option<OwnerAction>,
}
impl Field {
    pub fn new(label: &str, placeholder: &str, value: FieldValue, enabled: bool) -> Self {
        Self {
            label: label.into(),
            placeholder: placeholder.into(),
            value,
            enabled,
            compact: false,
            borderless: false,
            select_on_focus: false,
            autofocus: false,
            display_text: None,
            leading_icon: None,
            submit: None,
            escape: None,
        }
    }
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
    pub fn borderless(mut self) -> Self {
        self.borderless = true;
        self
    }
    pub fn select_on_focus(mut self) -> Self {
        self.select_on_focus = true;
        self
    }
    pub fn autofocus(mut self) -> Self {
        self.autofocus = true;
        self
    }
    pub fn display_text(mut self, text: String) -> Self {
        self.display_text = Some(text);
        self
    }
    pub fn leading_icon(mut self, icon: &str) -> Self {
        self.leading_icon = Some(icon.into());
        self
    }
    pub fn on_submit(mut self, done: impl Fn(&mut super::Explorer) + 'static) -> Self {
        self.submit = Some(Box::new(done));
        self
    }
    pub fn on_escape(mut self, done: impl Fn(&mut super::Explorer) + 'static) -> Self {
        self.escape = Some(Box::new(done));
        self
    }
    fn input(&self, owner: &mut super::Explorer, event: &UiEvent) -> bool {
        if !self.enabled && matches!(event.kind, UiEventKind::Input(_)) {
            return false;
        }
        owner.keyboard.set(event.modifiers);
        let icon_width = if self.leading_icon.is_some() {
            26.0
        } else {
            0.0
        };
        let geometry_changed = event
            .geometry
            .as_ref()
            .is_some_and(|geometry| self.value.geometry(geometry, icon_width));
        let changed = match &event.kind {
            UiEventKind::Focus(focused) => self.value.focus(*focused, self.select_on_focus),
            UiEventKind::Input(telorgon::InputEvent::Key(key)) => {
                if key.state == ButtonState::Pressed
                    && key.logical_key == LogicalKey::Named(NamedKey::Escape)
                {
                    if self.value.is_composing() {
                        return self
                            .value
                            .composition(&telorgon::input::TextInputEvent::Cancel);
                    }
                    if let Some(escape) = &self.escape {
                        escape(owner);
                        return true;
                    }
                    return owner.control_input(event);
                }
                if owner.control_input(event) {
                    return true;
                }
                if self.value.is_composing() {
                    return false;
                }
                if self.display_text.is_some() && key.state == ButtonState::Pressed {
                    self.value.begin_editing(self.select_on_focus);
                }
                if is_submit(key) {
                    if let Some(submit) = &self.submit {
                        submit(owner);
                        return true;
                    }
                }
                self.value.key(key)
            }
            UiEventKind::Input(telorgon::InputEvent::TextInput(input)) => {
                if self.display_text.is_some()
                    && matches!(
                        input,
                        telorgon::input::TextInputEvent::Preedit { .. }
                            | telorgon::input::TextInputEvent::Commit(_)
                    )
                {
                    self.value.begin_editing(self.select_on_focus);
                }
                self.value.composition(input)
            }
            UiEventKind::Layout(geometry) => self.value.geometry(geometry, icon_width),
            UiEventKind::Input(
                telorgon::InputEvent::PointerButton { .. }
                | telorgon::InputEvent::PointerMoved { .. },
            ) => {
                if self.display_text.is_some()
                    && matches!(
                        event.kind,
                        UiEventKind::Input(telorgon::InputEvent::PointerButton {
                            button: telorgon::input::PointerButton::PRIMARY,
                            state: ButtonState::Pressed,
                            ..
                        })
                    )
                {
                    self.value.begin_editing(self.select_on_focus);
                }
                self.value.pointer(event, icon_width)
            }
            _ => false,
        };
        changed || geometry_changed
    }
}
impl View for Field {
    fn into_element(self) -> Element {
        let presentation = self.value.0.presentation.lock().unwrap();
        let focused = presentation.focused && (self.display_text.is_none() || presentation.editing);
        drop(presentation);
        let editor = self.value.0.editor.lock().unwrap();
        let secure = editor.secure;
        let readonly = editor.read_only;
        let value = editor.text.clone();
        drop(editor);
        let content = self
            .value
            .content(&self.placeholder, self.display_text.as_deref());
        let body = row()
            .width(Dimension::FILL)
            .height(18.0)
            .gap(8.0)
            .align_items(Alignment::Center)
            .children(
                self.leading_icon
                    .as_ref()
                    .map(|icon| super::icon_view(icon, 18.0)),
            )
            .child(content);
        let focus_requested = self.value.0.presentation.lock().unwrap().focus_requested;
        let input = text_input()
            .key("text-editor")
            .accessible_label(&self.label)
            .value(value)
            .secure(secure)
            .read_only(readonly)
            .autofocus(self.autofocus || focus_requested)
            .ime_cursor_rect(self.value.ime_cursor_rect(if self.leading_icon.is_some() {
                26.0
            } else {
                0.0
            }))
            .cursor(CursorIcon::Text)
            .width(Dimension::FILL)
            .height(34.0)
            .enabled(self.enabled)
            .padding(8.0)
            .corner_radius(4.0)
            .background(if self.borderless {
                ColorRgba8::rgba(0, 0, 0, 0)
            } else if self.display_text.is_some() && !focused {
                PANEL
            } else {
                CARD
            })
            .uniform_border(
                if self.borderless && !focused {
                    0.0
                } else {
                    1.0
                },
                if focused { ACCENT } else { LINE },
            )
            .overflow(telorgon::ui::Overflow::Clip)
            .child(body);
        let outer = column()
            .key(format!("field-{}", self.label))
            .width(Dimension::FILL)
            .height(if self.compact { 34.0 } else { 56.0 })
            .gap(if self.compact { 0.0 } else { 4.0 })
            .children(
                (!self.compact).then(|| text(&self.label).size(12.0).color(MUTED).height(18.0)),
            );
        outer
            .child(
                input.on_input(move |owner: &mut super::Explorer, event| self.input(owner, event)),
            )
            .into_element()
    }
}
