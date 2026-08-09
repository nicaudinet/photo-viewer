//! What the app listens to, and who gets first look at a key.
//!
//! Two keymaps, one live at a time. A question on screen names its own answers,
//! and those can be any letter, so the ordinary bindings would fight them (`o`
//! opens a folder, `r` rotates). Swapping the whole map is what makes the
//! overlay genuinely modal.
//!
//! Outside a question the app claims only the keys that mean the same thing
//! wherever you are — quit, fullscreen, open, help. Every other key is offered
//! to whichever screen is live, which is why the app's vocabulary has no word
//! for "paint a range" or "stack similar photos". A key that leaves one screen
//! for another belongs to the screen it leaves, not here: the help reads as a
//! list of what can be done *here*, and a row under "Everywhere" that is live
//! on one screen out of three does not.
//!
//! Both keymaps are tables ([`crate::keymap`]), so every key carries the
//! sentence the help shows and a guard saying when it is live.

use std::time::Duration;

use iced::keyboard::{self, key::Named};
use iced::{Subscription, Task};

use crate::keymap::{lookup, rows, Binding, Chord, Row};
use crate::screens::single::SingleKey;
use crate::screens::wall::WallKey;

use super::{App, Message, Screen};

/// A key press, once the live screen has had a look at it.
enum Keyed {
    Single(SingleKey),
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
    ///
    /// A key that either of them answers also closes the help overlay: it is a
    /// reference, not a mode, so nothing has to be dismissed before working.
    /// `Esc` is no exception — closing the list is something a press does on its
    /// way past, not instead of its work, so the same press goes on to drop
    /// whatever the screen was holding.
    pub(super) fn key(&mut self, event: keyboard::Event) -> Task<Message> {
        // Clicks read their modifiers from `App::modifiers`, since a `button`
        // press carries none of its own. Not a binding: nothing was pressed
        // that the user could look up.
        if let keyboard::Event::ModifiersChanged(modifiers) = event {
            return self.update(Message::ModifiersChanged(modifiers));
        }

        let was_open = self.help_open;
        // `Esc` closes the help even when the screen below has nothing left to
        // drop, so it counts as answered whether or not a task comes back.
        let escaped = was_open && is_escape(&event);
        let task = self.dispatch(event);
        if was_open && (task.is_some() || escaped) {
            self.help_open = false;
        }
        task.unwrap_or_else(Task::none)
    }

    /// Offer the key to the app's keymap, then to the live screen's. `None` if
    /// neither of them binds it — which is what leaves the help up.
    fn dispatch(&mut self, event: keyboard::Event) -> Option<Task<Message>> {
        // The help's own `Esc` rung is not a stopping point: the caller closes
        // the list, and the key carries on down the screen's ladder. It stays a
        // binding all the same, so the help can name the key that dismisses it.
        if !(self.help_open && is_escape(&event)) {
            if let Some(message) = lookup(APP, self, &event) {
                return Some(self.update(message));
            }
        }

        // Look the key up first and act on the answer second: the lookup
        // borrows the live screen, and acting on it may replace that screen.
        let keyed = match &self.screen {
            Screen::Empty => None,
            Screen::Single(s) => s.key(&event).map(Keyed::Single),
            Screen::Wall(w) => w.key(&event).map(Keyed::Wall),
        };

        Some(match keyed? {
            Keyed::Single(SingleKey::Msg(msg)) => self.update(Message::Single(msg)),
            Keyed::Single(SingleKey::ShowWall) => self.show_wall(),
            Keyed::Wall(WallKey::Msg(msg)) => self.update(Message::Wall(msg)),
            Keyed::Wall(WallKey::Open(index)) => self.open_index(index),
            Keyed::Wall(WallKey::Trash) => {
                self.ask_about_trash();
                Task::none()
            }
            Keyed::Wall(WallKey::Pick) => {
                self.ask_about_pick();
                Task::none()
            }
            Keyed::Wall(WallKey::Transfer(kind)) => self.start_transfer(kind),
        })
    }

    /// Everything the app itself can be told right now, for the help overlay.
    pub(super) fn bindings(&self) -> Vec<Row> {
        rows(APP, self)
    }
}

/// Whether `event` is a bare `Esc` press.
fn is_escape(event: &keyboard::Event) -> bool {
    matches!(
        event,
        keyboard::Event::KeyPressed { key, modifiers, .. }
            if key.as_ref() == keyboard::Key::Named(Named::Escape) && !modifiers.command()
    )
}

/// The keys the app claims for itself: the ones that mean the same thing on
/// every screen. Anything else is the live screen's to answer.
///
/// `Esc` is listed only while the help is up, and only to say the help closes
/// with it — the press itself is not stopped here. Every rung below belongs to
/// whichever screen has something to drop — a painted range, a selection, the
/// stack the user is standing in.
///
/// Nothing here is guarded on which screen is live. A key that only works on
/// one of them is that screen's, and lives in its table.
const APP: &[Binding<App, Message>] = &[
    // `?` reaches here as the modified key: on a US layout the key pressed is
    // `/`, and the `?` exists only once shift is applied.
    //
    // One line, both ways: `?` reads as the key the list lives on, and a row
    // that only ever says "close" while the list is up says less than that.
    Binding::always(&[Chord::sym('?')], "Toggle the help menu", |_| {
        Message::ToggleHelp
    }),
    Binding::when(
        &[Chord::named(Named::Escape)],
        "Close this help",
        |app| app.help_open,
        |_| Message::ToggleHelp,
    ),
    Binding::always(&[Chord::key('o')], "Open a file or folder", |_| {
        Message::OpenFile
    }),
    Binding::when(
        &[Chord::key('e')],
        "Leave fullscreen",
        |app| app.fullscreen,
        |_| Message::ToggleFullscreen,
    ),
    Binding::when(
        &[Chord::key('e')],
        "Go fullscreen",
        |app| !app.fullscreen,
        |_| Message::ToggleFullscreen,
    ),
    Binding::always(&[Chord::key('q')], "Quit", |_| Message::AskQuit),
];

/// The keyboard while a question is up: the keys it names, and nothing else.
///
/// Letters only. Enter and Esc are the two keys pressed without looking, and an
/// answer about files that cannot be got back should be typed on purpose.
fn answer_key(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, .. } = event else {
        return None;
    };
    match key.as_ref() {
        keyboard::Key::Character("n") => Some(Message::ConfirmNo),
        // Quitting still works: a question nobody can answer must never be able
        // to trap the user in the app. It asks the same question `q` always
        // asks, replacing whatever was up, so the way out is two presses rather
        // than one — never none.
        keyboard::Key::Character("q") => Some(Message::AskQuit),
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::core::library::Library;
    use crate::core::pointed_list::PointedList;
    use crate::core::tags::Tags;
    use crate::screens::wall::WallState;

    /// An app on the empty screen with the help up. Built directly rather than
    /// through `App::new`, which reads `std::env::args` — under a test harness
    /// that is the harness's own arguments.
    fn helped() -> App {
        App {
            screen: Screen::Empty,
            help_open: true,
            fullscreen: false,
            revealed: false,
            confirm: None,
            beneath: None,
            modifiers: keyboard::Modifiers::default(),
        }
    }

    fn press(key: keyboard::Key) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            repeat: false,
            text: None,
        }
    }

    fn press_char(c: &str) -> keyboard::Event {
        press(keyboard::Key::Character(c.into()))
    }

    #[test]
    fn a_bound_key_closes_the_help_on_its_way_through() {
        let mut app = helped();
        // `e` is the app's, and does its work as well as closing the list.
        let _ = app.key(press_char("e"));
        assert!(!app.help_open);
        assert!(app.fullscreen, "the key was swallowed rather than acted on");
    }

    #[test]
    fn an_unbound_key_leaves_the_help_up() {
        let mut app = helped();
        let _ = app.key(press_char("z"));
        assert!(app.help_open, "a key that does nothing must change nothing");
    }

    #[test]
    fn the_help_key_closes_it_again() {
        let mut app = helped();
        let _ = app.key(press(keyboard::Key::Character("?".into())));
        assert!(!app.help_open);
    }

    #[test]
    fn quitting_asks_first() {
        let mut app = helped();
        let _ = app.key(press_char("q"));
        let confirm = app.confirm.as_ref().expect("q left without asking");
        assert!(
            confirm.choices.iter().any(|c| c.key == 'y'),
            "the question offered no way to say yes"
        );
    }

    #[test]
    fn a_question_on_screen_cannot_swallow_the_way_out() {
        // Whatever is being asked, `q` asks about leaving instead — so no
        // question can hold the user in the app.
        assert!(matches!(
            answer_key(press_char("q")),
            Some(Message::AskQuit)
        ));
    }

    #[test]
    fn escape_closes_the_help_with_nothing_below_it() {
        let mut app = helped();
        let _ = app.key(press(keyboard::Key::Named(Named::Escape)));
        assert!(!app.help_open);
        // And with the help down it is no longer the app's key at all: the
        // rungs below belong to the live screen.
        assert!(lookup(APP, &app, &press(keyboard::Key::Named(Named::Escape))).is_none());
    }

    /// A wall of one photo, that photo selected — so Esc has something to drop.
    fn selected_wall() -> WallState {
        let files = vec![PathBuf::from("0.jpg")];
        WallState::new(Library {
            all: files.clone(),
            paths: PointedList::new(files.clone()).unwrap(),
            image_dir: PathBuf::from("/wall"),
            selection: files.into_iter().collect(),
            tags: Tags::new(),
            filter: None,
            grouping: None,
        })
    }

    #[test]
    fn the_wall_has_no_key_back_to_the_single_view() {
        // A photograph is opened by naming it — Enter, or a click — so `w` on
        // the wall must not pick one for the user.
        let mut app = helped();
        app.screen = Screen::Wall(selected_wall());
        let _ = app.key(press_char("w"));
        assert!(matches!(app.screen, Screen::Wall(_)), "the wall was left");
        assert!(app.help_open, "a key that does nothing must change nothing");
    }

    #[test]
    fn escape_reaches_the_screen_through_the_help() {
        let mut app = helped();
        app.screen = Screen::Wall(selected_wall());
        let _ = app.key(press(keyboard::Key::Named(Named::Escape)));
        assert!(!app.help_open);
        let Screen::Wall(wall) = &app.screen else {
            unreachable!("the wall was replaced")
        };
        assert!(
            wall.library.selection.is_empty(),
            "the help swallowed the press instead of passing it on"
        );
    }
}
