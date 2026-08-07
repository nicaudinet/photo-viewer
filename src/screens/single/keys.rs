//! The single view's keyboard.
//!
//! Short, because this screen is a flat sequence of images: there is no
//! selection to paint and nothing to move between but previous and next.

use iced::keyboard::{self, key::Named};

use super::message::SingleMsg;
use super::SingleState;

impl SingleState {
    /// What `event` means here, or `None` if it means nothing.
    ///
    /// This screen is a flat sequence, so only left and right mean anything;
    /// the wall's up and down have nothing to move between. Every binding it
    /// shares with the wall is spelled out again rather than shared, because
    /// the two screens are free to disagree about what a key does — `f` here
    /// favourites one photo, not a selection.
    pub(crate) fn key(&self, event: &keyboard::Event) -> Option<SingleMsg> {
        let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
            return None;
        };
        match key.as_ref() {
            keyboard::Key::Named(Named::ArrowRight) => Some(SingleMsg::Next),
            keyboard::Key::Named(Named::ArrowLeft) => Some(SingleMsg::Prev),
            keyboard::Key::Character("l") => Some(SingleMsg::Next),
            keyboard::Key::Character("h") => Some(SingleMsg::Prev),
            keyboard::Key::Character("f") => Some(SingleMsg::ToggleFavourite),
            keyboard::Key::Character("R") => Some(SingleMsg::RotateClockwise),
            keyboard::Key::Character("r") if modifiers.shift() => Some(SingleMsg::RotateClockwise),
            keyboard::Key::Character("r") => Some(SingleMsg::RotateAnticlockwise),
            _ => None,
        }
    }
}
