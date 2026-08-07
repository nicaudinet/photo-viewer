//! The application shell: the screen that is live, the state that outlives any
//! one screen, and the transitions between them.
//!
//! `App` knows which screens exist; the screens do not know about `App`. Each
//! one owns its state, its actions and its view, and is handed messages by
//! [`update`].
//!
//! | module | what it owns |
//! |---|---|
//! | [`screen`] | which screen is live, and the moves between them |
//! | [`message`] | [`Message`], everything the app can be told |
//! | [`update`] | routing a message to whatever answers it |
//! | [`keys`] | the two keymaps, and the rest of the subscriptions |
//! | [`view`] | the live screen, plus the overlays over it |
//! | [`confirm`] | a modal question and its answers |
//! | [`destination`] | where a move or copy is aimed |
//! | [`trash`] | sending photos to the system trash |

mod confirm;
mod destination;
mod keys;
mod message;
mod screen;
mod trash;
mod update;
mod view;

use std::path::PathBuf;

use iced::{keyboard, Task};


pub(crate) use message::Message;
pub(crate) use screen::Screen;

use confirm::Confirm;

/// The whole model: only screen-independent state lives here. Per-view state
/// lives in the `Screen` variant that needs it.
pub(crate) struct App {
    screen: Screen,
    help_open: bool,
    fullscreen: bool,
    /// The window starts hidden (`visible: false`) and is revealed on its first
    /// rendered frame to avoid a white startup flash; this latches that reveal.
    revealed: bool,
    /// A question waiting on the user. Modal: while it is up the keyboard is
    /// swapped for one that speaks only its answers (see `App::subscription`),
    /// and `update` swallows whatever else still arrives.
    confirm: Option<Confirm>,
    /// Live modifier state, tracked from the keyboard subscription.
    ///
    /// A `button` reports only *that* it was pressed, so a click carrying
    /// `Cmd` or `Shift` has to be read from here instead. Keyboard events are
    /// the only source: if the window loses focus mid-chord this can go stale
    /// until the next key event, which costs at most one mis-read click.
    modifiers: keyboard::Modifiers,
}

impl App {
    pub(crate) fn new() -> (App, Task<Message>) {
        let mut app = App {
            screen: Screen::Empty,
            help_open: false,
            fullscreen: false,
            revealed: false,
            confirm: None,
            modifiers: keyboard::Modifiers::default(),
        };
        let task = match std::env::args().nth(1) {
            Some(arg) => app.open(PathBuf::from(arg)),
            // No path: start on the empty view. On macOS a late "Open With"
            // Apple Event is still picked up by the `PollOpenFiles` timer.
            None => Task::none(),
        };
        (app, task)
    }

    pub(crate) fn title(&self) -> String {
        match self.library().and_then(|lib| lib.current().file_name()) {
            Some(name) => format!("{} — Photo Viewer", name.to_string_lossy()),
            None => "Photo Viewer".to_string(),
        }
    }
}
