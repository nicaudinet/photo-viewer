//! PhotoViewer — Rust + iced rewrite.
//!
//! Phase 5: rotate, open-dir picker, and the empty view.
//! - Single view: fit-to-window decode, `←/→ h/l` nav, `r`/`Shift+R` rotate
//!   (writes the file to disk), `f`/`d` favourite/delete toggles + icon
//!   overlays, `Cmd+D` delete-all (confirm overlay), `Cmd+F` save-favourites.
//! - Wall view: async 300px thumbnails laid out shortest-column masonry in a
//!   vertical scroll, current image highlighted, click a thumbnail to open it,
//!   `f`/`d` filter to favourites/to-delete only.
//! - `w` toggles between the two; `o` opens a directory (native picker); a
//!   directory opens in wall view, a file in single view.
//! - Empty view when no image is loaded. `q` quit, `e` fullscreen, `?`/`Esc`
//!   help. Roadmap in `RUST_REWRITE_PLAN.md`.

// Pure domain core (Phase 1). Later phases consume more of its API; allow the
// still-unused surface for now.
#[allow(dead_code)]
mod library;
mod platform;
#[allow(dead_code)]
mod pointed_list;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use iced::alignment::{Horizontal, Vertical};
use iced::keyboard;
use iced::keyboard::key::Named;
use iced::widget::{
    button, center, column, container, image, responsive, row, scrollable, text, Column, Row,
    Stack,
};
use iced::window::Mode;
use iced::{
    Background, Border, Color, ContentFit, Element, Length, Shadow, Size, Subscription, Task, Theme,
};

use library::{load_library, Library, IMAGE_EXTENSIONS};

/// Star/delete overlay icons, baked into the binary (no runtime path lookup).
const STAR_ICON: &[u8] = include_bytes!("../icons/star.png");
const DELETE_ICON: &[u8] = include_bytes!("../icons/delete.png");

const ICON_SIZE: f32 = 40.0;
const ICON_MARGIN: f32 = 10.0;

/// Thumbnail column width and inter-item spacing (matches the Python wall).
const THUMB_WIDTH: u32 = 300;
const WALL_SPACING: f32 = 20.0;

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

/// Single-view state: the library plus the current fit-to-window decode and the
/// delete-all confirmation (only reachable from single view).
struct SingleState {
    library: Library,
    /// Latest fit-to-window decode for the current path. `None` until the first
    /// decode lands (the previous image stays on screen meanwhile).
    large: Option<image::Handle>,
    /// `Some(count)` while the delete-all confirmation overlay is showing.
    confirm_delete: Option<usize>,
}

/// Wall-view state: the library plus its thumbnail cache and visibility filter.
struct WallState {
    library: Library,
    /// Decoded thumbnails, keyed by path. Decoded once per wall session.
    thumbs: HashMap<PathBuf, ThumbState>,
    wall_filter: WallFilter,
}

/// Wall-view visibility filter. Toggling one filter off returns to `All`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WallFilter {
    All,
    Favourites,
    ToDelete,
}

/// A decoded thumbnail: the RGBA handle plus its scaled pixel size (needed for
/// masonry column-height bookkeeping).
struct ThumbState {
    handle: image::Handle,
    height: u32,
}

/// The whole model: only screen-independent state lives here. Per-view state
/// lives in the `Screen` variant that needs it.
struct App {
    screen: Screen,
    /// Bumped on every large decode (nav / open / rotate / toggle); tags
    /// `LargeDecoded` so a superseded decode can't overwrite a newer one. Global
    /// and monotonic — kept out of `SingleState` so it can't reset and let a
    /// stale in-flight decode collide with a fresh session's generation.
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
    Next,
    Prev,
    Quit,
    ToggleFullscreen,
    ToggleHelp,
    /// Esc: cancel the confirm overlay if open, else close help.
    Escape,
    /// `f`: favourite toggle (single) or favourites filter (wall).
    KeyF,
    /// `d`: delete toggle (single) or to-delete filter (wall).
    KeyD,
    ToggleWall,
    /// `r`: rotate the current image anticlockwise, writing it to disk.
    RotateAnticlockwise,
    /// `Shift+R`: rotate the current image clockwise, writing it to disk.
    RotateClockwise,
    /// Result of a rotate: on success, re-decode the (now rotated) file.
    Rotated {
        result: Result<(), String>,
    },
    /// `o`: pick a single image file to open (single view).
    OpenFile,
    OpenFilePicked(Option<PathBuf>),
    /// Timer tick: drain any paths the platform delivered (macOS "Open With").
    PollOpenFiles,
    /// The first frame has rendered; reveal the (initially hidden) window.
    WindowReady,
    ThumbClicked(usize),
    ThumbDecoded {
        path: PathBuf,
        result: Result<(image::Handle, u32), String>,
    },
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
    ("w", "Wall / single (toggle)"),
    ("r / \u{21e7}R", "Rotate anticlockwise / clockwise"),
    ("o", "Open file"),
    ("f", "Favourite / favourites filter"),
    ("\u{2318}F", "Save favourites"),
    ("d", "Mark to delete / to-delete filter"),
    ("\u{2318}D", "Delete all marked"),
    ("e", "Fullscreen (toggle)"),
    ("?", "Show help (toggle)"),
    ("Esc", "Close help"),
    ("q", "Quit"),
];

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
            Ok(Some(mut lib)) => {
                match target {
                    // A file: open it directly in single view.
                    Some(file) => {
                        lib.paths.goto_value(&file);
                        self.screen = Screen::Single(SingleState {
                            library: lib,
                            large: None,
                            confirm_delete: None,
                        });
                        self.decode_current()
                    }
                    // A directory: show the wall of all its images.
                    None => {
                        self.screen = Screen::Wall(WallState {
                            library: lib,
                            thumbs: HashMap::new(),
                            wall_filter: WallFilter::All,
                        });
                        self.decode_thumbs()
                    }
                }
            }
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

    /// Kick off an off-thread decode of the current image, tagged with a fresh
    /// generation so an earlier in-flight decode can't overwrite it. No-op off
    /// single view.
    fn decode_current(&mut self) -> Task<Message> {
        let path = match &self.screen {
            Screen::Single(s) => s.library.current().clone(),
            _ => return Task::none(),
        };
        self.generation += 1;
        let generation = self.generation;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || decode_large(&path))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::LargeDecoded { generation, result },
        )
    }

    /// Decode every not-yet-cached thumbnail of the current wall. No-op off wall
    /// view.
    fn decode_thumbs(&self) -> Task<Message> {
        match &self.screen {
            Screen::Wall(w) => w.decode_thumbs(),
            _ => Task::none(),
        }
    }

    /// Run `f` against the single-view library, then re-decode the new current.
    fn navigate(&mut self, f: impl FnOnce(&mut Library)) -> Task<Message> {
        match &mut self.screen {
            Screen::Single(s) => f(&mut s.library),
            _ => return Task::none(),
        }
        self.decode_current()
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
            self.decode_current()
        }
    }

    /// Rotate the current image 90° (clockwise if `clockwise`, else anti-),
    /// writing the result back to its file off-thread. Only in single view.
    fn rotate(&mut self, clockwise: bool) -> Task<Message> {
        let path = match &self.screen {
            Screen::Single(s) => s.library.current().clone(),
            _ => return Task::none(),
        };
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || rotate_file(&path, clockwise))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::Rotated { result },
        )
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
            Message::KeyF => {
                match &mut self.screen {
                    Screen::Single(s) => s.toggle_favourite(),
                    Screen::Wall(w) => {
                        w.wall_filter = toggle_filter(w.wall_filter, WallFilter::Favourites);
                    }
                    Screen::Empty => {}
                }
                Task::none()
            }
            Message::KeyD => {
                match &mut self.screen {
                    Screen::Single(s) => s.toggle_delete(),
                    Screen::Wall(w) => {
                        w.wall_filter = toggle_filter(w.wall_filter, WallFilter::ToDelete);
                    }
                    Screen::Empty => {}
                }
                Task::none()
            }
            // Move the library across to the other view. The losing view's decode
            // cache is dropped, so the new view always decodes fresh.
            Message::ToggleWall => match std::mem::replace(&mut self.screen, Screen::Empty) {
                Screen::Single(s) => {
                    self.screen = Screen::Wall(WallState {
                        library: s.library,
                        thumbs: HashMap::new(),
                        wall_filter: WallFilter::All,
                    });
                    self.decode_thumbs()
                }
                Screen::Wall(w) => {
                    self.screen = Screen::Single(SingleState {
                        library: w.library,
                        large: None,
                        confirm_delete: None,
                    });
                    self.decode_current()
                }
                Screen::Empty => Task::none(),
            },
            Message::RotateAnticlockwise => self.rotate(false),
            Message::RotateClockwise => self.rotate(true),
            Message::Rotated { result } => match result {
                // The file changed on disk: re-decode the current image. (Wall
                // thumbnails are rebuilt on next entry, so none is stale here.)
                Ok(()) => self.decode_current(),
                Err(e) => {
                    eprintln!("Rotate failed: {e}");
                    Task::none()
                }
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
            Message::WindowReady => {
                if self.revealed {
                    return Task::none();
                }
                self.revealed = true;
                iced::window::latest()
                    .and_then(|id| iced::window::set_mode(id, Mode::Windowed))
            }
            Message::ThumbClicked(index) => match std::mem::replace(&mut self.screen, Screen::Empty) {
                Screen::Wall(w) => {
                    let mut library = w.library;
                    library.goto(index);
                    self.screen = Screen::Single(SingleState {
                        library,
                        large: None,
                        confirm_delete: None,
                    });
                    self.decode_current()
                }
                // Only the wall emits `ThumbClicked`; restore anything else.
                other => {
                    self.screen = other;
                    Task::none()
                }
            },
            Message::ThumbDecoded { path, result } => {
                // Dropped if we've since left the wall (the cache went with it).
                if let Screen::Wall(w) = &mut self.screen {
                    match result {
                        Ok((handle, height)) => {
                            w.thumbs.insert(path, ThumbState { handle, height });
                        }
                        Err(e) => eprintln!("Thumbnail decode error: {e}"),
                    }
                }
                Task::none()
            }
            Message::SaveFavourites => {
                if !matches!(&self.screen, Screen::Single(_)) {
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
                if let Screen::Single(s) = &self.screen {
                    if let Err(e) = s.library.save_favourites(&dir) {
                        eprintln!("Save favourites failed: {e}");
                    }
                }
                Task::none()
            }
            Message::SaveFavDirPicked(None) => Task::none(),
            Message::DeleteAll => {
                if let Screen::Single(s) = &mut self.screen {
                    let count = s.library.to_delete.len();
                    if count > 0 {
                        s.confirm_delete = Some(count);
                    }
                }
                Task::none()
            }
            Message::ConfirmYes => {
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
            Message::ConfirmNo => {
                if let Screen::Single(s) = &mut self.screen {
                    s.confirm_delete = None;
                }
                Task::none()
            }
            Message::LargeDecoded { generation, result } => {
                // Generation gate first (global); then apply only if still in
                // single view — a decode that lands after a toggle is dropped.
                if generation == self.generation {
                    if let Screen::Single(s) = &mut self.screen {
                        match result {
                            Ok(handle) => s.large = Some(handle),
                            Err(e) => eprintln!("Decode error: {e}"),
                        }
                    }
                }
                Task::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        // 0.14 unified keyboard subscriptions into a single `listen()` that
        // emits raw `keyboard::Event`s; filter for key-presses ourselves.
        let keys = keyboard::listen().filter_map(|event| {
            let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
                return None;
            };
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
                keyboard::Key::Character("w") => Some(Message::ToggleWall),
                keyboard::Key::Character("o") => Some(Message::OpenFile),
                keyboard::Key::Character("R") => Some(Message::RotateClockwise),
                keyboard::Key::Character("r") if modifiers.shift() => {
                    Some(Message::RotateClockwise)
                }
                keyboard::Key::Character("r") => Some(Message::RotateAnticlockwise),
                keyboard::Key::Character("?") => Some(Message::ToggleHelp),
                keyboard::Key::Character("f") if cmd => Some(Message::SaveFavourites),
                keyboard::Key::Character("f") => Some(Message::KeyF),
                keyboard::Key::Character("d") if cmd => Some(Message::DeleteAll),
                keyboard::Key::Character("d") => Some(Message::KeyD),
                keyboard::Key::Character("y") => Some(Message::ConfirmYes),
                keyboard::Key::Character("n") => Some(Message::ConfirmNo),
                _ => None,
            }
        });

        let mut subs = vec![keys];

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

impl SingleState {
    /// Toggle the current image's favourite flag. Un-favouriting also
    /// un-marks it for deletion. The decoded image is unchanged (overlay only).
    fn toggle_favourite(&mut self) {
        let path = self.library.current().clone();
        let result = if self.library.favourites.contains(&path) {
            self.library
                .unfavourite(&path)
                .and_then(|()| self.library.undelete(&path))
        } else {
            self.library.favourite(&path)
        };
        if let Err(e) = result {
            eprintln!("Favourite toggle failed: {e}");
        }
    }

    /// Toggle the current image's delete mark. A favourite can't be marked.
    fn toggle_delete(&mut self) {
        let path = self.library.current().clone();
        let result = if self.library.to_delete.contains(&path) {
            self.library.undelete(&path)
        } else if self.library.favourites.contains(&path) {
            Ok(()) // refuse: can't delete a favourite
        } else {
            self.library.delete(&path)
        };
        if let Err(e) = result {
            eprintln!("Delete toggle failed: {e}");
        }
    }

    fn view<'a>(
        &'a self,
        star_icon: &'a image::Handle,
        delete_icon: &'a image::Handle,
    ) -> Element<'a, Message> {
        let base: Element<'a, Message> = match &self.large {
            Some(handle) => image(handle.clone())
                .content_fit(ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            None => iced::widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        };

        let path = self.library.current();
        let icon = if self.library.favourites.contains(path) {
            Some(star_icon.clone())
        } else if self.library.to_delete.contains(path) {
            Some(delete_icon.clone())
        } else {
            None
        };

        match icon {
            Some(handle) => Stack::with_children(vec![base, corner_icon(handle)]).into(),
            None => base,
        }
    }
}

impl WallState {
    /// Decode (once) every thumbnail not already cached.
    fn decode_thumbs(&self) -> Task<Message> {
        let tasks: Vec<Task<Message>> = self
            .library
            .paths
            .iter()
            .filter(|p| !self.thumbs.contains_key(*p))
            .map(|p| {
                let path = p.clone();
                let key = p.clone();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || decode_thumb(&path))
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    move |result| Message::ThumbDecoded {
                        path: key.clone(),
                        result,
                    },
                )
            })
            .collect();
        Task::batch(tasks)
    }

    fn view<'a>(
        &'a self,
        star_icon: &'a image::Handle,
        delete_icon: &'a image::Handle,
    ) -> Element<'a, Message> {
        responsive(move |size| self.build_wall(size, star_icon, delete_icon)).into()
    }

    /// Lay the (filtered) thumbnails out shortest-column masonry for `size`.
    fn build_wall<'a>(
        &'a self,
        size: Size,
        star_icon: &'a image::Handle,
        delete_icon: &'a image::Handle,
    ) -> Element<'a, Message> {
        let current = self.library.paths.index();

        let items: Vec<(usize, &PathBuf)> = self
            .library
            .paths
            .iter()
            .enumerate()
            .filter(|(_, p)| match self.wall_filter {
                WallFilter::All => true,
                WallFilter::Favourites => self.library.favourites.contains(*p),
                WallFilter::ToDelete => self.library.to_delete.contains(*p),
            })
            .collect();

        let item_width = WALL_SPACING + THUMB_WIDTH as f32;
        let col_count = (((size.width - WALL_SPACING) / item_width).floor() as usize).max(1);

        let mut buckets: Vec<Vec<Element<'a, Message>>> =
            (0..col_count).map(|_| Vec::new()).collect();
        let mut heights = vec![0.0_f32; col_count];

        for (index, path) in items {
            // Shortest-column placement (matches the Python masonry).
            let col = heights
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .map(|(i, _)| i)
                .unwrap_or(0);
            let thumb_height = self.thumbs.get(path).map(|t| t.height as f32).unwrap_or(THUMB_WIDTH as f32);
            buckets[col].push(self.thumb_element(index, path, current, star_icon, delete_icon));
            heights[col] += thumb_height + WALL_SPACING;
        }

        let columns: Vec<Element<'a, Message>> = buckets
            .into_iter()
            .map(|items| {
                Column::with_children(items)
                    .spacing(WALL_SPACING)
                    .width(Length::Fixed(THUMB_WIDTH as f32))
                    .into()
            })
            .collect();

        let grid = Row::with_children(columns).spacing(WALL_SPACING);
        let centered = container(grid)
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding(WALL_SPACING);
        scrollable(centered).height(Length::Fill).into()
    }

    /// One thumbnail: image (or placeholder) + icon overlay, clickable, with a
    /// highlight border when it is the current image. Icons are hidden while a
    /// filter is active (the filter already conveys the status).
    fn thumb_element<'a>(
        &'a self,
        index: usize,
        path: &'a PathBuf,
        current: usize,
        star_icon: &'a image::Handle,
        delete_icon: &'a image::Handle,
    ) -> Element<'a, Message> {
        let inner: Element<'a, Message> = match self.thumbs.get(path) {
            Some(thumb) => image(thumb.handle.clone())
                .width(Length::Fixed(THUMB_WIDTH as f32))
                .height(Length::Fixed(thumb.height as f32))
                .into(),
            None => container(text("Loading\u{2026}").size(14))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .width(Length::Fixed(THUMB_WIDTH as f32))
                .height(Length::Fixed(THUMB_WIDTH as f32))
                .style(placeholder_style)
                .into(),
        };

        let show_icons = self.wall_filter == WallFilter::All;
        let icon = if show_icons && self.library.favourites.contains(path) {
            Some(star_icon.clone())
        } else if show_icons && self.library.to_delete.contains(path) {
            Some(delete_icon.clone())
        } else {
            None
        };

        let body: Element<'a, Message> = match icon {
            Some(handle) => Stack::with_children(vec![inner, corner_icon(handle)]).into(),
            None => inner,
        };

        let selected = index == current;
        button(body)
            .padding(0)
            .on_press(Message::ThumbClicked(index))
            .style(move |theme: &Theme, _status| thumb_button_style(theme, selected))
            .into()
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

/// Rotate the image at `path` 90° and overwrite it, preserving its format (the
/// destination extension picks the encoder). `clockwise` matches `Shift+R`; the
/// anticlockwise case matches the Python `Image.rotate(90)`. Runs off-thread.
fn rotate_file(path: &Path, clockwise: bool) -> Result<(), String> {
    let img = ::image::open(path).map_err(|e| e.to_string())?;
    let rotated = if clockwise {
        img.rotate90()
    } else {
        img.rotate270()
    };
    rotated.save(path).map_err(|e| e.to_string())
}

/// Decode + fit an image to window size later; here just full-res RGBA. Runs on
/// a blocking thread; the returned handle is plain data, safe on the GUI thread.
fn decode_large(path: &Path) -> Result<image::Handle, String> {
    let img = ::image::open(path).map_err(|e| e.to_string())?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(image::Handle::from_rgba(width, height, rgba.into_raw()))
}

/// Decode + downscale to a `THUMB_WIDTH`-wide thumbnail. Returns the handle and
/// its scaled height (for masonry column bookkeeping).
fn decode_thumb(path: &Path) -> Result<(image::Handle, u32), String> {
    use ::image::GenericImageView;
    let img = ::image::open(path).map_err(|e| e.to_string())?;
    let (w, h) = img.dimensions();
    let target_h = (((h as f32) / (w as f32)) * THUMB_WIDTH as f32).round().max(1.0) as u32;
    let resized = img.resize(THUMB_WIDTH, target_h, ::image::imageops::FilterType::Triangle);
    let rgba = resized.to_rgba8();
    let (rw, rh) = rgba.dimensions();
    Ok((image::Handle::from_rgba(rw, rh, rgba.into_raw()), rh))
}

/// Toggle `filter` on: pressing its key again (when already active) returns to
/// `All`, otherwise it becomes the sole active filter.
fn toggle_filter(current: WallFilter, filter: WallFilter) -> WallFilter {
    if current == filter {
        WallFilter::All
    } else {
        filter
    }
}

fn thumb_button_style(theme: &Theme, selected: bool) -> button::Style {
    let palette = theme.extended_palette();
    button::Style {
        background: None,
        text_color: palette.background.base.text,
        border: if selected {
            Border {
                color: palette.primary.strong.color,
                width: 4.0,
                radius: 0.0.into(),
            }
        } else {
            Border::default()
        },
        shadow: Shadow::default(),
        // 0.14 added pixel-grid snapping; keep the non-crisp default.
        snap: false,
    }
}

fn placeholder_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(palette.background.weak.color)),
        ..container::Style::default()
    }
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
