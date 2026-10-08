use telorgon::app::*;

pub const BG: ColorRgba8 = ColorRgba8::rgba(25, 25, 25, 255);
pub const PANEL: ColorRgba8 = ColorRgba8::rgba(32, 32, 32, 255);
pub const CARD: ColorRgba8 = ColorRgba8::rgba(43, 43, 43, 255);
pub const TAB_BAR: ColorRgba8 = PANEL;
pub const COMMAND_BAR: ColorRgba8 = CARD;
pub const LINE: ColorRgba8 = ColorRgba8::rgba(58, 58, 58, 255);
pub const TEXT: ColorRgba8 = ColorRgba8::rgba(240, 240, 240, 255);
pub const MUTED: ColorRgba8 = ColorRgba8::rgba(164, 164, 164, 255);
pub const ACCENT: ColorRgba8 = ColorRgba8::rgba(96, 205, 255, 255);
pub const SELECTED: ColorRgba8 = ColorRgba8::rgba(51, 51, 51, 255);
pub const GREEN: ColorRgba8 = ColorRgba8::rgba(108, 203, 95, 255);
pub const RED: ColorRgba8 = ColorRgba8::rgba(255, 153, 164, 255);
const HOVER: ColorRgba8 = ColorRgba8::rgba(53, 53, 53, 255);
const PRESSED: ColorRgba8 = ColorRgba8::rgba(61, 61, 61, 255);

pub fn label(value: impl ToString, size: f32, color: ColorRgba8) -> Text {
    text(value).size(size).color(color)
}
pub fn control(value: impl ToString) -> Button {
    let value = value.to_string();
    base_control(&value)
        .width((value.chars().count() as f32 * 7.0 + 24.0).clamp(36.0, 240.0))
        .child(label(value, 13.0, TEXT))
}

pub fn primary(value: impl ToString) -> Button {
    let value = value.to_string();
    base_control(&value)
        .width((value.chars().count() as f32 * 7.0 + 28.0).clamp(76.0, 240.0))
        .background(ACCENT)
        .hover_effect(InteractionEffect::Background(ColorRgba8::rgba(
            86, 185, 230, 255,
        )))
        .press_effect(InteractionEffect::Background(ColorRgba8::rgba(
            77, 165, 204, 255,
        )))
        .center_content()
        .child(label(value, 13.0, BG))
}

/// A toolbar command with an icon and a short visible label.
pub fn command(label: &str, icon: &str) -> Button {
    let width = (label.chars().count() as f32 * 7.0 + 49.0).clamp(66.0, 240.0);
    base_control(label).width(width).child(
        row()
            .width(width - 16.0)
            .height(18.0)
            .gap(8.0)
            .align_items(Alignment::Center)
            .child(super::icons::icon_view(icon, 18.0))
            .child(self::label(label, 13.0, TEXT)),
    )
}

/// Compact commands retain a spoken name while drawing only the glyph.
pub fn icon_button(label: &str, icon: &str) -> Button {
    base_control(label)
        .width(36.0)
        .center_content()
        .child(super::icons::icon_view(icon, 18.0))
}

/// History controls use a neutral disabled fill and retain a keyboard focus cue.
pub fn navigation_button(label: &str, icon: &str) -> Button {
    use std::{collections::BTreeMap, sync::Arc};
    use telorgon::{
        theme::{CompiledComponentStyle, CompiledSlotStyle, CompiledStateStyle, InteractionState},
        ui::{
            Background, ComponentStyleId, InteractionFlags, Outline, StylePropertyPatch,
            StyleSlotId, ThemeDomainId,
        },
    };

    let root = StyleSlotId::named("root");
    let outline = Outline {
        width: 0.0,
        offset: 0.0,
        color: ACCENT,
    };
    let resting = StylePropertyPatch {
        background: Some(Background::Color(PANEL)),
        opacity: Some(1.0),
        outline: Some(outline),
        ..Default::default()
    };
    let slot = |patch| CompiledSlotStyle {
        patch,
        font_family: None,
    };
    let state = |patch| CompiledStateStyle {
        slots: BTreeMap::from([(root, slot(patch))]),
        transition: None,
    };
    let style = CompiledComponentStyle {
        id: ComponentStyleId::named(ThemeDomainId::APPLICATION, "button", "navigation"),
        slots: BTreeMap::from([(root, slot(resting))]),
        variants: Default::default(),
        states: BTreeMap::from([
            (
                InteractionState::FocusVisible,
                state(StylePropertyPatch {
                    outline: Some(Outline {
                        width: 1.0,
                        ..outline
                    }),
                    ..Default::default()
                }),
            ),
            (
                InteractionState::Disabled,
                state(StylePropertyPatch {
                    background: Some(Background::Color(PANEL)),
                    opacity: Some(0.45),
                    outline: Some(outline),
                    ..Default::default()
                }),
            ),
        ]),
        state_precedence: vec![InteractionState::FocusVisible, InteractionState::Disabled],
        relevant_states: InteractionFlags::from_bits(
            InteractionFlags::FOCUS_VISIBLE.bits() | InteractionFlags::DISABLED.bits(),
        ),
        transition: Default::default(),
        controlled_slots: BTreeMap::from([(root, resting)]),
        controlled_font_families: Default::default(),
    };
    icon_button(label, icon)
        .background(PANEL)
        .inline_style(Arc::new(style))
}

pub fn separator() -> Container {
    column().width(1.0).height(22.0).background(LINE)
}

fn base_control(label: &str) -> Button {
    button()
        .accessible_label(label)
        .height(34.0)
        .padding(8.0)
        .corner_radius(4.0)
        .background(ColorRgba8::rgba(0, 0, 0, 0))
        .hover_effect(InteractionEffect::Background(HOVER))
        .press_effect(InteractionEffect::Background(PRESSED))
        .on_input(|this: &mut super::Explorer, event| this.control_input(event))
}
pub fn content(height: f32) -> Container {
    column().box_style(BoxStyle {
        width: SizeRule::Fill(1.0),
        height: SizeRule::Logical(height),
        max_size: SizeRule2D {
            width: SizeRule::Fill(1.0),
            height: SizeRule::Logical(f32::MAX),
        },
        ..Default::default()
    })
}
pub fn size(bytes: u64) -> String {
    let mut value = bytes as f64;
    for unit in ["B", "KiB", "MiB", "GiB", "TiB"] {
        if value < 1024.0 || unit == "TiB" {
            return if unit == "B" {
                format!("{bytes} B")
            } else {
                format!("{value:.1} {unit}")
            };
        }
        value /= 1024.0;
    }
    unreachable!()
}
