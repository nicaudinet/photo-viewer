//! Where a [`Message`] turns into state and tasks.
//!
//! Mostly a routing table. The arms that need more than a line or two live with
//! their concern — [`super::confirm`], [`super::destination`], and the screens
//! themselves — so what is left here is the shape of the dispatch.

use iced::window::Mode;
use iced::Task;

use crate::screens::wall::Click;

use super::{App, Message, Screen};

impl App {
    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        // A confirmation is modal: nothing else reaches the app until it is
        // answered, so a stray keypress can't act on a library the user is
        // still deciding about.
        if self.confirm.is_some()
            && !matches!(
                message,
                Message::ConfirmNo
                    | Message::ConfirmChoice(_)
                    | Message::ConfirmDefault
                    | Message::Escape
                    | Message::Quit
                    | Message::Removed { .. }
                    | Message::ModifiersChanged(_)
                    | Message::WindowReady
            )
        {
            return Task::none();
        }

        match message {
            Message::Quit => iced::exit(),
            Message::ToggleHelp => {
                self.help_open = !self.help_open;
                Task::none()
            }
            // Esc is a ladder, and these are the two rungs the app owns: a
            // pending question, then the help overlay. The rungs below belong
            // to the live screen and are taken there — the key never reaches
            // this arm while one of them is live, because the app's keymap only
            // claims Esc while the help is up. One press is one rung.
            Message::Escape => {
                self.confirm = None;
                self.help_open = false;
                Task::none()
            }
            Message::ToggleFullscreen => {
                self.fullscreen = !self.fullscreen;
                let mode = if self.fullscreen {
                    Mode::Fullscreen
                } else {
                    Mode::Windowed
                };
                iced::window::latest().and_then(move |id| iced::window::set_mode(id, mode))
            }
            Message::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
                Task::none()
            }
            Message::WindowReady => {
                if self.revealed {
                    return Task::none();
                }
                self.revealed = true;
                iced::window::latest().and_then(|id| iced::window::set_mode(id, Mode::Windowed))
            }

            // Offered to the app first, then to whichever screen is live.
            Message::Key(event) => self.key(event),

            // The picker was cancelled: nothing was asked, so nothing happens.
            Message::TransferTarget(None) => Task::none(),
            Message::TransferTarget(Some(plan)) => {
                self.ask_about(plan);
                Task::none()
            }
            Message::ConfirmDefault => self.answer(None),
            Message::ConfirmChoice(key) => self.answer(Some(key)),
            Message::ConfirmNo => {
                self.confirm = None;
                Task::none()
            }
            Message::Trash(paths) => self.trash(paths),
            Message::Removed { gone, failed } => self.removed(gone, failed),
            Message::WallMeasure => match &self.screen {
                Screen::Wall(_) => crate::screens::wall::measure(),
                _ => Task::none(),
            },

            Message::ToggleWall => self.toggle_wall(),
            Message::OpenFile => {
                // Start where the photos on screen live. Left to itself the
                // panel reopens wherever it was last, which may be a network
                // share or an iCloud folder it then has to list before it can
                // draw — seconds, on a bad day, for a place nobody asked for.
                let dir = self.library().map(|lib| lib.image_dir.clone());
                // The panel is built here, on the main thread, and only its
                // answer is awaited off it.
                Task::perform(
                    crate::core::platform::pick_open_target(dir),
                    Message::OpenFilePicked,
                )
            }
            Message::OpenFilePicked(Some(file)) => self.open(file),
            Message::OpenFilePicked(None) => Task::none(),
            Message::PollOpenFiles => {
                // Finder may hand us several files; open the last (single window).
                let task = match crate::core::platform::take_open_files().pop() {
                    Some(path) => self.open(path),
                    None => Task::none(),
                };
                // The same tick is a good enough idle clock for the one job
                // that wants one. Idempotent, so asking every 200ms is free
                // after the first time it says yes.
                self.prewarm_file_dialog();
                task
            }
            Message::ThumbClicked(index) => {
                let modifiers = self.modifiers;
                match &mut self.screen {
                    Screen::Wall(w) => match w.click(index, modifiers) {
                        Click::Open => self.open_index(index),
                        Click::Handled(task) => task,
                    },
                    _ => Task::none(),
                }
            }

            // Screen-local: only acts when that screen is current.
            Message::Single(m) => match &mut self.screen {
                Screen::Single(s) => s.update(m),
                _ => Task::none(),
            },
            Message::Wall(m) => match &mut self.screen {
                Screen::Wall(w) => w.update(m),
                _ => Task::none(),
            },
        }
    }

    /// Build the first native file panel, once, at a moment nobody minds.
    ///
    /// The panel is expensive to bring up the first time and the cost lands on
    /// the main thread — about a second of frozen app, see
    /// [`crate::core::platform::prewarm_file_dialog`]. So it is spent when the
    /// window is up and nothing else is running: not during startup, where it
    /// would only look like a slow launch, and not while thumbnails are landing,
    /// where it would stall the wall filling in. In practice that is a second or
    /// two after the folder settles, long before anyone reaches for `o`.
    fn prewarm_file_dialog(&self) {
        let busy = match &self.screen {
            Screen::Wall(w) => w.is_decoding(),
            _ => false,
        };
        if self.revealed && !busy {
            crate::core::platform::prewarm_file_dialog();
        }
    }
}
