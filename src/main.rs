//! PhotoViewer — Rust + iced rewrite.
//!
//! Phase 3: single view + favourite/delete. On top of the Phase 2 MVP (open a
//! dir/file, fit-to-window decode, `←/→ h/l` nav, `q` quit, `e` fullscreen,
//! `?`/`Esc` help): `f` toggles favourite (un-favouriting also un-deletes),
//! `d` toggles mark-to-delete (refused on a favourite), star/delete icon
//! overlays, `Cmd+D` delete-all with an in-app confirm overlay, `Cmd+F`
//! save-favourites via a native dir picker. Roadmap in `RUST_REWRITE_PLAN.md`.

// Pure domain core (Phase 1). Later phases consume more of its API; allow the
// still-unused surface for now.
#[allow(dead_code)]
mod library;
#[allow(dead_code)]
mod pointed_list;

use std::path::{Path, PathBuf};

use iced::alignment::{Horizontal, Vertical};
use iced::keyboard;
use iced::keyboard::key::Named;
use iced::widget::{center, column, container, image, row, text, Stack};
use iced::window::Mode;
use iced::{Background, Border, Color, ContentFit, Element, Length, Size, Subscription, Task, Theme};

use library::{load_library, Library};

/// Star/delete overlay icons, baked into the binary (no runtime path lookup).
const STAR_ICON: &[u8] = include_bytes!("../icons/star.png");
const DELETE_ICON: &[u8] = include_bytes!("../icons/delete.png");

const ICON_SIZE: f32 = 40.0;
const ICON_MARGIN: f32 = 10.0;

pub fn main() -> iced::Result {
    iced::application(App::title, App::update, App::view)
        .subscription(App::subscription)
        .theme(|_app| Theme::Dark)
        .window_size(Size::new(800.0, 600.0))
        .run_with(App::new)
}

/// The whole model. `library` is `None` until a directory with images loads
/// (the "empty" state); `large` holds the current fit-to-window decode.
struct App {
    library: Option<Library>,
    /// Latest decoded image for the current path, or `None` while decoding.
    large: Option<image::Handle>,
    /// Bumped on every navigation; a `LargeDecoded` with a stale tag is dropped.
    generation: u64,
    help_open: bool,
    fullscreen: bool,
    /// `Some(count)` while the delete-all confirmation overlay is showing.
    confirm_delete: Option<usize>,
    star_icon: image::Handle,
    delete_icon: image::Handle,
}

#[derive(Debug, Clone)]
enum Message {
    Next,
    Prev,
    Quit,
    ToggleFullscreen,
    ToggleHelp,
    /// Esc: cancel the confirm overlay if open, else close help.
    Escape,
    ToggleFavourite,
    ToggleDelete,
    SaveFavourites,
    SaveFavDirPicked(Option<PathBuf>),
    DeleteAll,
    ConfirmYes,
    ConfirmNo,
    LargeDecoded {
        generation: u64,
        result: Result<image::Handle, String>,
    },
}

/// Shown in the help overlay, in press-order.
const SHORTCUTS: &[(&str, &str)] = &[
    ("\u{2190} / h", "Previous image"),
    ("\u{2192} / l", "Next image"),
    ("f", "Favourite (toggle)"),
    ("\u{2318}F", "Save favourites"),
    ("d", "Mark to delete (toggle)"),
    ("\u{2318}D", "Delete all marked"),
    ("e", "Fullscreen (toggle)"),
    ("?", "Show help (toggle)"),
    ("Esc", "Close help"),
    ("q", "Quit"),
];

impl App {
    fn new() -> (App, Task<Message>) {
        let mut app = App {
            library: None,
            large: None,
            generation: 0,
            help_open: false,
            fullscreen: false,
            confirm_delete: None,
            star_icon: image::Handle::from_bytes(STAR_ICON.to_vec()),
            delete_icon: image::Handle::from_bytes(DELETE_ICON.to_vec()),
        };
        let task = match std::env::args().nth(1) {
            Some(arg) => app.open(PathBuf::from(arg)),
            None => Task::none(),
        };
        (app, task)
    }

    /// Load the directory for `path` (its parent if `path` is a file), pointing
    /// the cursor at the file when one was named. Returns the decode task for
    /// the resulting current image, or empties out if nothing loadable.
    fn open(&mut self, path: PathBuf) -> Task<Message> {
        let (dir, target) = if path.is_file() {
            let parent = path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.clone());
            (parent, Some(path))
        } else {
            (path, None)
        };

        match load_library(&dir) {
            Ok(Some(mut lib)) => {
                if let Some(target) = target {
                    lib.paths.goto_value(&target);
                }
                self.library = Some(lib);
                self.decode_current()
            }
            Ok(None) => {
                self.library = None;
                Task::none()
            }
            Err(e) => {
                eprintln!("Failed to load {}: {e}", dir.display());
                self.library = None;
                Task::none()
            }
        }
    }

    /// Kick off an off-thread decode of the current image, tagged with a fresh
    /// generation so an earlier in-flight decode can't overwrite it.
    fn decode_current(&mut self) -> Task<Message> {
        let Some(lib) = &self.library else {
            return Task::none();
        };
        self.generation += 1;
        let generation = self.generation;
        let path = lib.current().clone();
        self.large = None;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || decode(&path))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::LargeDecoded { generation, result },
        )
    }

    /// Run `f` against the library (if any), then re-decode the new current.
    fn navigate(&mut self, f: impl FnOnce(&mut Library)) -> Task<Message> {
        if let Some(lib) = &mut self.library {
            f(lib);
        } else {
            return Task::none();
        }
        self.decode_current()
    }

    /// Toggle the current image's favourite flag. Un-favouriting also
    /// un-marks it for deletion (mirrors the Python `action_favourite`). The
    /// decoded image is unchanged, so no re-decode — only the overlay updates.
    fn toggle_favourite(&mut self) {
        let Some(lib) = &mut self.library else {
            return;
        };
        let path = lib.current().clone();
        let result = if lib.favourites.contains(&path) {
            lib.unfavourite(&path).and_then(|()| lib.undelete(&path))
        } else {
            lib.favourite(&path)
        };
        if let Err(e) = result {
            eprintln!("Favourite toggle failed: {e}");
        }
    }

    /// Toggle the current image's delete mark. A favourite can't be marked for
    /// deletion (mirrors the Python `action_delete`).
    fn toggle_delete(&mut self) {
        let Some(lib) = &mut self.library else {
            return;
        };
        let path = lib.current().clone();
        let result = if lib.to_delete.contains(&path) {
            lib.undelete(&path)
        } else if lib.favourites.contains(&path) {
            Ok(()) // refuse: can't delete a favourite
        } else {
            lib.delete(&path)
        };
        if let Err(e) = result {
            eprintln!("Delete toggle failed: {e}");
        }
    }

    /// Unlink every marked file, then re-decode the new current — or fall back
    /// to the empty view if the whole library was deleted.
    fn do_delete_all(&mut self) -> Task<Message> {
        if self.library.is_none() {
            return Task::none();
        }
        let lib = self.library.as_mut().unwrap();
        if let Err(e) = lib.delete_all() {
            eprintln!("Delete-all failed: {e}");
        }
        if lib.paths.is_empty() {
            self.library = None;
            self.large = None;
            Task::none()
        } else {
            self.decode_current()
        }
    }

    fn title(&self) -> String {
        match &self.library {
            Some(lib) => match lib.current().file_name() {
                Some(name) => format!("{} — Photo Viewer", name.to_string_lossy()),
                None => "Photo Viewer".to_string(),
            },
            None => "Photo Viewer".to_string(),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        // While the confirm overlay is up it's modal: swallow everything but
        // the confirm/cancel/quit keys.
        if self.confirm_delete.is_some()
            && !matches!(
                message,
                Message::ConfirmYes | Message::ConfirmNo | Message::Escape | Message::Quit
            )
        {
            return Task::none();
        }

        match message {
            Message::Next => self.navigate(Library::next),
            Message::Prev => self.navigate(Library::prev),
            Message::Quit => iced::exit(),
            Message::ToggleHelp => {
                self.help_open = !self.help_open;
                Task::none()
            }
            Message::Escape => {
                if self.confirm_delete.is_some() {
                    self.confirm_delete = None;
                } else {
                    self.help_open = false;
                }
                Task::none()
            }
            Message::ToggleFullscreen => {
                self.fullscreen = !self.fullscreen;
                let mode = if self.fullscreen {
                    Mode::Fullscreen
                } else {
                    Mode::Windowed
                };
                iced::window::get_latest()
                    .and_then(move |id| iced::window::change_mode(id, mode))
            }
            Message::ToggleFavourite => {
                self.toggle_favourite();
                Task::none()
            }
            Message::ToggleDelete => {
                self.toggle_delete();
                Task::none()
            }
            Message::SaveFavourites => {
                if self.library.is_none() {
                    return Task::none();
                }
                Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Select directory to save favourites")
                            .pick_folder()
                            .await
                            .map(|handle| handle.path().to_path_buf())
                    },
                    Message::SaveFavDirPicked,
                )
            }
            Message::SaveFavDirPicked(Some(dir)) => {
                if let Some(lib) = &self.library {
                    if let Err(e) = lib.save_favourites(&dir) {
                        eprintln!("Save favourites failed: {e}");
                    }
                }
                Task::none()
            }
            Message::SaveFavDirPicked(None) => Task::none(),
            Message::DeleteAll => {
                if let Some(lib) = &self.library {
                    let count = lib.to_delete.len();
                    if count > 0 {
                        self.confirm_delete = Some(count);
                    }
                }
                Task::none()
            }
            Message::ConfirmYes => {
                if self.confirm_delete.take().is_some() {
                    self.do_delete_all()
                } else {
                    Task::none()
                }
            }
            Message::ConfirmNo => {
                self.confirm_delete = None;
                Task::none()
            }
            Message::LargeDecoded { generation, result } => {
                // Drop decodes from a superseded navigation.
                if generation == self.generation {
                    match result {
                        Ok(handle) => self.large = Some(handle),
                        Err(e) => eprintln!("Decode error: {e}"),
                    }
                }
                Task::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        keyboard::on_key_press(|key, modifiers| {
            let cmd = modifiers.command();
            match key.as_ref() {
                keyboard::Key::Named(Named::ArrowRight) => Some(Message::Next),
                keyboard::Key::Named(Named::ArrowLeft) => Some(Message::Prev),
                keyboard::Key::Named(Named::Enter) => Some(Message::ConfirmYes),
                keyboard::Key::Named(Named::Escape) => Some(Message::Escape),
                keyboard::Key::Character("l") => Some(Message::Next),
                keyboard::Key::Character("h") => Some(Message::Prev),
                keyboard::Key::Character("q") => Some(Message::Quit),
                keyboard::Key::Character("e") => Some(Message::ToggleFullscreen),
                keyboard::Key::Character("?") => Some(Message::ToggleHelp),
                keyboard::Key::Character("f") if cmd => Some(Message::SaveFavourites),
                keyboard::Key::Character("f") => Some(Message::ToggleFavourite),
                keyboard::Key::Character("d") if cmd => Some(Message::DeleteAll),
                keyboard::Key::Character("d") => Some(Message::ToggleDelete),
                keyboard::Key::Character("y") => Some(Message::ConfirmYes),
                keyboard::Key::Character("n") => Some(Message::ConfirmNo),
                _ => None,
            }
        })
    }

    fn view(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match &self.library {
            Some(_) => self.single_view(),
            None => empty_view(),
        };

        let mut layers: Vec<Element<'_, Message>> = vec![content];
        if self.help_open {
            layers.push(help_overlay());
        }
        if let Some(count) = self.confirm_delete {
            layers.push(confirm_overlay(count));
        }

        if layers.len() == 1 {
            layers.pop().unwrap()
        } else {
            Stack::with_children(layers).into()
        }
    }

    fn single_view(&self) -> Element<'_, Message> {
        let base: Element<'_, Message> = match &self.large {
            Some(handle) => image(handle.clone())
                .content_fit(ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            None => center(text("Loading\u{2026}").size(20)).into(),
        };

        // Overlay the star (favourite) or delete icon top-right, star winning.
        let Some(lib) = &self.library else {
            return base;
        };
        let path = lib.current();
        let icon = if lib.favourites.contains(path) {
            Some(self.star_icon.clone())
        } else if lib.to_delete.contains(path) {
            Some(self.delete_icon.clone())
        } else {
            None
        };

        match icon {
            Some(handle) => Stack::with_children(vec![base, corner_icon(handle)]).into(),
            None => base,
        }
    }
}

/// An icon pinned to the top-right corner with a fixed margin.
fn corner_icon(handle: image::Handle) -> Element<'static, Message> {
    container(
        image(handle)
            .width(Length::Fixed(ICON_SIZE))
            .height(Length::Fixed(ICON_SIZE)),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Horizontal::Right)
    .align_y(Vertical::Top)
    .padding(ICON_MARGIN)
    .into()
}

/// Decode an image file into an iced RGBA handle. Runs on a blocking thread;
/// the returned handle is plain data, safe to hand back to the GUI thread.
fn decode(path: &std::path::Path) -> Result<image::Handle, String> {
    let img = ::image::open(path).map_err(|e| e.to_string())?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(image::Handle::from_rgba(width, height, rgba.into_raw()))
}

fn empty_view() -> Element<'static, Message> {
    let label = text("No image loaded\nPress ? for help!")
        .size(18)
        .center();
    center(
        container(label)
            .padding(60)
            .style(|theme: &Theme| container::Style {
                border: Border {
                    color: theme.extended_palette().background.strong.color,
                    width: 2.0,
                    radius: 4.0.into(),
                },
                ..container::Style::default()
            }),
    )
    .padding(40)
    .into()
}

/// Shared translucent-panel styling for the help and confirm overlays.
fn overlay_box(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(Color {
            a: 0.96,
            ..palette.background.weak.color
        })),
        border: Border {
            color: palette.background.strong.color,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    }
}

fn help_overlay() -> Element<'static, Message> {
    let title = text("Keyboard Shortcuts").size(24);
    let rows = SHORTCUTS.iter().fold(column![].spacing(10), |col, (keys, desc)| {
        col.push(
            row![
                text(*keys).size(16).width(Length::Fixed(90.0)),
                text(*desc).size(16),
            ]
            .spacing(20),
        )
    });

    center(
        container(column![title, rows].spacing(18))
            .padding(28)
            .style(overlay_box),
    )
    .into()
}

fn confirm_overlay(count: usize) -> Element<'static, Message> {
    let message = if count == 1 {
        "Delete 1 photo?".to_string()
    } else {
        format!("Delete {count} photos?")
    };
    center(
        container(
            column![
                text(message).size(22),
                text("y / Enter — yes      n / Esc — no").size(14),
            ]
            .spacing(18)
            .align_x(Horizontal::Center),
        )
        .padding(28)
        .style(overlay_box),
    )
    .into()
}
