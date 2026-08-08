//! Where a [`Message`] turns into state and tasks.
//!
//! Mostly a routing table. The arms that need more than a line or two live with
//! their concern — [`super::confirm`], [`super::destination`], and the screens
//! themselves — so what is left here is the shape of the dispatch.

use iced::window::Mode;
use iced::Task;

use crate::core::library::IMAGE_EXTENSIONS;
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
            Message::OpenFile => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Select an image to open")
                        .add_filter("Images", &IMAGE_EXTENSIONS)
                        .pick_file()
                        .await
                        .map(|handle| handle.path().to_path_buf())
                },
                Message::OpenFilePicked,
            ),
            Message::OpenFilePicked(Some(file)) => self.open(file),
            Message::OpenFilePicked(None) => Task::none(),
            Message::PollOpenFiles => {
                // Finder may hand us several files; open the last (single window).
                match crate::core::platform::take_open_files().pop() {
                    Some(path) => self.open(path),
                    None => Task::none(),
                }
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
}
