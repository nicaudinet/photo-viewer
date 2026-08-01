//! PhotoViewer — Rust + iced rewrite.
//!
//! Phase 2: single-view MVP. Open a dir or file from argv, fit-to-window the
//! current image (decoded off-thread, generation-tagged so stale decodes are
//! dropped), navigate with `←/→` `h/l`, `q` quit, `e` fullscreen, `?`/`Esc`
//! help. The full roadmap lives in `RUST_REWRITE_PLAN.md`.

// Pure domain core (Phase 1). Later phases consume more of its API; allow the
// still-unused surface (favourite/delete/save_favourites/...) for now.
#[allow(dead_code)]
mod library;
#[allow(dead_code)]
mod pointed_list;

use std::path::{Path, PathBuf};

use iced::keyboard;
use iced::keyboard::key::Named;
use iced::widget::{center, column, container, image, row, stack, text};
use iced::window::Mode;
use iced::{Background, Border, Color, ContentFit, Element, Length, Size, Subscription, Task, Theme};

use library::{load_library, Library};

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
}

#[derive(Debug, Clone)]
enum Message {
    Next,
    Prev,
    Quit,
    ToggleFullscreen,
    ToggleHelp,
    CloseHelp,
    LargeDecoded {
        generation: u64,
        result: Result<image::Handle, String>,
    },
}

/// Shown in the help overlay, in press-order.
const SHORTCUTS: &[(&str, &str)] = &[
    ("\u{2190} / h", "Previous image"),
    ("\u{2192} / l", "Next image"),
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
        match message {
            Message::Next => self.navigate(Library::next),
            Message::Prev => self.navigate(Library::prev),
            Message::Quit => iced::exit(),
            Message::ToggleHelp => {
                self.help_open = !self.help_open;
                Task::none()
            }
            Message::CloseHelp => {
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
                iced::window::get_latest()
                    .and_then(move |id| iced::window::change_mode(id, mode))
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
        keyboard::on_key_press(|key, _modifiers| match key.as_ref() {
            keyboard::Key::Named(Named::ArrowRight) => Some(Message::Next),
            keyboard::Key::Named(Named::ArrowLeft) => Some(Message::Prev),
            keyboard::Key::Named(Named::Escape) => Some(Message::CloseHelp),
            keyboard::Key::Character("l") => Some(Message::Next),
            keyboard::Key::Character("h") => Some(Message::Prev),
            keyboard::Key::Character("q") => Some(Message::Quit),
            keyboard::Key::Character("e") => Some(Message::ToggleFullscreen),
            keyboard::Key::Character("?") => Some(Message::ToggleHelp),
            _ => None,
        })
    }

    fn view(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match &self.library {
            Some(_) => self.single_view(),
            None => empty_view(),
        };
        if self.help_open {
            stack![content, help_overlay()].into()
        } else {
            content
        }
    }

    fn single_view(&self) -> Element<'_, Message> {
        match &self.large {
            Some(handle) => image(handle.clone())
                .content_fit(ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            None => center(text("Loading\u{2026}").size(20)).into(),
        }
    }
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

fn help_overlay() -> Element<'static, Message> {
    let title = text("Keyboard Shortcuts").size(24);
    let rows = SHORTCUTS.iter().fold(
        column![].spacing(10),
        |col, (keys, desc)| {
            col.push(
                row![
                    text(*keys).size(16).width(Length::Fixed(90.0)),
                    text(*desc).size(16),
                ]
                .spacing(20),
            )
        },
    );

    center(
        container(column![title, rows].spacing(18))
            .padding(28)
            .style(|theme: &Theme| {
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
            }),
    )
    .into()
}
