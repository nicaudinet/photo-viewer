//! Where a [`Message`] turns into state and tasks.
//!
//! Mostly a routing table. The arms that need more than a line or two live with
//! their concern — [`super::confirm`], [`super::destination`], and the screens
//! themselves — so what is left here is the shape of the dispatch.

use iced::window::Mode;
use iced::Task;

use crate::core::library::IMAGE_EXTENSIONS;
use crate::screens::single::SingleMsg;
use crate::screens::wall::{Click, Dir, WallMsg};

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
                    | Message::Activate
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
            // Esc is a ladder: a pending confirmation first, then the help
            // overlay, then the wall's own rungs (cancel a running batch, then
            // a painted range, then the selection). One press is one rung.
            Message::Escape => {
                if self.confirm.take().is_some() {
                    return Task::none();
                }
                if self.help_open {
                    self.help_open = false;
                    return Task::none();
                }
                match &mut self.screen {
                    Screen::Wall(w) => w.update(WallMsg::Escape),
                    _ => Task::none(),
                }
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

            // Shared keys: disambiguate by screen, then hand to that screen.
            Message::Nav(dir) => match &mut self.screen {
                // The single view is a flat sequence: only left/right mean
                // anything there.
                Screen::Single(s) => match dir {
                    Dir::Left => s.update(SingleMsg::Prev, &mut self.generation),
                    Dir::Right => s.update(SingleMsg::Next, &mut self.generation),
                    Dir::Up | Dir::Down => Task::none(),
                },
                Screen::Wall(w) => w.update(WallMsg::Nav(dir)),
                Screen::Empty => Task::none(),
            },
            Message::Rotate { clockwise } => match &mut self.screen {
                Screen::Single(s) => {
                    let msg = if clockwise {
                        SingleMsg::RotateClockwise
                    } else {
                        SingleMsg::RotateAnticlockwise
                    };
                    s.update(msg, &mut self.generation)
                }
                Screen::Wall(w) => w.update(WallMsg::Rotate { clockwise }),
                Screen::Empty => Task::none(),
            },
            // In the wall, Enter commits a painted range if one is in progress
            // and otherwise opens whatever the ring is around; it means nothing
            // on the other screens. With a question up it answers yes.
            Message::Activate if self.confirm.is_some() => self.answer(None),
            Message::Activate => match &mut self.screen {
                Screen::Wall(w) if w.is_visual() => w.update(WallMsg::CommitVisual),
                Screen::Wall(w) => {
                    let index = w.library.paths.index();
                    self.open_index(index)
                }
                _ => Task::none(),
            },

            // The one selection key the single view answers to as well — and
            // there it applies to the current image alone, like everything on
            // that screen.
            Message::ToggleFavourite => match &mut self.screen {
                Screen::Single(s) => s.update(SingleMsg::ToggleFavourite, &mut self.generation),
                Screen::Wall(w) => w.update(WallMsg::ToggleFavourite),
                Screen::Empty => Task::none(),
            },
            Message::ToggleFilter => self.wall_msg(WallMsg::ToggleFilter),
            Message::TagsSaved(Ok(())) => Task::none(),
            Message::TagsSaved(Err(e)) => {
                eprintln!("Could not save tags: {e}");
                Task::none()
            }

            Message::Visual { op } => self.wall_msg(WallMsg::EnterVisual { op }),
            Message::ToggleSelected => self.wall_msg(WallMsg::ToggleCursor),
            Message::SelectAll => self.wall_msg(WallMsg::SelectAll),
            Message::InvertSelection => self.wall_msg(WallMsg::InvertSelection),

            Message::DeleteSelected => {
                self.ask_about_trash();
                Task::none()
            }
            Message::Transfer { kind } => self.start_transfer(kind),
            // The picker was cancelled: nothing was asked, so nothing happens.
            Message::TransferTarget(None) => Task::none(),
            Message::TransferTarget(Some(plan)) => {
                self.ask_about(plan);
                Task::none()
            }
            Message::ConfirmChoice(key) => self.answer(Some(key)),
            Message::ConfirmNo => {
                self.confirm = None;
                Task::none()
            }
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
                Screen::Single(s) => s.update(m, &mut self.generation),
                _ => Task::none(),
            },
            Message::Wall(m) => match &mut self.screen {
                Screen::Wall(w) => w.update(m),
                _ => Task::none(),
            },
        }
    }
}
