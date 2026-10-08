use super::*;
use telorgon::ui::{ComponentStyleId, ThemeDomainId};

impl Explorer {
    pub(super) fn dismiss_layer(&self, menu: bool) -> Button {
        // An outside-click target must not inherit button hover, press, or focus paint.
        let neutral = telorgon::theme::CompiledComponentStyle {
            id: ComponentStyleId::named(ThemeDomainId::APPLICATION, "button", "dismiss-overlay"),
            slots: Default::default(),
            variants: Default::default(),
            states: Default::default(),
            state_precedence: Vec::new(),
            relevant_states: Default::default(),
            transition: Default::default(),
            controlled_slots: Default::default(),
            controlled_font_families: Default::default(),
        };
        button()
            .width(Dimension::FILL)
            .height(Dimension::FILL)
            .accessible_label("Dismiss dialog")
            .cursor(CursorIcon::Default)
            .background(ColorRgba8::rgba(0, 0, 0, if menu { 0 } else { 48 }))
            .inline_style(Arc::new(neutral))
            .on_input(|this: &mut Self, event| this.control_input(event))
            .on_press(|this: &mut Self| this.dismiss_dialog())
    }
}
