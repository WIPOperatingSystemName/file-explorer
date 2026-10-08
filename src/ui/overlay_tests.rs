use super::*;
use telorgon::{
    ComposedAppRuntime, MonotonicInstant, PointF, SemanticName, SemanticRole, SizeI,
    input::{ButtonState, PointerButton},
    ui::{InteractionFlags, UiNodeId},
};

fn named_button(runtime: &ComposedAppRuntime, name: &str) -> Option<UiNodeId> {
    runtime.ui().semantics.iter().find_map(|(node, semantics)| {
        (semantics.role == SemanticRole::Button
            && matches!(semantics.name, SemanticName::Text(value) if runtime.ui().string(value) == Some(name)))
        .then_some(node)
    })
}

#[test]
fn dismissal_layer_does_not_highlight_on_hover_press_or_focus_and_outside_click_closes() {
    let directory = std::env::temp_dir();
    for (dialog, alpha) in [
        (Dialog::Sort, 0),
        (Dialog::More, 0),
        (Dialog::NewFolder, 48),
    ] {
        let mut explorer = Explorer::new(directory.clone(), None, Arc::new(Mutex::new(None)));
        explorer.dialog = dialog;
        let mut runtime = ComposedAppRuntime::from_composed_with_extent(
            explorer,
            SizeI {
                width: 1240,
                height: 800,
            },
        )
        .unwrap();
        runtime.register_assets(crate::assets::bundle()).unwrap();
        runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
        let target = named_button(&runtime, "Dismiss dialog").unwrap();
        for (index, flags) in [
            InteractionFlags::HOVERED,
            InteractionFlags::from_bits(
                InteractionFlags::HOVERED.bits() | InteractionFlags::PRESSED.bits(),
            ),
            InteractionFlags::from_bits(
                InteractionFlags::FOCUSED.bits() | InteractionFlags::FOCUS_VISIBLE.bits(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            for flag in [
                InteractionFlags::HOVERED,
                InteractionFlags::PRESSED,
                InteractionFlags::FOCUSED,
                InteractionFlags::FOCUS_VISIBLE,
            ] {
                runtime
                    .ui_mut()
                    .route_interaction_flag(target, flag, flags.contains(flag));
            }
            runtime
                .prepare_frame(
                    MonotonicInstant::from_nanos((index as u64 + 1) * 1_000_000_000),
                    true,
                )
                .unwrap();
            let style = runtime.ui().box_styles.get(target).unwrap();
            assert_eq!(
                style.decoration.background,
                telorgon::ui::Background::Color(ColorRgba8::rgba(0, 0, 0, alpha))
            );
            assert_eq!(
                style.decoration.outline.width, 0.0,
                "fullscreen target gained a focus outline"
            );
        }
        for event in [
            telorgon::InputEvent::mouse_moved(PointF {
                x: 1100.0,
                y: 700.0,
            }),
            telorgon::InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Pressed),
            telorgon::InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Released),
        ] {
            runtime.queue_input(event);
        }
        let time = MonotonicInstant::from_nanos(4_000_000_000);
        runtime.flush_input(time);
        runtime.prepare_frame(time, true).unwrap();
        assert!(
            named_button(&runtime, "Dismiss dialog").is_none(),
            "click outside did not close the popup"
        );
    }
}
