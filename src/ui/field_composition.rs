use super::field::FieldValue;
use std::ops::Range;
use telorgon::{
    input::TextInputEvent,
    services::clipboard::{ClipboardError, ClipboardText, PasteTarget},
};

pub(super) struct Composition {
    pub text: String,
    pub selection: Range<usize>,
    pub range: Range<usize>,
    target: PasteTarget,
}
fn boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}
impl FieldValue {
    pub(super) fn is_composing(&self) -> bool {
        self.0.presentation.lock().unwrap().composition.is_some()
    }
    pub(super) fn composition(&self, event: &TextInputEvent) -> bool {
        match event {
            TextInputEvent::Preedit { text, selection } => {
                let editor = self.0.editor.lock().unwrap();
                if editor.read_only {
                    return false;
                }
                let mut presentation = self.0.presentation.lock().unwrap();
                let composition = presentation.composition.get_or_insert_with(|| Composition {
                    text: String::new(),
                    selection: 0..0,
                    range: editor.selection(),
                    target: editor.target(),
                });
                let selection = selection.map(|(start, end)| {
                    let start = boundary(text, start);
                    let end = boundary(text, end);
                    super::clipboard::single_line(&text[..start]).len()
                        ..super::clipboard::single_line(&text[..end]).len()
                });
                composition.text = super::clipboard::single_line(text);
                composition.selection =
                    selection.unwrap_or(composition.text.len()..composition.text.len());
                drop(presentation);
                drop(editor);
                self.changed();
                true
            }
            TextInputEvent::Commit(text) => {
                let composition = self.0.presentation.lock().unwrap().composition.take();
                let mut editor = self.0.editor.lock().unwrap();
                let text = super::clipboard::single_line(text);
                let result = if let Some(composition) = composition {
                    editor.paste(composition.target, &text)
                } else {
                    editor.replace_selection(&text)
                };
                drop(editor);
                if let Err(error) = result {
                    if error != ClipboardError::Stale {
                        self.error(error);
                    }
                }
                self.changed();
                true
            }
            TextInputEvent::Cancel => {
                let changed = self
                    .0
                    .presentation
                    .lock()
                    .unwrap()
                    .composition
                    .take()
                    .is_some();
                if changed {
                    self.changed();
                }
                changed
            }
        }
    }
    pub(super) fn presented_editor(
        &self,
        editor: &ClipboardText,
    ) -> (ClipboardText, Option<Range<usize>>) {
        let presentation = self.0.presentation.lock().unwrap();
        let Some(composition) = &presentation.composition else {
            let mut shown = ClipboardText::default();
            shown.text = editor.text.clone();
            shown.cursor = editor.cursor;
            shown.anchor = editor.anchor;
            shown.secure = editor.secure;
            return (shown, None);
        };
        let range = composition.range.clone();
        if editor.target() != composition.target || editor.text.get(range.clone()).is_none() {
            let mut shown = ClipboardText::default();
            shown.text = editor.text.clone();
            shown.cursor = editor.cursor;
            shown.anchor = editor.anchor;
            shown.secure = editor.secure;
            return (shown, None);
        }
        let mut shown = ClipboardText::default();
        shown.text = format!(
            "{}{}{}",
            &editor.text[..range.start],
            composition.text,
            &editor.text[range.end..]
        );
        shown.anchor = range.start + composition.selection.start;
        shown.cursor = range.start + composition.selection.end;
        shown.secure = editor.secure;
        (
            shown,
            Some(range.start..range.start + composition.text.len()),
        )
    }
}
