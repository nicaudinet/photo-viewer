//! The wall's keyboard.
//!
//! Every key that means something on the wall is named here, and nowhere else.
//! The app hands over the raw event and takes back either a [`WallMsg`] — the
//! wall's own business — or one of the few things only the app can do.

use iced::keyboard::{self, key::Named};

use crate::core::library::RangeOp;
use crate::core::transfer::TransferKind;

use super::message::{Dir, WallMsg};
use super::WallState;

/// What a key press on the wall asks for.
///
/// Most of them the wall answers itself. The rest need the app: opening an
/// image means leaving the wall, and trashing or transferring a selection puts
/// a question on screen first. Naming them here keeps them out of the app's
/// vocabulary, where they would read as things any screen might ask for.
pub(crate) enum WallKey {
    /// The wall's own business.
    Msg(WallMsg),
    /// Leave the wall and open this index in the single view.
    Open(usize),
    /// Ask before trashing the selection.
    Trash,
    /// Aim a move or copy at a folder the user picks.
    Transfer(TransferKind),
}

impl WallState {
    /// What `event` means here, or `None` if it means nothing.
    ///
    /// Takes `&self` because two keys are read against the current mode: Enter
    /// commits a painted range if there is one and otherwise opens the image
    /// under the cursor.
    pub(crate) fn key(&self, event: &keyboard::Event) -> Option<WallKey> {
        let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
            return None;
        };

        let msg = match key.as_ref() {
            keyboard::Key::Named(Named::ArrowRight) => WallMsg::Nav(Dir::Right),
            keyboard::Key::Named(Named::ArrowLeft) => WallMsg::Nav(Dir::Left),
            keyboard::Key::Named(Named::ArrowUp) => WallMsg::Nav(Dir::Up),
            keyboard::Key::Named(Named::ArrowDown) => WallMsg::Nav(Dir::Down),
            keyboard::Key::Character("l") => WallMsg::Nav(Dir::Right),
            keyboard::Key::Character("h") => WallMsg::Nav(Dir::Left),
            keyboard::Key::Character("k") => WallMsg::Nav(Dir::Up),
            keyboard::Key::Character("j") => WallMsg::Nav(Dir::Down),

            // Enter is the one key whose meaning depends on the mode: a range
            // being painted is waiting to be committed, so it cannot also be
            // the key that leaves the wall.
            keyboard::Key::Named(Named::Enter) if self.is_visual() => WallMsg::CommitVisual,
            // A stack has no single photograph to open, so Enter goes into it
            // instead — a wall of its members, with this one underneath.
            keyboard::Key::Named(Named::Enter) if self.stack_at(self.library.paths.index()) => {
                WallMsg::Descend {
                    index: self.library.paths.index(),
                }
            }
            keyboard::Key::Named(Named::Enter) => {
                return Some(WallKey::Open(self.library.paths.index()))
            }

            keyboard::Key::Named(Named::Space) => WallMsg::ToggleCursor,
            keyboard::Key::Character("v") => WallMsg::EnterVisual { op: RangeOp::Add },
            keyboard::Key::Character("x") => WallMsg::EnterVisual {
                op: RangeOp::Remove,
            },
            keyboard::Key::Character("i") => WallMsg::InvertSelection,
            keyboard::Key::Character("a") if modifiers.command() => WallMsg::SelectAll,

            keyboard::Key::Character("d") => return Some(WallKey::Trash),
            keyboard::Key::Character("m") => return Some(WallKey::Transfer(TransferKind::Move)),
            keyboard::Key::Character("c") => return Some(WallKey::Transfer(TransferKind::Copy)),

            keyboard::Key::Character("F") => WallMsg::ToggleFilter,
            keyboard::Key::Character("f") if modifiers.shift() => WallMsg::ToggleFilter,
            keyboard::Key::Character("f") => WallMsg::ToggleFavourite,

            keyboard::Key::Character("g") => WallMsg::ToggleGrouping,
            // `=` as well as `+`, so loosening does not need the shift key held
            // — it is a dial the user turns while watching the wall.
            keyboard::Key::Character("+") => WallMsg::Retune { looser: true },
            keyboard::Key::Character("=") => WallMsg::Retune { looser: true },
            keyboard::Key::Character("-") => WallMsg::Retune { looser: false },

            keyboard::Key::Character("R") => WallMsg::Rotate { clockwise: true },
            keyboard::Key::Character("r") if modifiers.shift() => {
                WallMsg::Rotate { clockwise: true }
            }
            keyboard::Key::Character("r") => WallMsg::Rotate { clockwise: false },

            _ => return None,
        };
        Some(WallKey::Msg(msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::wall::fixture::*;

    /// A key press with no modifiers held.
    fn press_key(state: &WallState, key: keyboard::Key) -> Option<WallKey> {
        state.key(&keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            repeat: false,
            text: None,
        })
    }

    fn press(state: &WallState, c: &str) -> Option<WallKey> {
        press_key(state, keyboard::Key::Character(c.into()))
    }

    fn enter(state: &WallState) -> Option<WallKey> {
        press_key(state, keyboard::Key::Named(Named::Enter))
    }

    #[test]
    fn enter_opens_the_image_under_the_cursor() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(3);
        assert!(matches!(enter(&state), Some(WallKey::Open(3))));
    }

    #[test]
    fn enter_commits_the_range_instead_while_painting() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        assert!(matches!(
            enter(&state),
            Some(WallKey::Msg(WallMsg::CommitVisual))
        ));
    }

    #[test]
    fn the_keys_that_need_the_app_say_so() {
        let state = wall(&[200.0; 6], 1);
        assert!(matches!(press(&state, "d"), Some(WallKey::Trash)));
        assert!(matches!(
            press(&state, "m"),
            Some(WallKey::Transfer(TransferKind::Move))
        ));
        assert!(matches!(
            press(&state, "c"),
            Some(WallKey::Transfer(TransferKind::Copy))
        ));
    }

    #[test]
    fn g_toggles_grouping_and_the_dial_turns_both_ways() {
        let state = wall(&[200.0; 6], 1);
        assert!(matches!(
            press(&state, "g"),
            Some(WallKey::Msg(WallMsg::ToggleGrouping))
        ));
        for key in ["+", "="] {
            assert!(
                matches!(
                    press(&state, key),
                    Some(WallKey::Msg(WallMsg::Retune { looser: true }))
                ),
                "{key} should loosen"
            );
        }
        assert!(matches!(
            press(&state, "-"),
            Some(WallKey::Msg(WallMsg::Retune { looser: false }))
        ));
    }

    #[test]
    fn enter_goes_into_a_stack_instead_of_opening_it() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 1, 2, 40, 41, 42]);
        // A stack has no single photograph to open.
        assert!(matches!(
            enter(&state),
            Some(WallKey::Msg(WallMsg::Descend { index: 0 }))
        ));

        state.library.goto(1);
        descend(&mut state, 1);
        // And inside it, Enter opens photographs again.
        assert!(matches!(enter(&state), Some(WallKey::Open(0))));
    }

    #[test]
    fn an_unbound_key_means_nothing_here() {
        let state = wall(&[200.0; 6], 1);
        assert!(press(&state, "z").is_none());
    }
}
