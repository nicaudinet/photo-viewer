//! PhotoViewer — Rust + iced rewrite.
//!
//! - Single view: fit-to-window decode, `←/→ h/l` nav, `r`/`Shift+R` rotate
//!   (writes the file to disk). Every action applies to the current image and
//!   nothing else.
//! - Wall view: async 300px thumbnails laid out shortest-column masonry in a
//!   vertical scroll, current image ringed, `←/→/↑/↓ hjkl` to move the ring
//!   (scrolling it into view), Enter or a click to open it, `r`/`Shift+R` to
//!   rotate it on disk, and a modal selection (`v`/`x`/`Space`, or the mouse)
//!   over groups of images, which can then be rotated, sent to another folder
//!   (`m`/`c`) or trashed (`d`) all at once.
//! - `w` toggles between the two; `o` opens a directory (native picker); a
//!   directory opens in wall view, a file in single view.
//! - Empty view when no image is loaded. `q` quit, `e` fullscreen, `?`/`Esc`
//!   help.
//!
//! Favourites and mark-to-delete were removed in phase 0 of
//! `SELECT_MODE_PLAN.md`; selection, and the batch operations that replace
//! them, land in the phases after it.
//!
//! `App` here owns only screen-independent state and the transitions between
//! screens; each screen's own state, actions, and view live in `gui/`.

mod exif;
mod library;
mod imaging;
mod platform;
// `PointedList` keeps a complete list API (`delete`, `contains`, …) that the
// selection phases will consume; allow the still-unused surface for now.
#[allow(dead_code)]
mod pointed_list;
mod transfer;

mod gui;

use std::path::{Path, PathBuf};
use std::time::Duration;

use iced::keyboard;
use iced::keyboard::key::Named;
use iced::widget::Stack;
use iced::window::Mode;
use iced::{Element, Size, Subscription, Task, Theme};

use gui::empty::empty_view;
use gui::single::{SingleMsg, SingleState};
use gui::wall::{BatchKind, Click, Dir, WallMsg, WallState};
use gui::{confirm_overlay, help_overlay};
use library::{load_library, Library, RangeOp, IMAGE_EXTENSIONS};
use transfer::{Collision, TransferKind};

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
// The wall's state is the larger by some way, but exactly one `Screen` exists
// for the life of the process — boxing it would buy an indirection on every
// access and save nothing.
#[allow(clippy::large_enum_variant)]
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

/// A question waiting on the user, and what each answer does.
struct Confirm {
    prompt: String,
    /// Anything that has to be known before answering — a name clash, or the
    /// fact that moved files will drop off the wall.
    detail: Option<String>,
    /// The answers on offer, in order. Enter picks the first, so that one must
    /// always be the safe reading of the question. `n` and Esc always cancel,
    /// so no answer may claim either key; nor `q`, which still quits.
    choices: Vec<Choice>,
}

struct Choice {
    key: char,
    label: &'static str,
    action: ConfirmAction,
}

impl Confirm {
    /// The keys line under the question. Every way out is named: an answer the
    /// overlay does not mention is an answer nobody can give.
    fn hint(&self) -> String {
        let mut parts: Vec<String> = self
            .choices
            .iter()
            .enumerate()
            .map(|(i, c)| match i {
                0 => format!("{} / \u{21b5} \u{2014} {}", c.key, c.label),
                _ => format!("{} \u{2014} {}", c.key, c.label),
            })
            .collect();
        parts.push("n / Esc \u{2014} cancel".to_string());
        parts.join("      ")
    }
}

enum ConfirmAction {
    /// Move every selected image to the system trash.
    DeleteSelected(Vec<PathBuf>),
    /// Move or copy the selection into the folder the user picked.
    Transfer {
        plan: TransferPlan,
        collision: Collision,
    },
}

/// A move or copy that has been aimed at a folder but not yet agreed to.
///
/// Everything the question needs is gathered before it is asked — including a
/// look at the destination — so the user is told the real number of name
/// clashes once, up front, rather than being interrupted per file.
#[derive(Debug, Clone)]
struct TransferPlan {
    kind: TransferKind,
    dest: PathBuf,
    paths: Vec<PathBuf>,
    /// How many of `paths` already have a namesake in `dest`.
    collisions: usize,
    /// The destination is the folder the images are already in.
    same_dir: bool,
    /// The destination sits inside the library's folder. The scan is not
    /// recursive, so moved images vanish from the wall — correct, but worth
    /// saying before it happens rather than after.
    inside_library: bool,
}

#[derive(Debug, Clone)]
enum Message {
    // Global — handled at the top level regardless of screen.
    Quit,
    ToggleFullscreen,
    ToggleHelp,
    /// Esc: close the help overlay.
    Escape,
    /// The first frame has rendered; reveal the (initially hidden) window.
    WindowReady,
    /// A modifier key went down or up; remembered for the next click.
    ModifiersChanged(keyboard::Modifiers),

    // Ambiguous shared keys: same key, different meaning per screen. The live
    // screen isn't visible to `subscription`, so these stay neutral here and are
    // dispatched to the current screen's update in `App::update`.
    /// Arrows / `hjkl`: previous-next (single) or grid movement (wall).
    Nav(Dir),
    /// `r` / `Shift+R`: rotate the current image (single) or the selected
    /// thumbnail (wall). Both write the file to disk.
    Rotate {
        clockwise: bool,
    },
    /// Enter: commit a painted range, else open the selected thumbnail (wall
    /// only).
    Activate,

    // Selection keys. Meaningful only on the wall, but the subscription can't
    // see which screen is live, so they are dispatched in `App::update`.
    /// `v` / `x`: paint a range that adds to / removes from the selection.
    Visual {
        op: RangeOp,
    },
    /// `Space`: select or deselect the image under the cursor.
    ToggleSelected,
    /// `Cmd+A`.
    SelectAll,
    /// `i`.
    InvertSelection,
    /// `d`: ask before trashing the selection.
    DeleteSelected,
    /// `m` / `c`: send the selection to another folder. Opens the folder picker
    /// first; nothing is written until the question that follows is answered.
    Transfer {
        kind: TransferKind,
    },
    /// The folder picker closed, and the destination has been looked at.
    /// `None` if the picker was cancelled.
    TransferTarget(Option<TransferPlan>),
    /// Images have left the library's folder — trashed, or moved elsewhere.
    /// Carries what actually went, so a failure leaves its image on the wall.
    Removed {
        gone: Vec<PathBuf>,
        failed: Vec<(PathBuf, String)>,
    },
    /// A key naming one of the answers on offer.
    ConfirmChoice(char),
    /// `n` — decline, whatever was asked.
    ConfirmNo,
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

    // Delegated to the current screen's own update.
    Single(SingleMsg),
    Wall(WallMsg),
}

/// Move each path to the system trash, one at a time so a single stubborn file
/// doesn't take the rest of the batch down with it. Returns what actually left
/// the disk and what didn't.
///
/// The trash, not `unlink`: there is no undo in this app, and a mistaken
/// selection of a hundred photos should be a nuisance rather than a
/// catastrophe.
fn trash_all(paths: Vec<PathBuf>) -> (Vec<PathBuf>, Vec<(PathBuf, String)>) {
    let mut gone = Vec::new();
    let mut failed = Vec::new();
    for path in paths {
        match trash::delete(&path) {
            Ok(()) => gone.push(path),
            // Already missing: the file is gone either way, which is all the
            // library cares about.
            Err(_) if !path.exists() => gone.push(path),
            Err(e) => failed.push((path, e.to_string())),
        }
    }
    (gone, failed)
}

/// Ask for a destination folder, then look at what is already in it.
///
/// The whole question is assembled before it is put to the user: how many files
/// would land on a namesake, and whether the folder is one the wall is showing.
/// Asking once, up front, beats interrupting a running batch per clash.
async fn pick_destination(
    kind: TransferKind,
    paths: Vec<PathBuf>,
    image_dir: PathBuf,
) -> Option<TransferPlan> {
    let title = match kind {
        TransferKind::Move => "Move photos to\u{2026}",
        TransferKind::Copy => "Copy photos to\u{2026}",
    };
    let dest = rfd::AsyncFileDialog::new()
        .set_title(title)
        .set_directory(&image_dir)
        .pick_folder()
        .await?
        .path()
        .to_path_buf();

    // One `stat` per file: off the GUI thread, since a big selection over a
    // network share is not instant.
    tokio::task::spawn_blocking(move || {
        let collisions = transfer::collisions(&paths, &dest);
        let same_dir = dest == image_dir;
        TransferPlan {
            kind,
            collisions,
            same_dir,
            inside_library: !same_dir && dest.starts_with(&image_dir),
            dest,
            paths,
        }
    })
    .await
    .ok()
}

/// A folder's own name, for the question — the full path is usually far too
/// long to read at a glance. Falls back to the path for a root directory.
fn folder_name(dir: &Path) -> String {
    match dir.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => dir.display().to_string(),
    }
}

impl App {
    fn new() -> (App, Task<Message>) {
        let mut app = App {
            screen: Screen::Empty,
            generation: 0,
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

    /// Carry out the answer keyed `key`, or the first one on offer if `key` is
    /// `None` (what Enter means). An unrecognised key leaves the question up.
    fn answer(&mut self, key: Option<char>) -> Task<Message> {
        let Some(confirm) = &self.confirm else {
            return Task::none();
        };
        let found = match key {
            Some(key) => confirm.choices.iter().position(|c| c.key == key),
            // Enter takes the first answer, which is always the safe one.
            None => (!confirm.choices.is_empty()).then_some(0),
        };
        let Some(index) = found else {
            // A question with no answers on offer is a statement; Enter
            // dismisses it, an unrecognised key leaves it up.
            if key.is_none() {
                self.confirm = None;
            }
            return Task::none();
        };
        let mut confirm = self.confirm.take().expect("checked just above");
        self.act(confirm.choices.swap_remove(index).action)
    }

    fn act(&mut self, action: ConfirmAction) -> Task<Message> {
        match action {
            ConfirmAction::DeleteSelected(paths) => Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || trash_all(paths))
                        .await
                        .unwrap_or_else(|e| (Vec::new(), vec![(PathBuf::new(), e.to_string())]))
                },
                |(gone, failed)| Message::Removed { gone, failed },
            ),
            ConfirmAction::Transfer { plan, collision } => self.wall_msg(WallMsg::StartBatch {
                kind: BatchKind::Transfer {
                    kind: plan.kind,
                    dest: plan.dest,
                    collision,
                },
                paths: plan.paths,
            }),
        }
    }

    /// Turn a destination the user picked into the question to ask about it.
    fn ask_about(&mut self, plan: TransferPlan) {
        let folder = folder_name(&plan.dest);
        if plan.same_dir {
            // Nothing to do, and every file would "clash" with itself — so this
            // is a statement rather than a question.
            self.confirm = Some(Confirm {
                prompt: format!("Those photos are already in {folder}."),
                detail: None,
                choices: Vec::new(),
            });
            return;
        }

        let count = plan.paths.len();
        let noun = if count == 1 { "photo" } else { "photos" };
        let prompt = format!("{} {count} {noun} to {folder}?", plan.kind.word());

        let mut detail = Vec::new();
        if plan.collisions > 0 {
            let (n, verb) = (
                plan.collisions,
                if plan.collisions == 1 { "is" } else { "are" },
            );
            detail.push(format!("{n} of them {verb} already there."));
        }
        if plan.inside_library && plan.kind == TransferKind::Move {
            detail.push("They will leave the wall: the folder scan is not recursive.".to_string());
        }

        // With clashes there is no honest yes/no: the answer *is* the policy,
        // chosen once and applied to every file, so nothing is written before
        // the user has said what should happen to the ones already there.
        let choices = if plan.collisions > 0 {
            vec![
                Choice {
                    key: 's',
                    label: "skip those",
                    action: ConfirmAction::Transfer {
                        plan: plan.clone(),
                        collision: Collision::Skip,
                    },
                },
                Choice {
                    key: 'k',
                    label: "keep both",
                    action: ConfirmAction::Transfer {
                        plan: plan.clone(),
                        collision: Collision::KeepBoth,
                    },
                },
                Choice {
                    key: 'o',
                    label: "overwrite",
                    action: ConfirmAction::Transfer {
                        plan,
                        collision: Collision::Overwrite,
                    },
                },
            ]
        } else {
            vec![Choice {
                key: 'y',
                label: "yes",
                action: ConfirmAction::Transfer {
                    plan,
                    collision: Collision::Skip,
                },
            }]
        };

        self.confirm = Some(Confirm {
            prompt,
            detail: (!detail.is_empty()).then(|| detail.join("\n")),
            choices,
        });
    }

    /// Hand a message to the wall, or drop it if the wall isn't on screen.
    /// Selection lives on the wall alone: the single view has no selection to
    /// show, so acting on one from there could only ever surprise.
    fn wall_msg(&mut self, msg: WallMsg) -> Task<Message> {
        match &mut self.screen {
            Screen::Wall(w) => w.update(msg),
            _ => Task::none(),
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

            Message::Visual { op } => self.wall_msg(WallMsg::EnterVisual { op }),
            Message::ToggleSelected => self.wall_msg(WallMsg::ToggleCursor),
            Message::SelectAll => self.wall_msg(WallMsg::SelectAll),
            Message::InvertSelection => self.wall_msg(WallMsg::InvertSelection),

            Message::DeleteSelected => {
                let Screen::Wall(w) = &self.screen else {
                    return Task::none();
                };
                let Some(selected) = w.operable_selection() else {
                    return Task::none();
                };
                let prompt = match selected.len() {
                    1 => "Move 1 photo to the trash?".to_string(),
                    n => format!("Move {n} photos to the trash?"),
                };
                self.confirm = Some(Confirm {
                    prompt,
                    detail: None,
                    choices: vec![Choice {
                        key: 'y',
                        label: "yes",
                        action: ConfirmAction::DeleteSelected(selected),
                    }],
                });
                Task::none()
            }
            Message::Transfer { kind } => {
                let Screen::Wall(w) = &self.screen else {
                    return Task::none();
                };
                let Some(selected) = w.operable_selection() else {
                    return Task::none();
                };
                let image_dir = w.library.image_dir.clone();
                Task::perform(
                    pick_destination(kind, selected, image_dir),
                    Message::TransferTarget,
                )
            }
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
            Message::Removed { gone, failed } => {
                for (path, error) in &failed {
                    eprintln!("Could not trash {}: {error}", path.display());
                }
                let gone: std::collections::HashSet<PathBuf> = gone.into_iter().collect();
                if gone.is_empty() {
                    return Task::none();
                }
                match &mut self.screen {
                    Screen::Wall(w) => match w.removed(&gone) {
                        (true, task) => task,
                        // Everything is gone: there is no library left to show.
                        (false, _) => {
                            self.screen = Screen::Empty;
                            Task::none()
                        }
                    },
                    _ => Task::none(),
                }
            }
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

    fn subscription(&self) -> Subscription<Message> {
        // Two keyboards, one live at a time. A question on screen names its own
        // answers, and those can be any letter, so the ordinary bindings would
        // fight them (`o` opens a folder, `r` rotates). Swapping the whole map
        // is what makes the overlay genuinely modal.
        //
        // iced requires these closures to capture nothing, so the choice is
        // made out here rather than inside one of them.
        let keys = if self.confirm.is_some() {
            keyboard::listen().filter_map(answer_key)
        } else {
            keyboard::listen().filter_map(normal_key)
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

    fn view(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match &self.screen {
            Screen::Empty => empty_view(),
            Screen::Single(s) => s.view(),
            Screen::Wall(w) => w.view(),
        };

        let mut layers: Vec<Element<'_, Message>> = vec![content];
        if self.help_open {
            layers.push(help_overlay());
        }
        if let Some(confirm) = &self.confirm {
            layers.push(confirm_overlay(
                &confirm.prompt,
                confirm.detail.as_deref(),
                &confirm.hint(),
            ));
        }

        if layers.len() == 1 {
            layers.pop().unwrap()
        } else {
            Stack::with_children(layers).into()
        }
    }
}

/// The keyboard while a question is up: the keys it names, and nothing else.
fn answer_key(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, .. } = event else {
        return None;
    };
    match key.as_ref() {
        keyboard::Key::Named(Named::Enter) => Some(Message::Activate),
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

/// The ordinary keyboard. 0.14 unified keyboard subscriptions into a single
/// `listen()` that emits raw `keyboard::Event`s; filter for key-presses here.
fn normal_key(event: keyboard::Event) -> Option<Message> {
    // Clicks read their modifiers from `App::modifiers`, since a `button` press
    // carries none of its own.
    if let keyboard::Event::ModifiersChanged(modifiers) = event {
        return Some(Message::ModifiersChanged(modifiers));
    }
    let keyboard::Event::KeyPressed {
        key,
        modified_key,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    match key.as_ref() {
        keyboard::Key::Named(Named::ArrowRight) => Some(Message::Nav(Dir::Right)),
        keyboard::Key::Named(Named::ArrowLeft) => Some(Message::Nav(Dir::Left)),
        keyboard::Key::Named(Named::ArrowUp) => Some(Message::Nav(Dir::Up)),
        keyboard::Key::Named(Named::ArrowDown) => Some(Message::Nav(Dir::Down)),
        keyboard::Key::Named(Named::Enter) => Some(Message::Activate),
        keyboard::Key::Named(Named::Escape) => Some(Message::Escape),
        keyboard::Key::Named(Named::Space) => Some(Message::ToggleSelected),
        keyboard::Key::Character("l") => Some(Message::Nav(Dir::Right)),
        keyboard::Key::Character("h") => Some(Message::Nav(Dir::Left)),
        keyboard::Key::Character("k") => Some(Message::Nav(Dir::Up)),
        keyboard::Key::Character("j") => Some(Message::Nav(Dir::Down)),
        keyboard::Key::Character("v") => Some(Message::Visual { op: RangeOp::Add }),
        keyboard::Key::Character("x") => Some(Message::Visual {
            op: RangeOp::Remove,
        }),
        keyboard::Key::Character("i") => Some(Message::InvertSelection),
        keyboard::Key::Character("d") => Some(Message::DeleteSelected),
        keyboard::Key::Character("m") => Some(Message::Transfer {
            kind: TransferKind::Move,
        }),
        keyboard::Key::Character("c") => Some(Message::Transfer {
            kind: TransferKind::Copy,
        }),
        keyboard::Key::Character("a") if modifiers.command() => Some(Message::SelectAll),
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
        _ if modified_key.as_ref() == keyboard::Key::Character("?") => Some(Message::ToggleHelp),
        _ => None,
    }
}
