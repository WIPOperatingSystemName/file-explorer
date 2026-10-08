use super::field::FieldValue;
use super::theme::{MUTED, SELECTED, TEXT};
use std::cell::RefCell;
use telorgon::app::*;
use telorgon::input::Modifiers;
use telorgon::services::clipboard::ClipboardText;
use telorgon::ui::UiEvent;
use telorgon::{
    PointF, Transform2D,
    input::{ButtonState, PointerButton},
    ui::{
        SizeRule2D, UiEventKind, UiInputGeometry,
        text::{ResolvedTextStyle, TextBuffer, TextEngine, TextLineLayout, TextOffset},
    },
};

thread_local! {
    static TEXT_ENGINE: RefCell<Option<TextEngine>> = const { RefCell::new(None) };
}
#[derive(Default)]
pub(super) struct FieldLayout {
    text: String,
    line: Option<TextLineLayout>,
    pub scroll: f32,
    viewport: f32,
    origin: PointF,
    dragging: bool,
    last_click: Option<(std::time::Instant, f32)>,
}
impl FieldLayout {
    pub(super) fn reset(&mut self) {
        self.scroll = 0.0;
        self.dragging = false;
        self.last_click = None;
    }
    pub(super) fn blur(&mut self) {
        self.dragging = false;
    }
    fn line(&mut self, text: &str) -> Option<&TextLineLayout> {
        if self.line.is_none() || self.text != text {
            self.text = text.into();
            self.line = TEXT_ENGINE.with(|cache| {
                let mut cache = cache.borrow_mut();
                if cache.is_none() {
                    *cache = TextEngine::new().ok();
                }
                cache
                    .as_mut()?
                    .line_layout(text, &ResolvedTextStyle::new(TEXT, 14))
                    .ok()
            });
        }
        self.line.as_ref()
    }
    fn reveal(&mut self, text: &str, cursor: usize) {
        let Some(line) = self.line(text) else {
            return;
        };
        let x = line.caret_x(cursor);
        let width = line.width();
        if self.viewport > 0.0 {
            if x < self.scroll {
                self.scroll = x;
            }
            if x > self.scroll + self.viewport - 2.0 {
                self.scroll = x - (self.viewport - 2.0).max(0.0);
            }
            self.scroll = self
                .scroll
                .clamp(0.0, (width + 2.0 - self.viewport).max(0.0));
        }
    }
}
fn positioned(width: f32, height: f32, x: f32) -> BoxStyle {
    BoxStyle {
        width: SizeRule::Logical(width.max(0.0)),
        height: SizeRule::Logical(height),
        max_size: SizeRule2D {
            width: SizeRule::Logical(f32::MAX),
            height: SizeRule::Logical(f32::MAX),
        },
        transform: Transform2D {
            translation: PointF { x, y: 0.0 },
            ..Default::default()
        },
        ..Default::default()
    }
}
// Secure fields use bullets. Mapping keeps editor offsets UTF-8-safe while laying out the masked string.
fn rendered(editor: &ClipboardText) -> String {
    if editor.secure {
        "•".repeat(editor.text.chars().count())
    } else {
        editor.text.clone()
    }
}
fn render_offset(editor: &ClipboardText, offset: usize) -> usize {
    if editor.secure {
        editor.text[..offset].chars().count() * '•'.len_utf8()
    } else {
        offset
    }
}
fn editor_offset(editor: &ClipboardText, offset: usize) -> usize {
    if editor.secure {
        editor
            .text
            .char_indices()
            .nth(offset / '•'.len_utf8())
            .map_or(editor.text.len(), |(i, _)| i)
    } else {
        offset
    }
}
impl FieldValue {
    pub(super) fn geometry(&self, geometry: &UiInputGeometry, icon_width: f32) -> bool {
        let width = (geometry.content_rect.width - icon_width).max(0.0);
        let mut layout = self.0.layout.lock().unwrap();
        let origin = PointF {
            x: geometry.content_rect.x,
            y: geometry.content_rect.y,
        };
        if (layout.viewport - width).abs() < 0.01 && layout.origin == origin {
            return false;
        }
        layout.viewport = width;
        layout.origin = origin;
        drop(layout);
        self.changed();
        true
    }
    pub(super) fn pointer(&self, event: &UiEvent, icon_width: f32) -> bool {
        let Some(geometry) = &event.geometry else {
            return false;
        };
        let Some(position) = geometry.position else {
            return false;
        };
        let x = position.x - geometry.content_rect.x - icon_width;
        let mut editor = self.0.editor.lock().unwrap();
        let text = rendered(&editor);
        let mut layout = self.0.layout.lock().unwrap();
        let scroll = layout.scroll;
        let Some(line) = layout.line(&text) else {
            return false;
        };
        let offset = editor_offset(&editor, line.hit_test(x + scroll));
        match &event.kind {
            UiEventKind::Input(telorgon::InputEvent::PointerButton {
                button: PointerButton::PRIMARY,
                state: ButtonState::Pressed,
                ..
            }) => {
                let mut presentation = self.0.presentation.lock().unwrap();
                if presentation.select_focus_pending {
                    presentation.select_focus_pending = false;
                    layout.dragging = false;
                    return false;
                }
                drop(presentation);
                let double = layout.last_click.is_some_and(|(time, previous)| {
                    time.elapsed().as_millis() < 420 && (previous - x).abs() < 4.0
                });
                layout.last_click = Some((std::time::Instant::now(), x));
                if double {
                    let range =
                        TextBuffer::from_text(editor.text.clone())
                            .ok()
                            .and_then(|buffer| {
                                buffer
                                    .snapshot()
                                    .word_range_at(TextOffset(offset as u32))
                                    .ok()
                                    .flatten()
                            });
                    if let Some(range) = range {
                        editor.anchor = range.start.as_usize();
                        editor.cursor = range.end.as_usize();
                    } else {
                        editor.anchor = offset;
                        editor.cursor = offset;
                    }
                } else {
                    editor.cursor = offset;
                    if !event.modifiers.contains(Modifiers::SHIFT) {
                        editor.anchor = offset;
                    }
                }
                layout.dragging = true;
            }
            UiEventKind::Input(telorgon::InputEvent::PointerButton {
                button: PointerButton::PRIMARY,
                state: ButtonState::Released,
                ..
            }) => {
                layout.dragging = false;
                return false;
            }
            UiEventKind::Input(telorgon::InputEvent::PointerMoved { .. }) if layout.dragging => {
                editor.cursor = offset
            }
            _ => return false,
        }
        editor.invalidate_paste();
        layout.reveal(&rendered(&editor), render_offset(&editor, editor.cursor));
        drop(layout);
        drop(editor);
        self.changed();
        true
    }
    pub(super) fn ime_cursor_rect(&self, icon_width: f32) -> telorgon::RectF {
        let editor = self.0.editor.lock().unwrap();
        let (editor, _) = self.presented_editor(&editor);
        let mut layout = self.0.layout.lock().unwrap();
        let x = layout.line(&rendered(&editor)).map_or(0.0, |line| {
            line.caret_x(render_offset(&editor, editor.cursor))
        });
        telorgon::RectF {
            x: layout.origin.x
                + icon_width
                + (x - layout.scroll).clamp(0.0, layout.viewport.max(0.0)),
            y: layout.origin.y,
            width: 1.0,
            height: 16.0,
        }
    }
    pub(super) fn content(&self, placeholder: &str, display: Option<&str>) -> Element {
        let presentation = self.0.presentation.lock().unwrap();
        let focused = presentation.focused && (display.is_none() || presentation.editing);
        drop(presentation);
        let editor = self.0.editor.lock().unwrap();
        let (editor, preedit) = self.presented_editor(&editor);
        let mut layout = self.0.layout.lock().unwrap();
        let display_text = if !focused && display.is_some() {
            display.unwrap().to_owned()
        } else if editor.text.is_empty() {
            placeholder.into()
        } else {
            rendered(&editor)
        };
        if focused {
            layout.reveal(&rendered(&editor), render_offset(&editor, editor.cursor));
        } else {
            layout.scroll = 0.0;
        }
        let scroll = layout.scroll;
        let line = layout.line(&display_text).cloned();
        let width = line.as_ref().map_or(0.0, TextLineLayout::width).max(1.0) + 2.0;
        let mut content = stack().box_style(positioned(width, 18.0, -scroll));
        if focused && !editor.selection().is_empty() {
            if let Some(line) = &line {
                let selection = editor.selection();
                for segment in line.selection_segments(
                    render_offset(&editor, selection.start)..render_offset(&editor, selection.end),
                ) {
                    content = content.child(
                        column()
                            .box_style(positioned(segment.end - segment.start, 18.0, segment.start))
                            .background(SELECTED),
                    );
                }
            }
        }
        if let (Some(range), Some(line)) = (preedit, &line) {
            for segment in line.selection_segments(
                render_offset(&editor, range.start)..render_offset(&editor, range.end),
            ) {
                let mut underline = positioned(segment.end - segment.start, 1.0, segment.start);
                underline.transform.translation.y = 17.0;
                content = content.child(column().box_style(underline).background(TEXT));
            }
        }
        content = content.child(
            text(&display_text)
                .size(14.0)
                .font_family("sans-serif")
                .line_height(18.0)
                .color(
                    if editor.text.is_empty() && (focused || display.is_none()) {
                        MUTED
                    } else {
                        TEXT
                    },
                )
                .box_style(positioned(width, 18.0, 0.0)),
        );
        if focused {
            let x = if editor.text.is_empty() {
                0.0
            } else {
                line.as_ref().map_or(0.0, |line| {
                    line.caret_x(render_offset(&editor, editor.cursor))
                })
            };
            content = content.child(
                column()
                    .box_style(positioned(1.0, 16.0, x))
                    .background(TEXT),
            );
        }
        column()
            .width(Dimension::FILL)
            .height(18.0)
            .overflow(telorgon::ui::Overflow::Clip)
            .child(content)
            .into_element()
    }
}
