//! What the app listens to, and who gets first look at a key.
//!
//! Two keymaps, one live at a time. A question on screen names its own answers,
//! and those can be any letter, so the ordinary bindings would fight them (`o`
//! opens a folder, `r` rotates). Swapping the whole map is what makes the
//! overlay genuinely modal.
//!
//! Outside a question the app claims only the keys that mean the same thing
//! wherever you are — quit, fullscreen, help, and moving between screens. Every
//! other key is offered to whichever screen is live, which is why the app's
//! vocabulary has no word for "paint a range" or "invert the selection".

use std::time::Duration;

use iced::keyboard::{self, key::Named};
use iced::{Subscription, Task};

use crate::screens::single::SingleMsg;
use crate::screens::wall::WallKey;

use super::{App, Message, Screen};

/// A key press, once the live screen has had a look at it.
enum Keyed {
    Single(SingleMsg),
    Wall(WallKey),
}

impl App {
    pub(crate) fn subscription(&self) -> Subscription<Message> {
        // iced requires these closures to capture nothing, so the choice of
        // keymap is made out here rather than inside one of them.
        let keys = if self.confirm.is_some() {
            keyboard::listen().filter_map(answer_key)
        } else {
            keyboard::listen().map(Message::Key)
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

    /// Offer a key to the app, then to the live screen.
    pub(super) fn key(&mut self, event: keyboard::Event) -> Task<Message> {
        if let Some(message) = app_key(&event) {
            return self.update(message);
        }

        // Look the key up first and act on the answer second: the lookup
        // borrows the live screen, and acting on it may replace that screen.
        let keyed = match &self.screen {
            Screen::Empty => None,
            Screen::Single(s) => s.key(&event).map(Keyed::Single),
            Screen::Wall(w) => w.key(&event).map(Keyed::Wall),
        };

        match keyed {
            None => Task::none(),
            Some(Keyed::Single(msg)) => self.update(Message::Single(msg)),
            Some(Keyed::Wall(WallKey::Msg(msg))) => self.update(Message::Wall(msg)),
            Some(Keyed::Wall(WallKey::Open(index))) => self.open_index(index),
            Some(Keyed::Wall(WallKey::Trash)) => {
                self.ask_about_trash();
                Task::none()
            }
            Some(Keyed::Wall(WallKey::Pick)) => {
                self.ask_about_pick();
                Task::none()
            }
            Some(Keyed::Wall(WallKey::Transfer(kind))) => self.start_transfer(kind),
        }
    }
}

/// The keys the app claims for itself: the ones that mean the same thing on
/// every screen. Anything else is the live screen's to answer.
fn app_key(event: &keyboard::Event) -> Option<Message> {
    // Clicks read their modifiers from `App::modifiers`, since a `button` press
    // carries none of its own.
    if let keyboard::Event::ModifiersChanged(modifiers) = event {
        return Some(Message::ModifiersChanged(*modifiers));
    }
    let keyboard::Event::KeyPressed {
        key, modified_key, ..
    } = event
    else {
        return None;
    };
    match key.as_ref() {
        keyboard::Key::Named(Named::Escape) => Some(Message::Escape),
        keyboard::Key::Character("q") => Some(Message::Quit),
        keyboard::Key::Character("e") => Some(Message::ToggleFullscreen),
        keyboard::Key::Character("w") => Some(Message::ToggleWall),
        keyboard::Key::Character("o") => Some(Message::OpenFile),
        // `key` is the base layout key (no modifiers), so Shift+/ shows
        // up as "/" here; the actual "?" lives in `modified_key`.
        _ if modified_key.as_ref() == keyboard::Key::Character("?") => Some(Message::ToggleHelp),
        _ => None,
    }
}

/// The keyboard while a question is up: the keys it names, and nothing else.
fn answer_key(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, .. } = event else {
        return None;
    };
    match key.as_ref() {
        keyboard::Key::Named(Named::Enter) => Some(Message::ConfirmDefault),
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
