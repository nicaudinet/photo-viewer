//! PhotoViewer — Rust + iced rewrite.
//!
//! Phase 5: rotate, open-dir picker, and the empty view.
//! - Single view: fit-to-window decode, `←/→ h/l` nav, `r`/`Shift+R` rotate
//!   (writes the file to disk), `f`/`d` favourite/delete toggles + icon
//!   overlays, `Cmd+D` delete-all (confirm overlay), `Cmd+F` save-favourites.
//! - Wall view: async 300px thumbnails laid out shortest-column masonry in a
//!   vertical scroll, current image ringed, `←/→/↑/↓ hjkl` to move the ring
//!   (scrolling it into view), Enter or a click to open it, `r`/`Shift+R` to
//!   rotate it on disk, `f`/`d` filter to favourites/to-delete only.
//! - `w` toggles between the two; `o` opens a directory (native picker); a
//!   directory opens in wall view, a file in single view.
//! - Empty view when no image is loaded. `q` quit, `e` fullscreen, `?`/`Esc`
//!   help. Roadmap in `RUST_REWRITE_PLAN.md`.
//!
//! `App` here owns only screen-independent state and the transitions between
//! screens; each screen's own state, actions, and view live in `gui/`.

// Pure domain core (Phase 1). Later phases consume more of its API; allow the
// still-unused surface for now.
#[allow(dead_code)]
mod library;
mod imaging;
mod platform;
#[allow(dead_code)]
mod pointed_list;

mod gui;

use std::path::{Path, PathBuf};
use std::time::Duration;

use iced::keyboard;
use iced::keyboard::key::Named;
use iced::widget::{image, Stack};
use iced::window::Mode;
use iced::{Element, Size, Subscription, Task, Theme};

use gui::empty::empty_view;
use gui::single::{SingleMsg, SingleState};
use gui::wall::{Dir, WallMsg, WallState};
use gui::{confirm_overlay, help_overlay};
use library::{load_library, Library, IMAGE_EXTENSIONS};

/// Star/delete overlay icons, baked into the binary (no runtime path lookup).
const STAR_ICON: &[u8] = include_bytes!("../icons/star.png");
const DELETE_ICON: &[u8] = include_bytes!("../icons/delete.png");

pub fn main() -> iced::Result {
    // Register the macOS open-file handler before the event loop starts, so a
    // launch-time "Open With" event is caught (no-op off macOS).
    platform::install_open_file_handler();

    // 0.14 moved `boot` to the first argument (was `run_with`) and turned the
    // title into a builder method.
    iced::application(App::new, App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .theme(Theme::Dark)
        .window(iced::window::Settings {
            size: Size::new(800.0, 600.0),
            // Start hidden and reveal on the first rendered frame (see
            // `App::subscription` / `Message::WindowReady`) so the window never
            // shows the OS default background before iced paints — this is what
            // removes the white startup flash.
            visible: false,
            ..iced::window::Settings::default()
        })
        .run()
}

/// Which view is on screen, together with that view's own state.
///
/// The library lives inside `Single`/`Wall`, so those views are unrepresentable
/// without one — `Empty` genuinely carries nothing. Toggling between views moves
/// the library across; the losing view's decode caches are dropped (and re-built
/// on return — a deliberate, revisitable trade-off).
enum Screen {
    Empty,
    Single(SingleState),
    Wall(WallState),
}

/// The whole model: only screen-independent state lives here. Per-view state
/// lives in the `Screen` variant that needs it.
struct App {
    screen: Screen,
    /// Bumped on every large decode (nav / open / rotate / toggle); tags
    /// `SingleMsg::LargeDecoded` so a superseded decode can't overwrite a newer
    /// one. Global and monotonic — kept out of `SingleState` so it can't reset
    /// and let a stale in-flight decode collide with a fresh session's number.
    generation: u64,
    help_open: bool,
    fullscreen: bool,
    /// The window starts hidden (`visible: false`) and is revealed on its first
    /// rendered frame to avoid a white startup flash; this latches that reveal.
    revealed: bool,
    star_icon: image::Handle,
    delete_icon: image::Handle,
}

#[derive(Debug, Clone)]
enum Message {
    // Global — handled at the top level regardless of screen.
    Quit,
    ToggleFullscreen,
    ToggleHelp,
    /// Esc: cancel the confirm overlay if open, else close help.
    Escape,
    /// The first frame has rendered; reveal the (initially hidden) window.
    WindowReady,

    // Ambiguous shared keys: same key, different meaning per screen. The live
    // screen isn't visible to `subscription`, so these stay neutral here and are
    // dispatched to the current screen's update in `App::update`.
    /// `f`: favourite toggle (single) or favourites filter (wall).
    KeyF,
    /// `d`: delete toggle (single) or to-delete filter (wall).
    KeyD,
    /// Arrows / `hjkl`: previous-next (single) or grid movement (wall).
    Nav(Dir),
    /// `r` / `Shift+R`: rotate the current image (single) or the selected
    /// thumbnail (wall). Both write the file to disk.
    Rotate { clockwise: bool },
    /// Enter: confirm a pending delete (single) or open the selection (wall).
    Activate,
    /// The window resized: re-measure the wall's viewport if it is on screen.
    /// The event's own size is the *window's*, not the scroll viewport's, so it
    /// is only a trigger — `gui::wall::measure` reads the real number back.
    WallMeasure,

    // Transitions between screens — owned by `App` because a per-screen update
    // can't reassign `self.screen`.
    ToggleWall,
    /// `o`: pick a single image file to open (single view).
    OpenFile,
    OpenFilePicked(Option<PathBuf>),
    /// Timer tick: drain any paths the platform delivered (macOS "Open With").
    PollOpenFiles,
    ThumbClicked(usize),
    DeleteAll,
    ConfirmYes,
    ConfirmNo,

    // Delegated to the current screen's own update.
    Single(SingleMsg),
    Wall(WallMsg),
}

impl App {
    fn new() -> (App, Task<Message>) {
        let mut app = App {
            screen: Screen::Empty,
            generation: 0,
            help_open: false,
            fullscreen: false,
            revealed: false,
            star_icon: image::Handle::from_bytes(STAR_ICON.to_vec()),
            delete_icon: image::Handle::from_bytes(DELETE_ICON.to_vec()),
        };
        let task = match std::env::args().nth(1) {
            Some(arg) => app.open(PathBuf::from(arg)),
            // No path: start on the empty view. On macOS a late "Open With"
            // Apple Event is still picked up by the `PollOpenFiles` timer.
            None => Task::none(),
        };
        (app, task)
    }

    /// Load the directory for `path`. A file opens in single view pointed at
    /// that file; a directory opens in the wall view (matching the Python app).
    fn open(&mut self, path: PathBuf) -> Task<Message> {
        let (dir, target) = if path.is_file() {
            let parent = path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.clone());
            (parent, Some(path))
        } else {
            (path, None)
        };

        match load_library(&dir) {
            Ok(Some(mut lib)) => match target {
                // A file: open it directly in single view.
                Some(file) => {
                    lib.paths.goto_value(&file);
                    let single = SingleState::new(lib);
                    let task = single.decode_current(&mut self.generation);
                    self.screen = Screen::Single(single);
                    task
                }
                // A directory: show the wall of all its images.
                None => {
                    let mut wall = WallState::new(lib);
                    let task = wall.enter();
                    self.screen = Screen::Wall(wall);
                    task
                }
            },
            Ok(None) => {
                self.screen = Screen::Empty;
                Task::none()
            }
            Err(e) => {
                eprintln!("Failed to load {}: {e}", dir.display());
                self.screen = Screen::Empty;
                Task::none()
            }
        }
    }

    /// Leave the wall and open `index` in the single view. Shared by clicking a
    /// thumbnail and by pressing Enter on the selection.
    fn open_index(&mut self, index: usize) -> Task<Message> {
        match std::mem::replace(&mut self.screen, Screen::Empty) {
            Screen::Wall(w) => {
                let mut library = w.library;
                library.goto(index);
                let single = SingleState::new(library);
                let task = single.decode_current(&mut self.generation);
                self.screen = Screen::Single(single);
                task
            }
            // Only the wall opens by index; restore anything else.
            other => {
                self.screen = other;
                Task::none()
            }
        }
    }

    /// Accept a pending delete confirmation, if one is open.
    fn confirm_yes(&mut self) -> Task<Message> {
        let confirmed = if let Screen::Single(s) = &mut self.screen {
            s.confirm_delete.take().is_some()
        } else {
            false
        };
        if confirmed {
            self.do_delete_all()
        } else {
            Task::none()
        }
    }

    /// Unlink every marked file, then re-decode the new current — or fall back
    /// to the empty view if the whole library was deleted.
    fn do_delete_all(&mut self) -> Task<Message> {
        let empty = {
            let Screen::Single(s) = &mut self.screen else {
                return Task::none();
            };
            if let Err(e) = s.library.delete_all() {
                eprintln!("Delete-all failed: {e}");
            }
            s.library.paths.is_empty()
        };
        if empty {
            self.screen = Screen::Empty;
            Task::none()
        } else {
            match &self.screen {
                Screen::Single(s) => s.decode_current(&mut self.generation),
                _ => Task::none(),
            }
        }
    }

    /// The current library, whichever loaded view holds it.
    fn library(&self) -> Option<&Library> {
        match &self.screen {
            Screen::Single(s) => Some(&s.library),
            Screen::Wall(w) => Some(&w.library),
            Screen::Empty => None,
        }
    }

    fn title(&self) -> String {
        match self.library().and_then(|lib| lib.current().file_name()) {
            Some(name) => format!("{} — Photo Viewer", name.to_string_lossy()),
            None => "Photo Viewer".to_string(),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        // While the confirm overlay is up it's modal: swallow everything but
        // the confirm/cancel/quit keys.
        let confirming = matches!(&self.screen, Screen::Single(s) if s.confirm_delete.is_some());
        if confirming
            && !matches!(
                message,
                Message::ConfirmYes
                    | Message::ConfirmNo
                    | Message::Activate
                    | Message::Escape
                    | Message::Quit
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
            Message::Escape => {
                if let Screen::Single(s) = &mut self.screen {
                    if s.confirm_delete.take().is_some() {
                        return Task::none();
                    }
                }
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
            Message::WindowReady => {
                if self.revealed {
                    return Task::none();
                }
                self.revealed = true;
                iced::window::latest().and_then(|id| iced::window::set_mode(id, Mode::Windowed))
            }

            // Shared keys: disambiguate by screen, then hand to that screen.
            Message::KeyF => match &mut self.screen {
                Screen::Single(s) => s.update(SingleMsg::ToggleFavourite, &mut self.generation),
                Screen::Wall(w) => w.update(WallMsg::FilterFavourites),
                Screen::Empty => Task::none(),
            },
            Message::KeyD => match &mut self.screen {
                Screen::Single(s) => s.update(SingleMsg::ToggleDelete, &mut self.generation),
                Screen::Wall(w) => w.update(WallMsg::FilterToDelete),
                Screen::Empty => Task::none(),
            },
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
            Message::Activate => match &self.screen {
                // In the wall, Enter opens whatever the ring is around.
                Screen::Wall(w) => {
                    let index = w.library.paths.index();
                    self.open_index(index)
                }
                // Elsewhere it only means "yes" to the confirm overlay.
                _ => self.confirm_yes(),
            },
            Message::WallMeasure => match &self.screen {
                Screen::Wall(_) => gui::wall::measure(),
                _ => Task::none(),
            },

            // Move the library across to the other view. The losing view's
            // decode cache is dropped, so the new view always decodes fresh.
            Message::ToggleWall => match std::mem::replace(&mut self.screen, Screen::Empty) {
                Screen::Single(s) => {
                    let mut wall = WallState::new(s.library);
                    let task = wall.enter();
                    self.screen = Screen::Wall(wall);
                    task
                }
                Screen::Wall(w) => {
                    let single = SingleState::new(w.library);
                    let task = single.decode_current(&mut self.generation);
                    self.screen = Screen::Single(single);
                    task
                }
                Screen::Empty => Task::none(),
            },
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
                match platform::take_open_files().pop() {
                    Some(path) => self.open(path),
                    None => Task::none(),
                }
            }
            Message::ThumbClicked(index) => self.open_index(index),
            Message::DeleteAll => {
                if let Screen::Single(s) = &mut self.screen {
                    let count = s.library.to_delete.len();
                    if count > 0 {
                        s.confirm_delete = Some(count);
                    }
                }
                Task::none()
            }
            Message::ConfirmYes => self.confirm_yes(),
            Message::ConfirmNo => {
                if let Screen::Single(s) = &mut self.screen {
                    s.confirm_delete = None;
                }
                Task::none()
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

    fn subscription(&self) -> Subscription<Message> {
        // 0.14 unified keyboard subscriptions into a single `listen()` that
        // emits raw `keyboard::Event`s; filter for key-presses ourselves.
        let keys = keyboard::listen().filter_map(|event| {
            let keyboard::Event::KeyPressed { key, modified_key, modifiers, .. } = event else {
                return None;
            };
            let cmd = modifiers.command();
            match key.as_ref() {
                keyboard::Key::Named(Named::ArrowRight) => Some(Message::Nav(Dir::Right)),
                keyboard::Key::Named(Named::ArrowLeft) => Some(Message::Nav(Dir::Left)),
                keyboard::Key::Named(Named::ArrowUp) => Some(Message::Nav(Dir::Up)),
                keyboard::Key::Named(Named::ArrowDown) => Some(Message::Nav(Dir::Down)),
                keyboard::Key::Named(Named::Enter) => Some(Message::Activate),
                keyboard::Key::Named(Named::Escape) => Some(Message::Escape),
                keyboard::Key::Character("l") => Some(Message::Nav(Dir::Right)),
                keyboard::Key::Character("h") => Some(Message::Nav(Dir::Left)),
                keyboard::Key::Character("k") => Some(Message::Nav(Dir::Up)),
                keyboard::Key::Character("j") => Some(Message::Nav(Dir::Down)),
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
                _ if modified_key.as_ref() == keyboard::Key::Character("?") => {
                    Some(Message::ToggleHelp)
                }
                keyboard::Key::Character("f") if cmd => Some(Message::Single(SingleMsg::SaveFavourites)),
                keyboard::Key::Character("f") => Some(Message::KeyF),
                keyboard::Key::Character("d") if cmd => Some(Message::DeleteAll),
                keyboard::Key::Character("d") => Some(Message::KeyD),
                keyboard::Key::Character("y") => Some(Message::ConfirmYes),
                keyboard::Key::Character("n") => Some(Message::ConfirmNo),
                _ => None,
            }
        });

        // A resize changes the wall's column count, so the layout `update` uses
        // for navigation has to be re-measured against the new tree.
        let mut subs = vec![keys, iced::window::resize_events().map(|_| Message::WallMeasure)];

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

    fn view(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match &self.screen {
            Screen::Empty => empty_view(),
            Screen::Single(s) => s.view(&self.star_icon, &self.delete_icon),
            Screen::Wall(w) => w.view(&self.star_icon, &self.delete_icon),
        };

        let mut layers: Vec<Element<'_, Message>> = vec![content];
        if self.help_open {
            layers.push(help_overlay());
        }
        if let Screen::Single(s) = &self.screen {
            if let Some(count) = s.confirm_delete {
                layers.push(confirm_overlay(count));
            }
        }

        if layers.len() == 1 {
            layers.pop().unwrap()
        } else {
            Stack::with_children(layers).into()
        }
    }
}
