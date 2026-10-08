use std::sync::{
    Arc,
    atomic::{AtomicU16, Ordering},
};
use telorgon::input::{KeyEvent, Modifiers};

/// Share modifier state for pointer selection, honoring native key snapshots.
#[derive(Clone, Default)]
pub(super) struct Keyboard(Arc<AtomicU16>);
impl PartialEq for Keyboard {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Keyboard {
    pub fn update(&self, key: &KeyEvent) -> KeyEvent {
        self.set(key.modifiers);
        key.clone()
    }
    pub fn set(&self, modifiers: Modifiers) {
        self.0.store(modifiers.bits(), Ordering::Relaxed);
    }
    pub fn reset(&self) {
        self.set(Modifiers::empty());
    }
    pub fn current(&self) -> Modifiers {
        Modifiers::from_bits(self.0.load(Ordering::Relaxed)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use telorgon::input::{ButtonState, LogicalKey, NamedKey, PhysicalKey};

    #[test]
    fn native_snapshots_clear_stale_bits_and_keep_other_side_of_modifier() {
        let keyboard = Keyboard::default();
        keyboard.set(Modifiers::ALT.union(Modifiers::SUPER));
        let shift = KeyEvent::new(PhysicalKey::UNIDENTIFIED, ButtonState::Pressed)
            .with_logical_key(LogicalKey::Named(NamedKey::Shift))
            .with_modifiers(Modifiers::SHIFT);
        keyboard.update(&shift);
        assert_eq!(keyboard.current(), Modifiers::SHIFT);

        // Releasing one Shift key leaves Shift active when its other side is held.
        let released = KeyEvent::new(PhysicalKey::UNIDENTIFIED, ButtonState::Released)
            .with_logical_key(LogicalKey::Named(NamedKey::Shift))
            .with_modifiers(Modifiers::SHIFT);
        keyboard.update(&released);
        assert_eq!(keyboard.current(), Modifiers::SHIFT);

        keyboard.reset();
        assert_eq!(keyboard.current(), Modifiers::empty());
        keyboard.set(Modifiers::ALT_GRAPH.union(Modifiers::META));
        assert_eq!(
            keyboard.current(),
            Modifiers::ALT_GRAPH.union(Modifiers::META)
        );
    }
}
