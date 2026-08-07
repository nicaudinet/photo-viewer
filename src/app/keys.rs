//! The keyboard, and the rest of what the app listens to.
//!
//! Two keymaps, one live at a time. A question on screen names its own answers,
//! and those can be any letter, so the ordinary bindings would fight them (`o`
//! opens a folder, `r` rotates). Swapping the whole map is what makes the
//! overlay genuinely modal.

use std::time::Duration;

use iced::keyboard;
use iced::keyboard::key::Named;
use iced::Subscription;

use crate::core::library::RangeOp;
use crate::core::transfer::TransferKind;
use crate::screens::wall::Dir;

use super::{App, Message};

impl App {
    pub(crate) fn subscription(&self) -> Subscription<Message> {
        // Two keyboards, one live at a time. A question on screen names its own
        // answers, and those can be any letter, so the ordinary bindings would
        // fight them (`o` opens a folder, `r` rotates). Swapping the whole map
        // is what makes the overlay genuinely modal.
        //
        // iced requires these closures to capture nothing, so the choice is
        // made out here rather than inside one of them.
        let keys = if self.confirm.is_some() {
            keyboard::listen().filter_map(answer_key)
        } else {
            keyboard::listen().filter_map(normal_key)
        };

        // A resize changes the wall's column count, so the layout `update` uses
        // for navigation has to be re-measured against the new tree.
        let mut subs = vec![
            keys,
            iced::window::resize_events().map(|_| Message::WallMeasure),
        ];

        // The window starts hidden; reveal it on its first rendered frame.
        // `frames()` only listens for `RedrawRequested` (it adds no redraws of
        // its own) and drops out of the set once `revealed` latches.
        if !self.revealed {
            subs.push(iced::window::frames().map(|_| Message::WindowReady));
        }

        // macOS delivers "Open With" files as Apple Events off the main input
        // path; poll the buffer they land in. No such source elsewhere.
        #[cfg(target_os = "macos")]
        subs.push(iced::time::every(Duration::from_millis(200)).map(|_| Message::PollOpenFiles));

        Subscription::batch(subs)
    }
}

/// The keyboard while a question is up: the keys it names, and nothing else.
fn answer_key(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, .. } = event else {
        return None;
    };
    match key.as_ref() {
        keyboard::Key::Named(Named::Enter) => Some(Message::Activate),
        keyboard::Key::Named(Named::Escape) => Some(Message::Escape),
        keyboard::Key::Character("n") => Some(Message::ConfirmNo),
        // Quitting still works: a question nobody can answer must never be able
        // to trap the user in the app.
        keyboard::Key::Character("q") => Some(Message::Quit),
        keyboard::Key::Character(c) => {
            let mut chars = c.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(Message::ConfirmChoice(c)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The ordinary keyboard. 0.14 unified keyboard subscriptions into a single
/// `listen()` that emits raw `keyboard::Event`s; filter for key-presses here.
fn normal_key(event: keyboard::Event) -> Option<Message> {
    // Clicks read their modifiers from `App::modifiers`, since a `button` press
    // carries none of its own.
    if let keyboard::Event::ModifiersChanged(modifiers) = event {
        return Some(Message::ModifiersChanged(modifiers));
    }
    let keyboard::Event::KeyPressed {
        key,
        modified_key,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    match key.as_ref() {
        keyboard::Key::Named(Named::ArrowRight) => Some(Message::Nav(Dir::Right)),
        keyboard::Key::Named(Named::ArrowLeft) => Some(Message::Nav(Dir::Left)),
        keyboard::Key::Named(Named::ArrowUp) => Some(Message::Nav(Dir::Up)),
        keyboard::Key::Named(Named::ArrowDown) => Some(Message::Nav(Dir::Down)),
        keyboard::Key::Named(Named::Enter) => Some(Message::Activate),
        keyboard::Key::Named(Named::Escape) => Some(Message::Escape),
        keyboard::Key::Named(Named::Space) => Some(Message::ToggleSelected),
        keyboard::Key::Character("l") => Some(Message::Nav(Dir::Right)),
        keyboard::Key::Character("h") => Some(Message::Nav(Dir::Left)),
        keyboard::Key::Character("k") => Some(Message::Nav(Dir::Up)),
        keyboard::Key::Character("j") => Some(Message::Nav(Dir::Down)),
        keyboard::Key::Character("v") => Some(Message::Visual { op: RangeOp::Add }),
        keyboard::Key::Character("x") => Some(Message::Visual {
            op: RangeOp::Remove,
        }),
        keyboard::Key::Character("i") => Some(Message::InvertSelection),
        keyboard::Key::Character("d") => Some(Message::DeleteSelected),
        keyboard::Key::Character("m") => Some(Message::Transfer {
            kind: TransferKind::Move,
        }),
        keyboard::Key::Character("c") => Some(Message::Transfer {
            kind: TransferKind::Copy,
        }),
        keyboard::Key::Character("F") => Some(Message::ToggleFilter),
        keyboard::Key::Character("f") if modifiers.shift() => Some(Message::ToggleFilter),
        keyboard::Key::Character("f") => Some(Message::ToggleFavourite),
        keyboard::Key::Character("a") if modifiers.command() => Some(Message::SelectAll),
        keyboard::Key::Character("q") => Some(Message::Quit),
        keyboard::Key::Character("e") => Some(Message::ToggleFullscreen),
        keyboard::Key::Character("w") => Some(Message::ToggleWall),
        keyboard::Key::Character("o") => Some(Message::OpenFile),
        keyboard::Key::Character("R") => Some(Message::Rotate { clockwise: true }),
        keyboard::Key::Character("r") if modifiers.shift() => {
            Some(Message::Rotate { clockwise: true })
        }
        keyboard::Key::Character("r") => Some(Message::Rotate { clockwise: false }),
        // `key` is the base layout key (no modifiers), so Shift+/ shows
        // up as "/" here; the actual "?" lives in `modified_key`.
        _ if modified_key.as_ref() == keyboard::Key::Character("?") => Some(Message::ToggleHelp),
        _ => None,
    }
}
