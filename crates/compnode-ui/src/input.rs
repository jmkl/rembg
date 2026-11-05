//! Backend-agnostic input events. The host translates its own events (egui,
//! winit, a test script) into these.
use compnode_core::Vec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Modifiers {
    pub const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
    };
    pub const SHIFT: Modifiers = Modifiers {
        shift: true,
        ctrl: false,
        alt: false,
    };
    pub const CTRL: Modifiers = Modifiers {
        shift: false,
        ctrl: true,
        alt: false,
    };
    pub const ALT: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: true,
    };
    pub const CTRL_SHIFT: Modifiers = Modifiers {
        shift: true,
        ctrl: true,
        alt: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Space,
    Period,
    A,
    C,
    D,
    F,
    H,
    I,
    M,
    R,
    V,
    X,
    Y,
    Z,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    PointerMove {
        pos: Vec2,
        mods: Modifiers,
    },
    PointerDown {
        pos: Vec2,
        button: PointerButton,
        mods: Modifiers,
    },
    PointerUp {
        pos: Vec2,
        button: PointerButton,
        mods: Modifiers,
    },
    /// `delta.y > 0` scrolls/zooms in.
    Scroll {
        pos: Vec2,
        delta: Vec2,
        mods: Modifiers,
    },
    /// A key press (auto-repeat counts as another press).
    Key {
        key: Key,
        mods: Modifiers,
    },
    /// Text typed while no host text field is active (currently unused by the editor).
    Text(String),
}

impl InputEvent {
    pub fn mv(pos: Vec2) -> InputEvent {
        InputEvent::PointerMove {
            pos,
            mods: Modifiers::NONE,
        }
    }
    pub fn down(pos: Vec2) -> InputEvent {
        InputEvent::PointerDown {
            pos,
            button: PointerButton::Primary,
            mods: Modifiers::NONE,
        }
    }
    pub fn up(pos: Vec2) -> InputEvent {
        InputEvent::PointerUp {
            pos,
            button: PointerButton::Primary,
            mods: Modifiers::NONE,
        }
    }
    pub fn key(key: Key) -> InputEvent {
        InputEvent::Key {
            key,
            mods: Modifiers::NONE,
        }
    }
    pub fn scroll(pos: Vec2, y: f32) -> InputEvent {
        InputEvent::Scroll {
            pos,
            delta: Vec2 { x: 0.0, y },
            mods: Modifiers::NONE,
        }
    }
    /// Same event with other modifiers (no effect on `Text`).
    pub fn with_mods(mut self, m: Modifiers) -> InputEvent {
        match &mut self {
            InputEvent::PointerMove { mods, .. }
            | InputEvent::PointerDown { mods, .. }
            | InputEvent::PointerUp { mods, .. }
            | InputEvent::Scroll { mods, .. }
            | InputEvent::Key { mods, .. } => *mods = m,
            InputEvent::Text(_) => {}
        }
        self
    }
    /// Same event with another button (pointer down/up only).
    pub fn with_button(mut self, b: PointerButton) -> InputEvent {
        match &mut self {
            InputEvent::PointerDown { button, .. } | InputEvent::PointerUp { button, .. } => {
                *button = b
            }
            _ => {}
        }
        self
    }
    pub fn pos(&self) -> Option<Vec2> {
        match self {
            InputEvent::PointerMove { pos, .. }
            | InputEvent::PointerDown { pos, .. }
            | InputEvent::PointerUp { pos, .. }
            | InputEvent::Scroll { pos, .. } => Some(*pos),
            _ => None,
        }
    }
    pub fn mods(&self) -> Modifiers {
        match self {
            InputEvent::PointerMove { mods, .. }
            | InputEvent::PointerDown { mods, .. }
            | InputEvent::PointerUp { mods, .. }
            | InputEvent::Scroll { mods, .. }
            | InputEvent::Key { mods, .. } => *mods,
            InputEvent::Text(_) => Modifiers::NONE,
        }
    }
    /// Same event with the pointer position replaced (used to map screen
    /// coordinates into graph space before handing events to widgets).
    pub fn with_pos(&self, p: Vec2) -> InputEvent {
        let mut e = self.clone();
        match &mut e {
            InputEvent::PointerMove { pos, .. }
            | InputEvent::PointerDown { pos, .. }
            | InputEvent::PointerUp { pos, .. }
            | InputEvent::Scroll { pos, .. } => *pos = p,
            _ => {}
        }
        e
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compnode_core::geom::vec2;

    #[test]
    fn builders_and_accessors() {
        let e = InputEvent::down(vec2(1.0, 2.0)).with_mods(Modifiers::SHIFT);
        assert_eq!(e.pos(), Some(vec2(1.0, 2.0)));
        assert!(e.mods().shift && !e.mods().ctrl);
        let e = e
            .with_pos(vec2(5.0, 6.0))
            .with_button(PointerButton::Middle);
        assert_eq!(e.pos(), Some(vec2(5.0, 6.0)));
        assert!(matches!(
            e,
            InputEvent::PointerDown {
                button: PointerButton::Middle,
                ..
            }
        ));
        assert_eq!(InputEvent::key(Key::A).pos(), None);
    }
}
