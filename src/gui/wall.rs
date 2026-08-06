//! The wall view: async thumbnails laid out shortest-column masonry, with
//! keyboard navigation, rotate, click-to-open, a modal selection, and the
//! operations over it — favourite, move, copy, trash.
//!
//! Selection is vim-shaped: [`WallMode::Normal`] moves a cursor,
//! [`WallMode::Visual`] paints a range as the cursor moves, and
//! [`WallMode::Select`] holds the committed set. See `SELECT_MODE_PLAN.md`.
//!
//! Layout runs in two phases. [`WallState::layout`] is a pure function from
//! (viewport width, thumbnail heights) to a [`WallLayout`] — *where*
//! each thumbnail sits, as plain data. The view turns that into widgets;
//! `update` reads it to answer "what is to the left of the current image?".
//! Both derive from the same function over the same state, so keyboard
//! navigation always agrees with what is on screen.
//!
//! The viewport width phase one needs only exists once iced has laid the tree
//! out, so it is read back with a widget [`Operation`] (see [`measure`]) and
//! stored by `update`. The view never writes state.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iced::advanced::widget::operation::scrollable::Scrollable;
use iced::advanced::widget::operation::Outcome;
use iced::advanced::widget::{operate, Id, Operation};
use iced::theme::palette::lighten;
use iced::widget::operation::{scroll_to, AbsoluteOffset};
use iced::alignment::{Horizontal, Vertical};
use iced::keyboard::Modifiers;
use iced::widget::{button, container, image, scrollable, text, Column, Row, Space, Stack};
use iced::{
    Background, Border, Color, Element, Length, Rectangle, Shadow, Size, Task, Theme, Vector,
};

use super::ICON_MARGIN;
use crate::library::{Library, RangeOp};
use crate::tags;
use crate::transfer::{self, Collision, TransferKind, Transferred};
use crate::Message;

/// Thumbnail column width and inter-item spacing (matches the Python wall).
const THUMB_WIDTH: u32 = 300;
/// Inter-tile spacing, and the outer padding of the wall (the layout maths in
/// [`WallState::layout`] assumes the two are equal).
const WALL_SPACING: f32 = 12.0;
/// Width of the selection ring drawn around the current thumbnail.
///
/// `button` fills its border quad at its full bounds and *then* draws the
/// content on top, so a zero-padding button hides its own border behind an
/// opaque child. Every thumbnail therefore carries this much padding — the ring
/// is only made visible (rather than added) when the thumbnail is selected, so
/// selection never reflows the wall.
const SEL_BORDER: f32 = 4.0;
/// A thumbnail plus its selection ring: what the masonry actually places.
const TILE_WIDTH: f32 = THUMB_WIDTH as f32 + 2.0 * SEL_BORDER;
/// How far an accent ring is lifted off its palette colour (OKLCH lightness).
/// On the dark theme this takes the primary `#5865F2` to `#8B8BFF`.
const SEL_LIGHTEN: f32 = 0.25;
/// Tint alpha over a committed selection, and over a range still being painted.
/// The pending one is lighter so an uncommitted range never looks decided.
const TINT_COMMITTED: f32 = 0.32;
const TINT_PENDING: f32 = 0.18;
/// Height of the mode bar along the bottom (hidden in `Normal`).
const BAR_HEIGHT: f32 = 34.0;
/// How close together two clicks on the same thumbnail count as a double
/// click. macOS's own default threshold is 500ms; this is a little tighter so
/// two deliberate toggles of the same tile aren't read as one.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Identifies the wall's scrollable so [`measure`] and [`WallState::reveal`]
/// can target it.
const WALL_ID: Id = Id::new("wall");

/// Max thumbnail decodes running at once. A wall of N images no longer fires N
/// tasks up front; the scheduler keeps at most this many in flight and refills
/// as each lands (see [`WallState::schedule`]). Sized to the CPU count, since
/// decode is CPU-bound; bounding it caps thrash and peak memory.
fn max_in_flight() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    })
}

/// A keyboard navigation direction, shared with the single view (where left and
/// right mean previous and next).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// Which selection mode the wall is in.
///
/// There is exactly one cursor — `library.paths.index()` — and it moves in
/// every mode. `Visual` adds only an anchor; the moving end of the painted
/// range *is* the cursor, which is why navigation needs no special case for it.
///
/// Invariant, restored by [`WallState::settle`] after every selection change:
/// outside `Visual`, the mode is `Select` exactly when the selection is
/// non-empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WallMode {
    Normal,
    Visual { anchor: usize, op: RangeOp },
    Select,
}

/// Messages produced only while the wall view is on screen. Routed to
/// [`WallState::update`] by `App::update`.
#[derive(Debug, Clone)]
pub(crate) enum WallMsg {
    /// The size of the wall's scroll viewport, read back from the laid-out
    /// widget tree by [`measure`].
    Measured(Size),
    /// Header-derived thumbnail heights for the whole library.
    RatiosLoaded(Vec<(PathBuf, f32)>),
    ThumbDecoded {
        path: PathBuf,
        result: Result<(image::Handle, u32), String>,
    },
    /// The wall scrolled; carries the absolute vertical offset in pixels so the
    /// scheduler can prioritise thumbnails near the viewport.
    Scrolled(f32),
    /// Move the selection one thumbnail in `Dir`.
    Nav(Dir),
    /// `r` / `Shift+R`: rotate the selected image 90°, writing it to disk.
    Rotate { clockwise: bool },
    /// Result of a rotate. Carries its own path: the selection may have moved
    /// while the write was in flight.
    Rotated {
        path: PathBuf,
        result: Result<(), String>,
    },
    /// `v` / `x`: start painting a range from the cursor — or, pressed while
    /// already painting one, cancel it.
    EnterVisual { op: RangeOp },
    /// Enter: fold the painted range into the selection.
    CommitVisual,
    /// `Space`: add or remove the single image under the cursor.
    ToggleCursor,
    /// `Cmd+A` / `i`.
    SelectAll,
    InvertSelection,
    /// `f`: favourite the selection, or the image under the cursor.
    ToggleFavourite,
    /// `Shift+F`: show only the favourites, or show everything again.
    ToggleFilter,
    /// Esc: one rung down the ladder (cancel a running batch, then the painted
    /// range, then the selection).
    Escape,
    /// Run an operation over `paths`, which the caller has already confirmed.
    ///
    /// The files come with the message rather than being re-read from the
    /// selection: the question the user answered named a count, and that count
    /// has to be what actually happens.
    StartBatch {
        kind: BatchKind,
        paths: Vec<PathBuf>,
    },
    /// One file of a batch finished.
    BatchProgress {
        path: PathBuf,
        result: Result<FileDone, String>,
    },
}

/// What one finished file means for the wall — the only part of an operation's
/// result the view has to care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileDone {
    /// It changed shape on disk, so its thumbnail and height are stale.
    Reshaped,
    /// It is no longer in the library's folder.
    Gone,
    /// Nothing the wall shows has changed.
    Unchanged,
}

/// A decoded thumbnail: the RGBA handle plus its scaled pixel size (needed for
/// masonry column-height bookkeeping).
struct ThumbState {
    handle: image::Handle,
    height: u32,
}

/// Where the masonry put one thumbnail. `y` is absolute within the scrollable's
/// content, so it can be compared against the scroll offset directly.
struct Slot {
    col: usize,
    row: usize,
    y: f32,
    height: f32,
}

/// The result of one masonry pass: pure data, no widgets.
struct WallLayout {
    /// Per column, the library indices it holds, top to bottom.
    columns: Vec<Vec<usize>>,
    /// Library index -> where it sits.
    slots: HashMap<usize, Slot>,
    /// Library indices in library order — the space decode priority is
    /// expressed in.
    order: Vec<usize>,
    /// Total scrollable content height, including the outer padding.
    content_height: f32,
}

/// Wall-view state: the library plus its thumbnail cache and measurements.
/// Every field is written only by [`WallState::update`].
pub(crate) struct WallState {
    pub(crate) library: Library,
    /// Decoded thumbnails, keyed by path. Decoded once per wall session.
    thumbs: HashMap<PathBuf, ThumbState>,
    /// Header-derived thumbnail heights, known before any decode lands so the
    /// masonry settles once instead of reshuffling under the user.
    ratios: HashMap<PathBuf, f32>,
    /// Paths whose decode has been dispatched but not yet landed. Bounds
    /// concurrency (`len <= max_in_flight`) and stops double-dispatching.
    in_flight: HashSet<PathBuf>,
    /// Paths rotated while their decode was in flight: that decode holds
    /// pre-rotation pixels, so it is discarded on arrival rather than cached.
    stale: HashSet<PathBuf>,
    /// Paths with a rotate write in flight. Holding the key down would
    /// otherwise race two read-modify-writes against the same file.
    rotating: HashSet<PathBuf>,
    /// Size of the scroll viewport. `None` until the first [`measure`] lands —
    /// the wall renders an empty scrollable until then.
    viewport: Option<Size>,
    /// Absolute vertical scroll offset in pixels.
    scroll_y: f32,
    /// Position (in [`WallLayout::order`]) of the thumbnail nearest the middle
    /// of the viewport. Cached rather than recomputed per decode, so filling a
    /// large wall stays linear overall.
    focus: f32,
    /// Sticky vertical centre for runs of left/right moves, so `h` then `l`
    /// returns to where it started instead of drifting.
    desired_y: Option<f32>,
    mode: WallMode,
    /// The last thumbnail clicked and when, for double-click detection.
    last_click: Option<(usize, Instant)>,
    /// The operation running over the selection, if any.
    batch: Option<Batch>,
}

/// What a click on a thumbnail asks for. `Open` leaves the wall, so any task
/// the wall would have run is moot — hence no payload.
pub(crate) enum Click {
    Open,
    Handled(Task<Message>),
}

/// One operation applied to every image in the selection, independently.
///
/// The work is dispatched a few files at a time rather than all at once, for
/// the same reason thumbnail decodes are (see [`WallState::schedule`]): a
/// selection can be thousands of images, and handing the runtime all of them
/// would thrash the disk and the CPU to no end.
struct Batch {
    kind: BatchKind,
    pending: VecDeque<PathBuf>,
    in_flight: usize,
    done: usize,
    total: usize,
    /// What went wrong, per file. Reported once at the end rather than one
    /// message per failure, which for a whole failed batch would be a wall of
    /// identical lines.
    failed: Vec<(PathBuf, String)>,
    /// Files that have left the library's folder. Collected rather than removed
    /// one by one: re-laying the masonry per file would shuffle the wall under
    /// whoever is watching it.
    gone: Vec<PathBuf>,
    /// Set by Esc: nothing further is dispatched, but work already handed to
    /// the runtime is left to land rather than abandoned mid-write.
    cancelled: bool,
}

/// The operations a selection can be put through.
#[derive(Debug, Clone)]
pub(crate) enum BatchKind {
    Rotate {
        clockwise: bool,
    },
    Transfer {
        kind: TransferKind,
        dest: PathBuf,
        collision: Collision,
    },
}

impl BatchKind {
    fn verb(&self) -> &'static str {
        match self {
            BatchKind::Rotate { .. } => "Rotating",
            BatchKind::Transfer { kind, .. } => kind.verb(),
        }
    }
}

impl WallState {
    pub(crate) fn new(library: Library) -> Self {
        // The mode is derived, not carried: a selection that survived a trip
        // through the single view puts the wall straight back into `Select`.
        // A range being painted does not survive — it is inherently transient.
        let mode = if library.selection.is_empty() {
            WallMode::Normal
        } else {
            WallMode::Select
        };
        Self {
            library,
            thumbs: HashMap::new(),
            ratios: HashMap::new(),
            in_flight: HashSet::new(),
            stale: HashSet::new(),
            rotating: HashSet::new(),
            viewport: None,
            scroll_y: 0.0,
            focus: 0.0,
            desired_y: None,
            mode,
            last_click: None,
            batch: None,
        }
    }

    /// Whether Enter should commit a painted range rather than open an image.
    pub(crate) fn is_visual(&self) -> bool {
        matches!(self.mode, WallMode::Visual { .. })
    }

    /// The selection, in library order, if it can be operated on right now.
    ///
    /// `None` while painting (the range is not committed, so the target is
    /// undecided) or while a batch is already running.
    pub(crate) fn operable_selection(&self) -> Option<Vec<PathBuf>> {
        if self.is_visual() || self.batch.is_some() || self.mode != WallMode::Select {
            return None;
        }
        let selected: Vec<PathBuf> = self
            .library
            .paths
            .iter()
            .filter(|p| self.library.is_selected(p))
            .cloned()
            .collect();
        (!selected.is_empty()).then_some(selected)
    }

    /// Drop images that are no longer on disk, and re-lay the wall around what
    /// is left. Returns `false` if the library is now empty.
    pub(crate) fn removed(&mut self, gone: &HashSet<PathBuf>) -> (bool, Task<Message>) {
        for path in gone {
            self.thumbs.remove(path);
            self.ratios.remove(path);
            // A decode of a deleted file is worthless; discard it on arrival.
            if self.in_flight.contains(path) {
                self.stale.insert(path.clone());
            }
        }
        if !self.library.remove(gone) {
            return (false, Task::none());
        }

        self.desired_y = None;
        let settle = self.settle();
        self.refocus();
        let reveal = match self.viewport {
            Some(viewport) => self.reveal(&self.layout(viewport.width)),
            None => Task::none(),
        };
        (true, Task::batch([settle, reveal, self.schedule()]))
    }

    /// Handle a click on thumbnail `index`, carrying the modifiers live at the
    /// time (a `button` reports none of its own — see `App::modifiers`).
    ///
    /// The mouse is modal in the same way the keyboard is. In `Normal` a plain
    /// click still opens the image, exactly as it did before selection existed;
    /// `Cmd` is the way in without touching the keyboard. Once a selection is
    /// live, plain clicks select instead, and opening moves to a double click.
    ///
    /// Every click also moves the cursor to what it hit, so a `v` pressed after
    /// a click anchors where the user is actually looking.
    pub(crate) fn click(&mut self, index: usize, modifiers: Modifiers) -> Click {
        if index >= self.library.paths.len() {
            return Click::Handled(Task::none());
        }
        let cursor = self.library.paths.index();
        let double = self.register_click(index);

        // While painting, a click is a motion: the cursor is the moving end of
        // the range, so clicking a thumbnail extends the range to it, exactly
        // as `j` would. Editing the committed set here would settle the mode
        // and end the range behind the user's back, as it does for `Space`.
        if self.is_visual() {
            return Click::Handled(self.move_cursor_to(index));
        }

        // In `Normal` a bare click keeps its old meaning; a modifier is the
        // request to select instead.
        let selecting = self.mode == WallMode::Select || modifiers.command() || modifiers.shift();
        if !selecting {
            return Click::Open;
        }

        if modifiers.shift() {
            // Extend from wherever the cursor is — which, after any earlier
            // click, is the last thumbnail clicked.
            self.library.apply_range(cursor, index, RangeOp::Add);
        } else {
            self.library.toggle_selected(index);
        }

        let task = Task::batch([self.move_cursor_to(index), self.settle()]);
        if double {
            // The first click of the pair toggled this tile and the second
            // toggled it back, so opening leaves the selection as it was.
            Click::Open
        } else {
            Click::Handled(task)
        }
    }

    /// Whether this click completes a double click, and record it either way.
    /// A completed pair clears the record, so a third click starts afresh
    /// rather than reading as another double.
    fn register_click(&mut self, index: usize) -> bool {
        let now = Instant::now();
        let double = matches!(
            self.last_click,
            Some((last, at)) if last == index && now.duration_since(at) < DOUBLE_CLICK
        );
        self.last_click = (!double).then_some((index, now));
        double
    }

    /// Put the cursor on `index` and bring it into view.
    fn move_cursor_to(&mut self, index: usize) -> Task<Message> {
        self.library.goto(index);
        // The move was to an arbitrary point, so any sticky centre a run of
        // `h`/`l` had built up no longer describes where the cursor sits.
        self.desired_y = None;
        let reveal = match self.viewport {
            Some(viewport) => self.reveal(&self.layout(viewport.width)),
            None => Task::none(),
        };
        self.refocus();
        Task::batch([reveal, self.schedule()])
    }

    /// Everything the wall needs on entry: measure the viewport, read the
    /// thumbnail dimensions, and start decoding.
    pub(crate) fn enter(&mut self) -> Task<Message> {
        Task::batch([measure(), self.load_ratios(), self.schedule()])
    }

    pub(crate) fn update(&mut self, msg: WallMsg) -> Task<Message> {
        match msg {
            WallMsg::Measured(size) => {
                // Never re-measure from here: that would loop.
                if self.viewport == Some(size) {
                    return Task::none();
                }
                let first = self.viewport.is_none();
                self.viewport = Some(size);
                self.refocus();
                // On the first measurement the wall has just appeared, so bring
                // the image carried in from the single view into view.
                let reveal = if first {
                    self.reveal(&self.layout(size.width))
                } else {
                    Task::none()
                };
                Task::batch([reveal, self.schedule()])
            }
            WallMsg::RatiosLoaded(heights) => {
                self.ratios.extend(heights);
                self.refocus();
                self.schedule()
            }
            WallMsg::ThumbDecoded { path, result } => {
                self.in_flight.remove(&path);
                // Rotated mid-decode: these are the old pixels. Dropping them
                // leaves the path un-cached, so `schedule` re-dispatches it.
                if self.stale.remove(&path) {
                    return self.schedule();
                }
                match result {
                    Ok((handle, height)) => {
                        self.thumbs.insert(path, ThumbState { handle, height });
                    }
                    Err(e) => eprintln!("Thumbnail decode error: {e}"),
                }
                // A slot freed: dispatch the next-nearest pending thumbnail(s).
                // Deliberately no `refocus` — with heights known up front the
                // layout barely moves, and this runs once per image.
                self.schedule()
            }
            WallMsg::Scrolled(offset) => {
                self.scroll_y = if offset.is_finite() { offset.max(0.0) } else { 0.0 };
                // Reprioritise: the next freed slots go to the new viewport.
                self.refocus();
                self.schedule()
            }
            WallMsg::Nav(dir) => self.navigate(dir),
            WallMsg::Rotate { clockwise } => self.rotate(clockwise),
            WallMsg::Rotated { path, result } => self.rotated(path, result),
            WallMsg::EnterVisual { op } => self.enter_visual(op),
            WallMsg::CommitVisual => self.commit_visual(),
            // These three edit the committed set, and `settle` would drop the
            // mode back out of `Visual` — so mid-paint they are ignored rather
            // than allowed to end the range behind the user's back. Finish or
            // cancel the range first.
            WallMsg::ToggleCursor if self.is_visual() => Task::none(),
            WallMsg::SelectAll if self.is_visual() => Task::none(),
            WallMsg::InvertSelection if self.is_visual() => Task::none(),
            WallMsg::ToggleCursor => {
                self.library.toggle_selected(self.library.paths.index());
                self.settle()
            }
            WallMsg::SelectAll => {
                self.library.select_all();
                self.settle()
            }
            WallMsg::InvertSelection => {
                self.library.invert_selection();
                self.settle()
            }
            WallMsg::ToggleFavourite => self.favourite(),
            WallMsg::ToggleFilter => self.toggle_filter(),
            WallMsg::Escape => self.escape(),
            WallMsg::StartBatch { kind, paths } => self.start_batch(kind, paths),
            WallMsg::BatchProgress { path, result } => self.batch_progress(path, result),
        }
    }

    /// Start painting a range from the cursor. Pressing the same key again
    /// while painting cancels, as `v` does in vim.
    fn enter_visual(&mut self, op: RangeOp) -> Task<Message> {
        match self.mode {
            WallMode::Visual { .. } => self.escape(),
            // `x` means "remove a run", which is meaningless with nothing
            // selected — and entering a mode that can only be a no-op would
            // just strand the user in it.
            WallMode::Normal if op == RangeOp::Remove => Task::none(),
            WallMode::Normal | WallMode::Select => {
                self.mode = WallMode::Visual {
                    anchor: self.library.paths.index(),
                    op,
                };
                self.remeasure()
            }
        }
    }

    /// Fold the painted range into the selection and leave `Visual`. The cursor
    /// stays where it is: the range's far end is where the user is looking.
    fn commit_visual(&mut self) -> Task<Message> {
        let WallMode::Visual { anchor, op } = self.mode else {
            return Task::none();
        };
        self.library.apply_range(anchor, self.library.paths.index(), op);
        self.settle()
    }

    /// One rung down the Esc ladder. A running batch is the top rung — the
    /// user is most likely reaching for Esc to stop it. From `Visual` it drops
    /// the painted range
    /// and returns the cursor to the anchor — the trip was cancelled, so it
    /// ends where it began. From `Select` it clears the set and leaves the
    /// cursor alone, because there the user moved it deliberately.
    ///
    /// `Visual` entered from `Select` therefore takes two presses to leave
    /// entirely: one keypress must not discard a large selection.
    fn escape(&mut self) -> Task<Message> {
        if self.batch.is_some() {
            return self.cancel_batch();
        }
        match self.mode {
            WallMode::Visual { anchor, .. } => {
                self.library.goto(anchor);
                self.desired_y = None;
                let settle = self.settle();
                let reveal = match self.viewport {
                    Some(viewport) => self.reveal(&self.layout(viewport.width)),
                    None => Task::none(),
                };
                Task::batch([settle, reveal, self.schedule()])
            }
            WallMode::Select => {
                self.library.clear_selection();
                self.settle()
            }
            WallMode::Normal => Task::none(),
        }
    }

    /// Restore the mode invariant after a change to the selection, and
    /// re-measure in case the mode bar appeared or disappeared.
    fn settle(&mut self) -> Task<Message> {
        self.mode = if self.library.selection.is_empty() {
            WallMode::Normal
        } else {
            WallMode::Select
        };
        self.remeasure()
    }

    /// The mode bar takes height from the scroll viewport, and `reveal` scrolls
    /// against that height — so showing or hiding it has to be measured again.
    fn remeasure(&self) -> Task<Message> {
        measure()
    }

    /// Rotate 90° on disk: the whole selection in `Select`, otherwise just the
    /// image under the cursor.
    ///
    /// Ignored while painting a range, for the same reason `Space` is: the
    /// range has no committed meaning yet, so there is no honest answer to
    /// "which images?". Finish or cancel it first. Also ignored while a batch
    /// is already running, so two batches can't fight over the same files.
    fn rotate(&mut self, clockwise: bool) -> Task<Message> {
        if self.is_visual() || self.batch.is_some() {
            return Task::none();
        }
        if let Some(selected) = self.operable_selection() {
            return self.start_batch(BatchKind::Rotate { clockwise }, selected);
        }
        self.rotate_one(clockwise)
    }

    /// Favourite the whole selection, or — with nothing selected — the image
    /// under the cursor.
    ///
    /// The same shape as `r`: an operation over the selection where there is
    /// one, over the cursor where there is not. Unlike `r` it needs no batch —
    /// a tag is a line in a small text file, not a write per photo.
    ///
    /// Ignored while painting, for the reason everything is: an uncommitted
    /// range has no settled meaning.
    fn favourite(&mut self) -> Task<Message> {
        if self.is_visual() {
            return Task::none();
        }
        let paths = match self.operable_selection() {
            Some(selected) => selected,
            None => vec![self.library.current().clone()],
        };
        self.library.toggle_tag(tags::FAVOURITE, &paths);
        // Only bites while the favourites are what is on screen, where
        // un-favouriting takes photos off the wall as it goes.
        self.library.refilter();
        self.relaid()
    }

    /// Show only the favourites, or show everything again.
    ///
    /// Refused while anything is selected. A filter that hid a selected photo
    /// would put images the user cannot see into the next batch; forbidding the
    /// combination removes that whole class of bug rather than papering over it
    /// (see `SELECT_MODE_PLAN.md`). Clearing the selection with Esc is one key.
    fn toggle_filter(&mut self) -> Task<Message> {
        if self.is_visual() || self.batch.is_some() || !self.library.selection.is_empty() {
            return Task::none();
        }
        let tag = match self.library.filter {
            Some(_) => None,
            None => Some(tags::FAVOURITE.to_string()),
        };
        if !self.library.set_filter(tag) {
            // Nothing carries the tag: the wall would go blank and claim the
            // folder was empty.
            return Task::none();
        }
        // Only the view changed, so there is nothing to write to disk.
        Task::batch([self.resettled(), self.remeasure()])
    }

    /// Everything the wall has to redo after the visible list changed, plus the
    /// write to disk that a tag change earns.
    fn relaid(&mut self) -> Task<Message> {
        let save = Task::perform(
            tags::save_async(self.library.image_dir.clone(), self.library.tags.clone()),
            Message::TagsSaved,
        );
        Task::batch([self.resettled(), save, self.settle()])
    }

    /// Re-aim the wall at wherever the cursor ended up. The tiles themselves
    /// have not changed shape, so no thumbnail is thrown away — the masonry
    /// simply has fewer or more of them to place.
    fn resettled(&mut self) -> Task<Message> {
        self.desired_y = None;
        self.refocus();
        let reveal = match self.viewport {
            Some(viewport) => self.reveal(&self.layout(viewport.width)),
            None => Task::none(),
        };
        Task::batch([reveal, self.schedule()])
    }

    /// Begin an operation over `paths`.
    ///
    /// Refused while a range is being painted or another batch is running, for
    /// the same reasons [`WallState::rotate`] is: an uncommitted range has no
    /// settled meaning, and two batches would fight over the same files.
    fn start_batch(&mut self, kind: BatchKind, paths: Vec<PathBuf>) -> Task<Message> {
        if self.is_visual() || self.batch.is_some() || paths.is_empty() {
            return Task::none();
        }
        let pending: VecDeque<PathBuf> = paths.into_iter().collect();
        self.batch = Some(Batch {
            kind,
            total: pending.len(),
            pending,
            in_flight: 0,
            done: 0,
            failed: Vec::new(),
            gone: Vec::new(),
            cancelled: false,
        });
        Task::batch([self.refill(), self.remeasure()])
    }

    /// Hand the runtime as many of the batch's remaining files as its
    /// concurrency cap allows, and no more.
    fn refill(&mut self) -> Task<Message> {
        let Some(batch) = &mut self.batch else {
            return Task::none();
        };
        let kind = batch.kind.clone();
        let free = max_in_flight().saturating_sub(batch.in_flight);
        let mut chosen = Vec::new();
        if !batch.cancelled {
            while chosen.len() < free {
                let Some(path) = batch.pending.pop_front() else {
                    break;
                };
                chosen.push(path);
            }
        }
        batch.in_flight += chosen.len();

        let tasks: Vec<Task<Message>> = chosen
            .into_iter()
            .map(|path| {
                let key = path.clone();
                if matches!(kind, BatchKind::Rotate { .. }) {
                    // The same per-file claim a single rotate takes, so a batch
                    // and a stray keypress can't write one file twice at once.
                    self.rotating.insert(path.clone());
                }
                Task::perform(run_one(path, kind.clone()), move |result| {
                    Message::Wall(WallMsg::BatchProgress {
                        path: key.clone(),
                        result,
                    })
                })
            })
            .collect();
        Task::batch(tasks)
    }

    /// One file of a batch has landed: fold it in, refill the slot it freed,
    /// and finish up if it was the last.
    fn batch_progress(&mut self, path: PathBuf, result: Result<FileDone, String>) -> Task<Message> {
        self.rotating.remove(&path);
        let Some(batch) = &mut self.batch else {
            return Task::none();
        };
        batch.in_flight = batch.in_flight.saturating_sub(1);
        batch.done += 1;

        let mut reshaped = false;
        match result {
            Ok(FileDone::Reshaped) => reshaped = true,
            // Held back rather than removed now: dropping images from the
            // library re-lays the masonry, and doing that per file would
            // shuffle the wall while the user is watching it.
            Ok(FileDone::Gone) => batch.gone.push(path.clone()),
            Ok(FileDone::Unchanged) => {}
            Err(e) => batch.failed.push((path.clone(), e)),
        }

        // Deliberately no `reveal` and no `refocus` here either: those move the
        // view. They run once, at the end.
        let invalidate = if reshaped {
            self.invalidate_rotated(path)
        } else {
            Task::none()
        };
        let finished = self
            .batch
            .as_ref()
            .is_some_and(|b| b.in_flight == 0 && (b.pending.is_empty() || b.cancelled));

        if finished {
            let batch = self.batch.take().expect("checked just above");
            if !batch.failed.is_empty() {
                eprintln!(
                    "{} failed for {} of {} images; first: {}",
                    batch.kind.verb(),
                    batch.failed.len(),
                    batch.total,
                    batch.failed[0].1
                );
            }
            // Every tile the batch touched changed shape or left, so a sticky
            // centre from before it means nothing now.
            self.desired_y = None;
            self.refocus();
            // The files that left go back through `App`, which is the only
            // place that can fall back to the empty screen if none are left.
            let gone = if batch.gone.is_empty() {
                Task::none()
            } else {
                Task::done(Message::Removed {
                    gone: batch.gone,
                    failed: Vec::new(),
                })
            };
            return Task::batch([invalidate, gone, self.remeasure(), self.schedule()]);
        }
        Task::batch([invalidate, self.refill(), self.schedule()])
    }

    /// Stop dispatching new work. Files already handed to the runtime finish —
    /// abandoning a write half-done is how a photo gets corrupted.
    fn cancel_batch(&mut self) -> Task<Message> {
        if let Some(batch) = &mut self.batch {
            batch.cancelled = true;
            batch.pending.clear();
            if batch.in_flight == 0 {
                self.batch = None;
                return self.remeasure();
            }
        }
        Task::none()
    }

    /// Rotate the image under the cursor 90° on disk, off-thread. Ignored while
    /// a rotate of the same file is already in flight.
    fn rotate_one(&mut self, clockwise: bool) -> Task<Message> {
        let Some(path) = self.claim_rotate() else {
            return Task::none();
        };
        let key = path.clone();
        Task::perform(rotate_async(path, clockwise), move |result| {
            Message::Wall(WallMsg::Rotated {
                path: key.clone(),
                result,
            })
        })
    }

    /// Claim the selected path for a rotate, or `None` if one is already
    /// writing it — two concurrent read-modify-writes of the same file would
    /// race, and one 90° turn would be lost.
    fn claim_rotate(&mut self) -> Option<PathBuf> {
        let path = self.library.current().clone();
        self.rotating.insert(path.clone()).then_some(path)
    }

    /// The file changed shape on disk: invalidate its thumbnail, re-read its
    /// dimensions, and let the masonry settle around the new one.
    ///
    /// Only slots at or after the rotated image can move — the shortest-column
    /// pass places index `i` from the column heights left by indices before it,
    /// which this doesn't touch. So nothing above needs re-decoding, and the
    /// thumbnails below simply get placed at new offsets with the handles they
    /// already have.
    fn rotated(&mut self, path: PathBuf, result: Result<(), String>) -> Task<Message> {
        self.rotating.remove(&path);
        if let Err(e) = result {
            eprintln!("Rotate failed: {e}");
            return Task::none();
        }

        let invalidate = self.invalidate_rotated(path);
        // Rotating changes the tile's height, so a sticky centre taken before
        // it no longer describes where the selection sits.
        self.desired_y = None;
        self.refocus();

        let reveal = match self.viewport {
            Some(viewport) => self.reveal(&self.layout(viewport.width)),
            None => Task::none(),
        };
        Task::batch([invalidate, reveal, self.schedule()])
    }

    /// Drop what the wall knows about a file that has just changed shape on
    /// disk, and start re-reading its dimensions.
    ///
    /// Split out from [`WallState::rotated`] because a batch runs this once per
    /// image and must *not* scroll or re-aim the view while doing so — see
    /// [`WallState::batch_progress`].
    fn invalidate_rotated(&mut self, path: PathBuf) -> Task<Message> {
        self.thumbs.remove(&path);
        // An in-flight decode of this path predates the write; discard it when
        // it lands (see `WallMsg::ThumbDecoded`).
        if self.in_flight.contains(&path) {
            self.stale.insert(path.clone());
        }
        // Provisional height so the placeholder is the right shape immediately:
        // a 90° turn swaps the aspect ratio, so `h' = WIDTH^2 / h`. The header
        // read below replaces it with the exact value a moment later — that one
        // has to match `imaging::thumbnail` bit for bit or the wall shifts when
        // the decode lands.
        if let Some(height) = self.ratios.get(&path).copied() {
            let width = THUMB_WIDTH as f32;
            self.ratios
                .insert(path.clone(), (width * width / height).round().max(1.0));
        }
        Task::perform(
            crate::imaging::thumb_heights_async(vec![path], THUMB_WIDTH),
            |heights| Message::Wall(WallMsg::RatiosLoaded(heights)),
        )
    }

    /// Move the selection one thumbnail in `dir`, scroll it into view, and
    /// re-aim the decode scheduler at it. A no-op at the edges of the wall.
    fn navigate(&mut self, dir: Dir) -> Task<Message> {
        let Some(viewport) = self.viewport else {
            return Task::none();
        };
        let layout = self.layout(viewport.width);
        let current = self.library.paths.index();
        let Some(next) = neighbour(&layout, current, dir, self.desired_y) else {
            return Task::none();
        };
        self.library.goto(next);
        // Left/right keeps the vertical centre it started from; up/down resets
        // it, since those moves redefine where "beside" is.
        self.desired_y = match dir {
            Dir::Left | Dir::Right => Some(
                self.desired_y
                    .unwrap_or_else(|| centre_of(&layout, current).unwrap_or(0.0)),
            ),
            Dir::Up | Dir::Down => None,
        };
        let reveal = self.reveal(&layout);
        self.refocus();
        Task::batch([reveal, self.schedule()])
    }

    /// Scroll the selected thumbnail into view, but only if it isn't already —
    /// and only far enough to reach the nearer edge.
    ///
    /// Takes `&mut self` because a programmatic scroll doesn't fire `on_scroll`,
    /// so the new offset has to be recorded here or the next call would decide
    /// from a stale one.
    fn reveal(&mut self, layout: &WallLayout) -> Task<Message> {
        let Some(viewport) = self.viewport else {
            return Task::none();
        };
        let Some(slot) = layout.slots.get(&self.library.paths.index()) else {
            return Task::none();
        };
        // Include the selection ring, so it never sits half off-screen.
        let top = slot.y - SEL_BORDER;
        let bottom = slot.y + slot.height + SEL_BORDER;

        let target = if top < self.scroll_y {
            top - WALL_SPACING
        } else if bottom > self.scroll_y + viewport.height {
            bottom + WALL_SPACING - viewport.height
        } else {
            return Task::none();
        };

        let max = (layout.content_height - viewport.height).max(0.0);
        let y = target.clamp(0.0, max);
        self.scroll_y = y;
        scroll_to(WALL_ID, AbsoluteOffset { x: 0.0, y })
    }

    /// Re-aim decode priority at whatever is now in the middle of the viewport.
    fn refocus(&mut self) {
        let Some(viewport) = self.viewport else {
            self.focus = 0.0;
            return;
        };
        let layout = self.layout(viewport.width);
        self.focus = nearest_position(&layout, self.scroll_y + viewport.height / 2.0);
    }

    /// Read the library's thumbnail dimensions from their headers, off-thread.
    /// One task for the whole library: the cost is per-file IO, and a single
    /// task keeps it from competing with the decode scheduler.
    fn load_ratios(&self) -> Task<Message> {
        let paths: Vec<PathBuf> = self.library.paths.iter().cloned().collect();
        Task::perform(
            crate::imaging::thumb_heights_async(paths, THUMB_WIDTH),
            |heights| Message::Wall(WallMsg::RatiosLoaded(heights)),
        )
    }

    /// Fill free decode slots with the highest-priority pending thumbnails.
    ///
    /// Priority is distance from [`WallState::focus`], so what you're looking
    /// at decodes soonest. Called on wall entry and whenever a slot frees or
    /// priorities shift.
    pub(crate) fn schedule(&mut self) -> Task<Message> {
        let free = max_in_flight().saturating_sub(self.in_flight.len());
        if free == 0 {
            return Task::none();
        }

        // Choose the paths first (borrows self immutably), then dispatch and
        // record them (borrows self mutably) — the scope ends the read borrow.
        let chosen: Vec<PathBuf> = {
            let paths: Vec<&PathBuf> = self.library.paths.iter().collect();
            prioritise(&paths, |p| self.needs_decode(p), self.focus, free)
                .into_iter()
                .cloned()
                .collect()
        };

        let tasks: Vec<Task<Message>> = chosen
            .into_iter()
            .map(|path| {
                self.in_flight.insert(path.clone());
                let key = path.clone();
                Task::perform(
                    crate::imaging::thumbnail_async(path, THUMB_WIDTH),
                    move |result| {
                        Message::Wall(WallMsg::ThumbDecoded {
                            path: key.clone(),
                            result,
                        })
                    },
                )
            })
            .collect();
        Task::batch(tasks)
    }

    /// A path still needs decoding: not cached and not already dispatched.
    fn needs_decode(&self, path: &PathBuf) -> bool {
        !self.thumbs.contains_key(path) && !self.in_flight.contains(path)
    }

    /// The height a thumbnail will occupy: its decoded height, else the
    /// header-derived one, else a square guess until either arrives.
    fn tile_height(&self, path: &PathBuf) -> f32 {
        self.thumbs
            .get(path)
            .map(|t| t.height as f32)
            .or_else(|| self.ratios.get(path).copied())
            .unwrap_or(THUMB_WIDTH as f32)
    }

    /// Place every thumbnail into the shortest column, as data.
    ///
    /// Deterministic in (`width`, `thumbs`, `ratios`, library order) — which is
    /// exactly why the view and `update` can call it separately and still
    /// agree.
    fn layout(&self, width: f32) -> WallLayout {
        // `n` tiles need `n * TILE_WIDTH + (n + 1) * WALL_SPACING` (spacing
        // between them, plus the equal outer padding) — hence the single
        // `WALL_SPACING` subtracted before the division.
        let item_width = WALL_SPACING + TILE_WIDTH;
        let col_count = (((width - WALL_SPACING) / item_width).floor() as usize).max(1);

        let mut columns: Vec<Vec<usize>> = vec![Vec::new(); col_count];
        let mut slots = HashMap::new();
        let mut order = Vec::new();
        // Seeded with the container's top padding, so every `y` is absolute
        // within the scrollable's content rather than relative to the grid.
        let mut heights = vec![WALL_SPACING; col_count];

        for (index, path) in self.library.paths.iter().enumerate() {
            let col = heights
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap_or(0);
            let height = self.tile_height(path);
            slots.insert(
                index,
                Slot {
                    col,
                    row: columns[col].len(),
                    y: heights[col],
                    height,
                },
            );
            columns[col].push(index);
            order.push(index);
            heights[col] += height + 2.0 * SEL_BORDER + WALL_SPACING;
        }

        // The tallest column's running total already includes the bottom
        // padding, since each placement adds a trailing `WALL_SPACING`.
        let content_height = heights.into_iter().fold(0.0_f32, f32::max);
        WallLayout {
            columns,
            slots,
            order,
            content_height,
        }
    }

    pub(crate) fn view(&self) -> Element<'_, Message> {
        // The scrollable is rendered even before the first measurement, empty:
        // `measure` has to find it in the tree to report its size at all.
        let grid: Element<'_, Message> = match self.viewport {
            Some(viewport) => self.build_grid(&self.layout(viewport.width)),
            None => Column::new().into(),
        };

        let centered = container(grid)
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding(WALL_SPACING);
        let wall = scrollable(centered)
            .id(WALL_ID)
            .on_scroll(|viewport| Message::Wall(WallMsg::Scrolled(viewport.absolute_offset().y)))
            .height(Length::Fill);

        match self.mode_bar() {
            // Below the wall rather than over it: with no cursor ring to look
            // for in `Visual`, the bar is the only thing saying which mode is
            // live, so it must not be able to hide behind a thumbnail.
            Some(bar) => Column::with_children(vec![wall.into(), bar]).into(),
            None => wall.into(),
        }
    }

    /// The mode indicator along the bottom, or `None` in `Normal` — where there
    /// is no mode to indicate and nothing to say.
    fn mode_bar(&self) -> Option<Element<'_, Message>> {
        // A running batch takes the bar: it is the only thing telling the user
        // that work is under way, and how to stop it.
        if let Some(batch) = &self.batch {
            let label = if batch.cancelled {
                format!("{} \u{2014} finishing {}", batch.kind.verb(), batch.in_flight)
            } else {
                format!("{} {}/{}", batch.kind.verb(), batch.done, batch.total)
            };
            return Some(self.bar(label, "Esc cancel"));
        }

        let (label, hint) = match self.mode {
            // Nothing to say in `Normal` — unless the wall is showing a subset
            // of the folder, which the user must never have to guess at.
            WallMode::Normal => {
                return self.library.filter.as_ref().map(|tag| {
                    let label = format!(
                        "{} \u{2014} {} of {}",
                        tag.to_uppercase(),
                        self.library.paths.len(),
                        self.library.all.len()
                    );
                    self.bar(label, "\u{21e7}F show all \u{b7} f unfavourite")
                });
            }
            WallMode::Visual { anchor, op } => {
                let (total, delta) = self.pending_counts(anchor, op);
                let verb = match op {
                    RangeOp::Add => "add",
                    RangeOp::Remove => "remove",
                };
                (
                    format!("-- VISUAL ({verb}) -- {total} ({delta:+})"),
                    "\u{21b5} commit \u{b7} Esc cancel",
                )
            }
            WallMode::Select => (
                format!("SELECT {}", self.library.selection.len()),
                "f fav \u{b7} r rotate \u{b7} m move \u{b7} c copy \u{b7} d trash \u{b7} v add \u{b7} x remove \u{b7} Esc clear",
            ),
        };
        Some(self.bar(label, hint))
    }

    /// The bar itself: a label on the left, the keys that apply on the right.
    fn bar<'a>(&self, label: String, hint: &'a str) -> Element<'a, Message> {
        container(
            Row::with_children(vec![
                text(label).size(14).into(),
                Space::new().width(Length::Fill).into(),
                text(hint).size(13).style(text::secondary).into(),
            ])
            .align_y(Vertical::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(BAR_HEIGHT))
        .padding([0, 12])
        .align_y(Vertical::Center)
        .style(mode_bar_style)
        .into()
    }

    /// What the painted range would leave selected: the resulting total, and
    /// the signed change. Shown live so a range is never committed blind.
    fn pending_counts(&self, anchor: usize, op: RangeOp) -> (usize, i64) {
        let cursor = self.library.paths.index();
        let (lo, hi) = (anchor.min(cursor), anchor.max(cursor));
        let touched = self
            .library
            .paths
            .iter()
            .skip(lo)
            .take(hi + 1 - lo)
            .filter(|p| match op {
                RangeOp::Add => !self.library.is_selected(p),
                RangeOp::Remove => self.library.is_selected(p),
            })
            .count();
        let current = self.library.selection.len();
        match op {
            RangeOp::Add => (current + touched, touched as i64),
            RangeOp::Remove => (current - touched, -(touched as i64)),
        }
    }

    /// Turn a computed [`WallLayout`] into the column-of-thumbnails widget tree.
    fn build_grid<'a>(&'a self, layout: &WallLayout) -> Element<'a, Message> {
        let current = self.library.paths.index();
        let paths: Vec<&'a PathBuf> = self.library.paths.iter().collect();

        let columns: Vec<Element<'a, Message>> = layout
            .columns
            .iter()
            .map(|indices| {
                let items: Vec<Element<'a, Message>> = indices
                    .iter()
                    .filter_map(|&index| {
                        let path = *paths.get(index)?;
                        Some(self.thumb_element(index, path, current))
                    })
                    .collect();
                Column::with_children(items)
                    .spacing(WALL_SPACING)
                    .width(Length::Fixed(TILE_WIDTH))
                    .into()
            })
            .collect();

        Row::with_children(columns).spacing(WALL_SPACING).into()
    }

    /// One thumbnail: image (or placeholder), clickable, decorated according to
    /// [`WallState::tile_look`].
    fn thumb_element<'a>(
        &'a self,
        index: usize,
        path: &'a PathBuf,
        current: usize,
    ) -> Element<'a, Message> {
        let height = self.tile_height(path);
        let inner: Element<'a, Message> = match self.thumbs.get(path) {
            Some(thumb) => image(thumb.handle.clone())
                .width(Length::Fixed(THUMB_WIDTH as f32))
                .height(Length::Fixed(thumb.height as f32))
                .into(),
            None => container(text("Loading\u{2026}").size(14))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .width(Length::Fixed(THUMB_WIDTH as f32))
                .height(Length::Fixed(height))
                .style(placeholder_style)
                .into(),
        };

        let look = self.tile_look(index, path, current);

        // Layers over the thumbnail, sized to it explicitly rather than left to
        // fill: a `Stack` takes its size from the first child, and the decoded
        // height and the header-derived one can differ by a pixel.
        let mut layers: Vec<Element<'a, Message>> = vec![inner];
        if let Some(accent) = look.tint {
            let alpha = look.tint_alpha;
            layers.push(
                container(Space::new())
                    .width(Length::Fixed(THUMB_WIDTH as f32))
                    .height(Length::Fixed(height))
                    .style(move |theme: &Theme| container::Style {
                        background: Some(Background::Color(Color {
                            a: alpha,
                            ..accent_color(theme, accent)
                        })),
                        ..container::Style::default()
                    })
                    .into(),
            );
        }
        if look.badge {
            layers.push(corner_badge("\u{2713}", Accent::Select, Horizontal::Right));
        }
        if look.star {
            // Opposite corner from the selection tick, so a favourite that is
            // also selected shows both rather than one hiding the other.
            layers.push(favourite_star());
        }

        let body: Element<'a, Message> = if layers.len() == 1 {
            layers.pop().unwrap()
        } else {
            Stack::with_children(layers).into()
        };

        let ring = look.ring;
        button(body)
            // Inset the content so the ring isn't drawn over it.
            .padding(SEL_BORDER)
            .on_press(Message::ThumbClicked(index))
            .style(move |theme: &Theme, _status| thumb_button_style(theme, ring))
            .into()
    }

    /// How one tile is decorated.
    ///
    /// Two independent channels, so a tile can say two things at once: the ring
    /// is where the cursor is, the tint and badge are what is selected. The
    /// cursor ring is hidden in `Visual` — the leading edge of the painted
    /// range already shows where the cursor is, and a second highlight
    /// competing with the tint just reads as noise.
    fn tile_look(&self, index: usize, path: &std::path::Path, current: usize) -> TileLook {
        let selected = self.library.is_selected(path);
        let pending = match self.mode {
            WallMode::Visual { anchor, op } => {
                let (lo, hi) = (anchor.min(current), anchor.max(current));
                (lo..=hi).contains(&index).then_some(op)
            }
            _ => None,
        };

        let cursor = index == current && !self.is_visual();
        let ring = if cursor {
            // The cursor wins the ring: it is the thing that moves, so it has
            // to stay findable. Selection still shows through tint and badge.
            Some(Accent::Cursor)
        } else {
            match pending {
                Some(RangeOp::Add) => Some(Accent::Select),
                Some(RangeOp::Remove) => Some(Accent::Remove),
                None => selected.then_some(Accent::Select),
            }
        };

        let (tint, tint_alpha) = match pending {
            Some(RangeOp::Add) => (Some(Accent::Select), TINT_PENDING),
            Some(RangeOp::Remove) => (Some(Accent::Remove), TINT_PENDING),
            None if selected => (Some(Accent::Select), TINT_COMMITTED),
            None => (None, 0.0),
        };

        TileLook {
            ring,
            tint,
            tint_alpha,
            // Kept on a tile pending removal: it is still selected until the
            // range is committed, and saying otherwise would pre-empt the user.
            badge: selected,
            // Redundant while the wall is filtered to the favourites — every
            // tile would carry one, which says nothing.
            star: self.library.filter.is_none() && self.library.is_tagged(tags::FAVOURITE, path),
        }
    }
}

/// How one tile is decorated. `ring` is the border colour, `tint` a
/// translucent wash over the thumbnail, `badge` the corner checkmark.
#[derive(Clone, Copy)]
struct TileLook {
    ring: Option<Accent>,
    tint: Option<Accent>,
    tint_alpha: f32,
    badge: bool,
    star: bool,
}

/// The three things a tile can be saying, mapped to palette colours by
/// [`accent_color`]. Distinct hues, not shades of one: cursor and selection
/// have to be told apart at a glance across a wall of photos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Accent {
    Cursor,
    Select,
    Remove,
    Favourite,
}

fn accent_color(theme: &Theme, accent: Accent) -> Color {
    let palette = theme.extended_palette();
    match accent {
        // `primary.strong` is only a 0.10 lift off the base; take a bigger one
        // so the ring reads clearly against dark thumbnails.
        Accent::Cursor => lighten(palette.primary.base.color, SEL_LIGHTEN),
        Accent::Select => lighten(palette.success.base.color, SEL_LIGHTEN),
        Accent::Remove => lighten(palette.danger.base.color, SEL_LIGHTEN),
        // Amber, from no palette slot — the four palette roles are spoken for,
        // and a favourite has to be told apart from a selection at a glance.
        Accent::Favourite => Color::from_rgb(0.98, 0.75, 0.18),
    }
}

/// The favourite marker. Public so the single view draws exactly the same one:
/// two screens disagreeing about what a favourite looks like would be worse
/// than either choice.
pub(crate) fn favourite_star() -> Element<'static, Message> {
    corner_badge("\u{2605}", Accent::Favourite, Horizontal::Left)
}

/// A symbol in a rounded pill in one corner of a tile.
///
/// A shape as well as a colour, so neither selection nor favouriting is carried
/// by hue alone.
fn corner_badge(symbol: &'static str, accent: Accent, side: Horizontal) -> Element<'static, Message> {
    // The amber is light enough that white on it would not read.
    let fg = match accent {
        Accent::Favourite => Color::BLACK,
        _ => Color::WHITE,
    };
    container(
        container(text(symbol).size(18).color(fg))
            .padding([1, 7])
            .style(move |theme: &Theme| container::Style {
                background: Some(Background::Color(accent_color(theme, accent))),
                border: Border {
                    radius: 999.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(side)
    .align_y(Vertical::Top)
    .padding(ICON_MARGIN)
    .into()
}

/// Rotate one file on the blocking pool. Shared by the single-image path and
/// the batch, so both write files exactly the same way.
async fn rotate_async(path: PathBuf, clockwise: bool) -> Result<(), String> {
    tokio::task::spawn_blocking(move || crate::imaging::rotate_in_place(&path, clockwise))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

/// Apply one file's worth of a batch, off the GUI thread.
async fn run_one(path: PathBuf, kind: BatchKind) -> Result<FileDone, String> {
    match kind {
        BatchKind::Rotate { clockwise } => rotate_async(path, clockwise)
            .await
            .map(|()| FileDone::Reshaped),
        BatchKind::Transfer {
            kind,
            dest,
            collision,
        } => tokio::task::spawn_blocking(move || transfer::transfer(&path, &dest, kind, collision))
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
            .map(|done| match done {
                // A move that went through: the image is not in this folder any
                // more, so the wall has to stop showing it.
                Transferred::SourceGone => FileDone::Gone,
                // A copy, or a move skipped over a name clash. Either way the
                // wall is unchanged — and a skipped file stays selected, so a
                // second attempt with a different answer hits exactly those.
                Transferred::SourceKept => FileDone::Unchanged,
            }),
    }
}

/// Read the wall scrollable's viewport size back out of the laid-out widget
/// tree, as a message for `update` to store.
///
/// Yields nothing if the wall isn't on screen — the operation simply finds no
/// scrollable with [`WALL_ID`] and no message is sent.
pub(crate) fn measure() -> Task<Message> {
    operate(measure_wall(WALL_ID)).map(|size| Message::Wall(WallMsg::Measured(size)))
}

fn measure_wall(target: Id) -> impl Operation<Size> {
    struct Measure {
        target: Id,
        found: Option<Size>,
    }

    impl Operation<Size> for Measure {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Size>)) {
            operate(self);
        }

        fn scrollable(
            &mut self,
            id: Option<&Id>,
            bounds: Rectangle,
            _content_bounds: Rectangle,
            _translation: Vector,
            _state: &mut dyn Scrollable,
        ) {
            if Some(&self.target) == id {
                self.found = Some(bounds.size());
            }
        }

        fn finish(&self) -> Outcome<Size> {
            self.found.map_or(Outcome::None, Outcome::Some)
        }
    }

    Measure {
        target,
        found: None,
    }
}

/// The library index one step from `current` in `dir`, or `None` at an edge.
///
/// Up and down are simply the neighbouring row of the same column. Left and
/// right cross to the adjacent column and take whichever thumbnail's vertical
/// centre is nearest — masonry rows are not aligned, so "the same row index" in
/// the next column can be at an entirely different height. `desired_y` overrides
/// the reference centre so a run of sideways moves doesn't drift.
fn neighbour(
    layout: &WallLayout,
    current: usize,
    dir: Dir,
    desired_y: Option<f32>,
) -> Option<usize> {
    let slot = layout.slots.get(&current)?;
    match dir {
        Dir::Up => layout.columns[slot.col]
            .get(slot.row.checked_sub(1)?)
            .copied(),
        Dir::Down => layout.columns[slot.col].get(slot.row + 1).copied(),
        Dir::Left | Dir::Right => {
            let col = if dir == Dir::Left {
                slot.col.checked_sub(1)?
            } else {
                slot.col + 1
            };
            let reference = desired_y.unwrap_or(slot.y + slot.height / 2.0);
            layout.columns.get(col)?.iter().copied().min_by(|a, b| {
                distance_to(layout, *a, reference)
                    .partial_cmp(&distance_to(layout, *b, reference))
                    .unwrap_or(Ordering::Equal)
            })
        }
    }
}

/// Vertical centre of `index`'s slot.
fn centre_of(layout: &WallLayout, index: usize) -> Option<f32> {
    layout.slots.get(&index).map(|s| s.y + s.height / 2.0)
}

fn distance_to(layout: &WallLayout, index: usize, y: f32) -> f32 {
    centre_of(layout, index).map_or(f32::MAX, |c| (c - y).abs())
}

/// Position in [`WallLayout::order`] of the thumbnail whose centre is nearest
/// `y` — decode priority is expressed in that space, not in pixels, so a decode
/// landing can't reshuffle it.
fn nearest_position(layout: &WallLayout, y: f32) -> f32 {
    layout
        .order
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            distance_to(layout, **a, y)
                .partial_cmp(&distance_to(layout, **b, y))
                .unwrap_or(Ordering::Equal)
        })
        .map(|(pos, _)| pos as f32)
        .unwrap_or(0.0)
}

/// Order `paths` for decode and return the first `free`.
///
/// `pending` marks which paths still need decoding; those are taken nearest
/// `focus` (a position in the list) first. Priority is by list position, not
/// pixel height, so a decode landing never reshuffles it. The sort is stable,
/// so equal-distance items keep list order.
fn prioritise<'a>(
    paths: &[&'a PathBuf],
    pending: impl Fn(&PathBuf) -> bool,
    focus: f32,
    free: usize,
) -> Vec<&'a PathBuf> {
    let mut candidates: Vec<(f32, &'a PathBuf)> = paths
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, p)| pending(p))
        .map(|(pos, p)| ((pos as f32 - focus).abs(), p))
        .collect();
    candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
    candidates.into_iter().take(free).map(|(_, p)| p).collect()
}

fn thumb_button_style(theme: &Theme, ring: Option<Accent>) -> button::Style {
    let palette = theme.extended_palette();
    button::Style {
        background: None,
        text_color: palette.background.base.text,
        // Always the same width — only the colour changes — so decorating a
        // thumbnail never shifts the masonry.
        border: Border {
            color: ring.map_or(Color::TRANSPARENT, |a| accent_color(theme, a)),
            width: SEL_BORDER,
            radius: 0.0.into(),
        },
        shadow: Shadow::default(),
        // 0.14 added pixel-grid snapping; keep the non-crisp default.
        snap: false,
    }
}

fn mode_bar_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(palette.background.weak.color)),
        text_color: Some(palette.background.base.text),
        border: Border {
            color: palette.background.strong.color,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

fn placeholder_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(palette.background.weak.color)),
        ..container::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn paths(n: usize) -> Vec<PathBuf> {
        (0..n).map(|i| PathBuf::from(format!("{i}.jpg"))).collect()
    }

    /// A wall of thumbnails `THUMB_WIDTH` wide and `heights[i]` tall, sized to
    /// fit exactly `col_count` columns. Heights are seeded as if the
    /// header-read prepass had already landed.
    fn wall(heights: &[f32], col_count: usize) -> WallState {
        let files = paths(heights.len());
        let library = Library {
            all: files.clone(),
            paths: crate::pointed_list::PointedList::new(files.clone()).unwrap(),
            image_dir: PathBuf::from("/wall"),
            selection: HashSet::new(),
            tags: crate::tags::Tags::new(),
            filter: None,
        };
        let mut state = WallState::new(library);
        state.ratios = files.into_iter().zip(heights.iter().copied()).collect();
        state.viewport = Some(Size::new(width_for(col_count), 1000.0));
        state
    }

    /// The narrowest width that fits exactly `n` columns.
    fn width_for(n: usize) -> f32 {
        n as f32 * (WALL_SPACING + TILE_WIDTH) + WALL_SPACING
    }

    #[test]
    fn masonry_fills_the_shortest_column() {
        // Heights chosen so placement order is 0->c0, 1->c1, 2->c2, 3->c0
        // (shortest at 220), 4->c2, 5->c1.
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        assert_eq!(layout.columns, vec![vec![0, 3], vec![1, 5], vec![2, 4]]);
    }

    #[test]
    fn left_and_right_pick_the_nearest_vertical_centre() {
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        // 3 sits in column 0 at y=232, centre 382. Column 1 holds 1 (centre
        // 212) and 5 (centre 552) — 5 is nearer. List order would have said 4,
        // which is two columns away.
        assert_eq!(neighbour(&layout, 3, Dir::Right, None), Some(5));
        assert_eq!(neighbour(&layout, 5, Dir::Left, None), Some(3));
    }

    #[test]
    fn up_and_down_stay_in_the_column() {
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        assert_eq!(neighbour(&layout, 0, Dir::Down, None), Some(3));
        assert_eq!(neighbour(&layout, 3, Dir::Up, None), Some(0));
    }

    #[test]
    fn navigation_stops_at_the_edges() {
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        assert_eq!(neighbour(&layout, 0, Dir::Up, None), None);
        assert_eq!(neighbour(&layout, 0, Dir::Left, None), None);
        assert_eq!(neighbour(&layout, 3, Dir::Down, None), None);
        assert_eq!(neighbour(&layout, 4, Dir::Right, None), None);
    }

    #[test]
    fn desired_y_overrides_the_current_centre() {
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        // From 3 (centre 382) the nearest in column 1 is 5, but anchored near
        // the top the answer is 1 instead — this is what stops h/l drifting.
        assert_eq!(neighbour(&layout, 3, Dir::Right, Some(0.0)), Some(1));
    }

    #[test]
    fn single_column_layout_is_the_library_order() {
        let state = wall(&[200.0; 4], 1);
        let layout = state.layout(width_for(1));
        assert_eq!(layout.columns, vec![vec![0, 1, 2, 3]]);
        assert_eq!(neighbour(&layout, 1, Dir::Down, None), Some(2));
        assert_eq!(neighbour(&layout, 1, Dir::Right, None), None);
    }

    #[test]
    fn slot_y_is_absolute_within_the_scroll_content() {
        let state = wall(&[200.0, 200.0], 1);
        let layout = state.layout(width_for(1));
        // First tile starts below the container's top padding, not at 0.
        assert_eq!(layout.slots[&0].y, WALL_SPACING);
        // Second is a tile (200 + 2 * ring) and a gap further down.
        assert_eq!(layout.slots[&1].y, WALL_SPACING + 200.0 + 2.0 * SEL_BORDER + WALL_SPACING);
        // Content height includes the bottom padding.
        assert_eq!(layout.content_height, layout.slots[&1].y + 200.0 + 2.0 * SEL_BORDER + WALL_SPACING);
    }

    /// Heights that lay out as columns `[[0, 3], [1, 5], [2, 4]]` at 3 columns.
    const SPREAD: [f32; 6] = [200.0, 400.0, 200.0, 300.0, 200.0, 200.0];

    #[test]
    fn measuring_unblocks_navigation() {
        let mut state = wall(&SPREAD, 3);
        // Before the first measurement there is no layout, so nav is inert.
        state.viewport = None;
        let _ = state.update(WallMsg::Nav(Dir::Right));
        assert_eq!(state.library.paths.index(), 0);

        let size = Size::new(width_for(3), 1000.0);
        let _ = state.update(WallMsg::Measured(size));
        assert_eq!(state.viewport, Some(size));

        let _ = state.update(WallMsg::Nav(Dir::Right));
        assert_eq!(state.library.paths.index(), 1);
        let _ = state.update(WallMsg::Nav(Dir::Down));
        assert_eq!(state.library.paths.index(), 5);
        let _ = state.update(WallMsg::Nav(Dir::Left));
        assert_eq!(state.library.paths.index(), 3);
    }

    #[test]
    fn sideways_moves_round_trip() {
        let mut state = wall(&SPREAD, 3);
        let _ = state.update(WallMsg::Nav(Dir::Down)); // 0 -> 3
        assert_eq!(state.library.paths.index(), 3);
        // 3's nearest neighbour rightwards is 5, but 5's is 3 only because the
        // sticky centre survives the second press.
        let _ = state.update(WallMsg::Nav(Dir::Right));
        assert_eq!(state.library.paths.index(), 5);
        let _ = state.update(WallMsg::Nav(Dir::Left));
        assert_eq!(state.library.paths.index(), 3);
    }

    #[test]
    fn up_or_down_clears_the_sticky_centre() {
        let mut state = wall(&SPREAD, 3);
        let _ = state.update(WallMsg::Nav(Dir::Down)); // 0 -> 3, centre 382
        let _ = state.update(WallMsg::Nav(Dir::Right)); // -> 5, sticky 382
        let _ = state.update(WallMsg::Nav(Dir::Up)); // -> 1, sticky cleared
        assert_eq!(state.library.paths.index(), 1);
        assert_eq!(state.desired_y, None);
        // Now anchored on 1 (centre 212), not the stale 382, so left gives 0.
        let _ = state.update(WallMsg::Nav(Dir::Left));
        assert_eq!(state.library.paths.index(), 0);
    }

    // --- Selection modes ---

    /// The selected library indices, sorted — the selection is by path, but
    /// indices are what the tests reason in.
    fn selected(state: &WallState) -> Vec<usize> {
        state
            .library
            .paths
            .iter()
            .enumerate()
            .filter(|(_, p)| state.library.is_selected(p))
            .map(|(i, _)| i)
            .collect()
    }

    fn enter_visual(state: &mut WallState, op: RangeOp) {
        let _ = state.update(WallMsg::EnterVisual { op });
    }

    fn nav(state: &mut WallState, dir: Dir) {
        let _ = state.update(WallMsg::Nav(dir));
    }

    #[test]
    fn a_committed_range_selects_the_whole_run() {
        // One column, so `j` moves one library index at a time.
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);
        nav(&mut state, Dir::Down);
        let _ = state.update(WallMsg::CommitVisual);

        assert_eq!(selected(&state), vec![0, 1, 2]);
        assert_eq!(state.mode, WallMode::Select);
        // The cursor stays at the far end of the range, where the user is
        // looking — it does not snap back to the anchor.
        assert_eq!(state.library.paths.index(), 2);
    }

    #[test]
    fn a_range_painted_upwards_covers_the_same_run() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(4);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Up);
        nav(&mut state, Dir::Up);
        let _ = state.update(WallMsg::CommitVisual);
        assert_eq!(selected(&state), vec![2, 3, 4]);
    }

    #[test]
    fn escape_from_visual_cancels_and_returns_the_cursor() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);
        nav(&mut state, Dir::Down);
        let _ = state.update(WallMsg::Escape);

        assert!(state.library.selection.is_empty());
        assert_eq!(state.mode, WallMode::Normal);
        // The trip was cancelled, so it ends where it began.
        assert_eq!(state.library.paths.index(), 1);
    }

    #[test]
    fn v_pressed_twice_cancels_like_vim() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        enter_visual(&mut state, RangeOp::Add);
        assert_eq!(state.mode, WallMode::Normal);
        assert!(state.library.selection.is_empty());
    }

    #[test]
    fn a_second_range_unions_into_the_selection() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);
        let _ = state.update(WallMsg::CommitVisual); // 0..=1

        state.library.goto(4);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);
        let _ = state.update(WallMsg::CommitVisual); // 4..=5

        // Editing a selection is re-entering visual: runs accumulate.
        assert_eq!(selected(&state), vec![0, 1, 4, 5]);
    }

    #[test]
    fn x_paints_a_range_that_subtracts() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::SelectAll);

        state.library.goto(1);
        enter_visual(&mut state, RangeOp::Remove);
        nav(&mut state, Dir::Down);
        let _ = state.update(WallMsg::CommitVisual);

        assert_eq!(selected(&state), vec![0, 3, 4, 5]);
        assert_eq!(state.mode, WallMode::Select);
    }

    #[test]
    fn x_does_nothing_with_an_empty_selection() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Remove);
        // Entering a mode whose only possible outcome is a no-op would just
        // strand the user in it.
        assert_eq!(state.mode, WallMode::Normal);
    }

    #[test]
    fn set_edits_are_ignored_mid_paint() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);

        // Each of these edits the committed set, so honouring it would settle
        // the mode and end the range without the user saying so.
        for msg in [
            WallMsg::ToggleCursor,
            WallMsg::SelectAll,
            WallMsg::InvertSelection,
        ] {
            let _ = state.update(msg);
            assert_eq!(
                state.mode,
                WallMode::Visual {
                    anchor: 0,
                    op: RangeOp::Add
                }
            );
            assert!(state.library.selection.is_empty());
        }
    }

    #[test]
    fn escape_from_select_clears_but_leaves_the_cursor() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(3);
        let _ = state.update(WallMsg::ToggleCursor);
        assert_eq!(state.mode, WallMode::Select);

        let _ = state.update(WallMsg::Escape);
        assert!(state.library.selection.is_empty());
        assert_eq!(state.mode, WallMode::Normal);
        // In `Select` the user moves the cursor deliberately, so it stays put.
        assert_eq!(state.library.paths.index(), 3);
    }

    #[test]
    fn visual_over_select_takes_two_escapes_to_leave() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::ToggleCursor); // select 0
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);

        // One press must not discard a selection built up over many moves.
        let _ = state.update(WallMsg::Escape);
        assert_eq!(state.mode, WallMode::Select);
        assert_eq!(selected(&state), vec![0]);

        let _ = state.update(WallMsg::Escape);
        assert_eq!(state.mode, WallMode::Normal);
        assert!(state.library.selection.is_empty());
    }

    #[test]
    fn toggling_the_last_selected_image_returns_to_normal() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::ToggleCursor);
        assert_eq!(state.mode, WallMode::Select);
        let _ = state.update(WallMsg::ToggleCursor);
        // The invariant: no mode bar and no `Select` with nothing selected.
        assert_eq!(state.mode, WallMode::Normal);
    }

    #[test]
    fn select_all_and_invert_move_into_and_out_of_select() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::SelectAll);
        assert_eq!(selected(&state), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(state.mode, WallMode::Select);

        let _ = state.update(WallMsg::InvertSelection);
        assert!(state.library.selection.is_empty());
        assert_eq!(state.mode, WallMode::Normal);
    }

    #[test]
    fn a_selection_survives_a_trip_through_the_single_view() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::ToggleCursor);

        // `w` moves the library across and rebuilds the wall from it; the mode
        // is derived from the selection rather than carried.
        let rebuilt = WallState::new(state.library);
        assert_eq!(rebuilt.mode, WallMode::Select);
        assert_eq!(selected(&rebuilt), vec![0]);
    }

    #[test]
    fn a_painted_range_does_not_survive_that_trip() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);

        let rebuilt = WallState::new(state.library);
        assert_eq!(rebuilt.mode, WallMode::Normal);
        assert!(rebuilt.library.selection.is_empty());
    }

    // --- Batches ---

    fn rotate(state: &mut WallState) {
        let _ = state.update(WallMsg::Rotate { clockwise: true });
    }

    /// Land one file of the running batch.
    fn land(state: &mut WallState, path: PathBuf, result: Result<(), String>) {
        let result = result.map(|()| FileDone::Reshaped);
        let _ = state.update(WallMsg::BatchProgress { path, result });
    }

    /// Land one file of a transfer batch, saying what became of the source.
    fn landed(state: &mut WallState, path: PathBuf, done: FileDone) {
        let _ = state.update(WallMsg::BatchProgress {
            path,
            result: Ok(done),
        });
    }

    fn batch_paths(state: &WallState) -> Vec<PathBuf> {
        state
            .batch
            .as_ref()
            .map(|b| b.pending.iter().cloned().collect())
            .unwrap_or_default()
    }

    #[test]
    fn rotate_in_normal_turns_only_the_cursor_image() {
        let mut state = wall(&[200.0; 6], 1);
        rotate(&mut state);
        assert!(state.batch.is_none());
        // The single-image path claims the file the old way.
        assert_eq!(state.rotating.len(), 1);
    }

    #[test]
    fn rotate_in_select_turns_every_selected_image() {
        let mut state = wall(&[200.0; 40], 1);
        let _ = state.update(WallMsg::SelectAll);
        rotate(&mut state);

        let batch = state.batch.as_ref().expect("batch started");
        assert_eq!(batch.total, 40);
        // Bounded like the decode scheduler: the rest wait their turn rather
        // than all being handed to the runtime at once.
        assert_eq!(batch.in_flight, max_in_flight().min(40));
        assert_eq!(batch.pending.len(), 40 - max_in_flight().min(40));
    }

    #[test]
    fn a_batch_refills_as_each_file_lands() {
        let mut state = wall(&[200.0; 40], 1);
        let _ = state.update(WallMsg::SelectAll);
        rotate(&mut state);

        let first = state.library.paths.iter().next().unwrap().clone();
        let before = batch_paths(&state).len();
        land(&mut state, first, Ok(()));

        let batch = state.batch.as_ref().expect("still running");
        assert_eq!(batch.done, 1);
        // The freed slot was handed the next file, not left idle.
        assert_eq!(batch.pending.len(), before - 1);
        assert_eq!(batch.in_flight, max_in_flight().min(40));
    }

    #[test]
    fn a_batch_ends_when_the_last_file_lands() {
        let mut state = wall(&[200.0; 2], 1);
        let _ = state.update(WallMsg::SelectAll);
        rotate(&mut state);

        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        land(&mut state, paths[0].clone(), Ok(()));
        assert!(state.batch.is_some());
        land(&mut state, paths[1].clone(), Ok(()));
        assert!(state.batch.is_none());
        // Claims released, so the files can be rotated again.
        assert!(state.rotating.is_empty());
    }

    #[test]
    fn a_batch_invalidates_the_thumbnail_of_each_file_it_turns() {
        let mut state = wall(&[200.0; 2], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        state.thumbs.insert(paths[0].clone(), fake_thumb(200));
        let _ = state.update(WallMsg::SelectAll);
        rotate(&mut state);

        land(&mut state, paths[0].clone(), Ok(()));
        assert!(!state.thumbs.contains_key(&paths[0]));
        // 300 wide over a 200-tall thumb becomes 300 * 300/200 = 450 tall.
        assert_eq!(state.ratios.get(&paths[0]), Some(&450.0));
    }

    #[test]
    fn a_batch_does_not_scroll_the_wall_around() {
        let mut state = wall(&[200.0; 40], 1);
        let _ = state.update(WallMsg::SelectAll);
        state.scroll_y = 1000.0;
        rotate(&mut state);

        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        land(&mut state, paths[0].clone(), Ok(()));
        // Revealing per completion would drag the view back to the cursor while
        // the user is watching something else.
        assert_eq!(state.scroll_y, 1000.0);
    }

    #[test]
    fn a_failed_file_does_not_stop_the_batch() {
        let mut state = wall(&[200.0; 3], 1);
        let _ = state.update(WallMsg::SelectAll);
        rotate(&mut state);

        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        land(&mut state, paths[0].clone(), Err("nope".into()));
        let batch = state.batch.as_ref().expect("still running");
        assert_eq!(batch.failed.len(), 1);
        assert_eq!(batch.done, 1);

        land(&mut state, paths[1].clone(), Ok(()));
        land(&mut state, paths[2].clone(), Ok(()));
        assert!(state.batch.is_none());
        // The selection is untouched by a rotate, so a retry is one keypress.
        assert_eq!(selected(&state).len(), 3);
    }

    #[test]
    fn escape_cancels_a_batch_but_lets_the_current_files_land() {
        let mut state = wall(&[200.0; 40], 1);
        let _ = state.update(WallMsg::SelectAll);
        rotate(&mut state);

        let _ = state.update(WallMsg::Escape);
        let batch = state.batch.as_ref().expect("still finishing");
        // Nothing new is dispatched...
        assert!(batch.pending.is_empty());
        assert!(batch.cancelled);
        // ...but abandoning a write half-done is how a photo gets corrupted.
        assert!(batch.in_flight > 0);

        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        for path in paths.iter().take(batch.in_flight) {
            land(&mut state, path.clone(), Ok(()));
        }
        assert!(state.batch.is_none());
    }

    #[test]
    fn escape_during_a_batch_does_not_also_clear_the_selection() {
        let mut state = wall(&[200.0; 40], 1);
        let _ = state.update(WallMsg::SelectAll);
        rotate(&mut state);
        let _ = state.update(WallMsg::Escape);
        // The batch is the top rung: one press stops it and nothing else.
        assert_eq!(selected(&state).len(), 40);
        assert_eq!(state.mode, WallMode::Select);
    }

    #[test]
    fn a_second_batch_cannot_start_while_one_is_running() {
        let mut state = wall(&[200.0; 40], 1);
        let _ = state.update(WallMsg::SelectAll);
        rotate(&mut state);
        let done_before = state.batch.as_ref().unwrap().pending.len();

        rotate(&mut state);
        // Two batches over the same files would race each other's writes.
        assert_eq!(state.batch.as_ref().unwrap().pending.len(), done_before);
    }

    #[test]
    fn rotate_is_ignored_while_painting() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::SelectAll);
        enter_visual(&mut state, RangeOp::Add);
        rotate(&mut state);
        // A half-painted range has no committed meaning, so there is no honest
        // answer to "which images?".
        assert!(state.batch.is_none());
        assert!(state.rotating.is_empty());
    }

    // --- Transfers ---

    fn move_kind() -> BatchKind {
        BatchKind::Transfer {
            kind: TransferKind::Move,
            dest: PathBuf::from("/elsewhere"),
            collision: Collision::Skip,
        }
    }

    fn start(state: &mut WallState, kind: BatchKind, paths: Vec<PathBuf>) {
        let _ = state.update(WallMsg::StartBatch { kind, paths });
    }

    #[test]
    fn a_transfer_runs_over_the_paths_it_was_handed() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        let _ = state.update(WallMsg::SelectAll);

        // The question the user answered named a count, so that count is what
        // runs — not whatever the selection happens to hold by now.
        start(&mut state, move_kind(), paths[..2].to_vec());
        assert_eq!(state.batch.as_ref().unwrap().total, 2);
    }

    #[test]
    fn a_transfer_is_bounded_like_every_other_batch() {
        let mut state = wall(&[200.0; 40], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        start(&mut state, move_kind(), paths);

        let batch = state.batch.as_ref().expect("batch started");
        assert_eq!(batch.in_flight, max_in_flight().min(40));
    }

    #[test]
    fn a_transfer_does_not_claim_files_for_rotation() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        start(&mut state, move_kind(), paths);
        // The rotate claim exists to stop two writes racing one file; a move
        // does not rewrite anything, and claiming would just leak the key.
        assert!(state.rotating.is_empty());
    }

    #[test]
    fn moved_files_are_held_back_until_the_batch_ends() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        start(&mut state, move_kind(), paths[..2].to_vec());

        landed(&mut state, paths[0].clone(), FileDone::Gone);
        // Re-laying the masonry per file would shuffle the wall under whoever
        // is watching it, so the library is untouched until the end.
        assert_eq!(state.library.paths.len(), 6);
        assert_eq!(state.batch.as_ref().unwrap().gone, vec![paths[0].clone()]);

        landed(&mut state, paths[1].clone(), FileDone::Gone);
        assert!(state.batch.is_none());
    }

    #[test]
    fn a_copy_leaves_the_thumbnails_alone() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        state.thumbs.insert(paths[0].clone(), fake_thumb(200));
        start(
            &mut state,
            BatchKind::Transfer {
                kind: TransferKind::Copy,
                dest: PathBuf::from("/elsewhere"),
                collision: Collision::Skip,
            },
            vec![paths[0].clone()],
        );

        landed(&mut state, paths[0].clone(), FileDone::Unchanged);
        // Nothing about the original changed, so re-decoding it would be pure
        // waste — and on a big copy, a wall's worth of it.
        assert!(state.thumbs.contains_key(&paths[0]));
        assert_eq!(state.ratios.get(&paths[0]), Some(&200.0));
    }

    #[test]
    fn a_skipped_move_stays_on_the_wall() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        let _ = state.update(WallMsg::SelectAll);
        start(&mut state, move_kind(), vec![paths[0].clone()]);

        landed(&mut state, paths[0].clone(), FileDone::Unchanged);
        assert!(state.batch.is_none());
        assert_eq!(state.library.paths.len(), 6);
        // Still selected, so answering the question differently retries exactly
        // the files that were skipped.
        assert_eq!(selected(&state).len(), 6);
    }

    #[test]
    fn a_transfer_cannot_start_on_top_of_another_batch() {
        let mut state = wall(&[200.0; 40], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        let _ = state.update(WallMsg::SelectAll);
        rotate(&mut state);

        start(&mut state, move_kind(), paths);
        // Two batches over the same files would fight over them.
        assert!(matches!(
            state.batch.as_ref().unwrap().kind,
            BatchKind::Rotate { .. }
        ));
    }

    #[test]
    fn a_transfer_is_ignored_while_painting() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        let _ = state.update(WallMsg::SelectAll);
        enter_visual(&mut state, RangeOp::Add);

        start(&mut state, move_kind(), paths);
        assert!(state.batch.is_none());
    }

    #[test]
    fn escape_cancels_a_transfer_too() {
        let mut state = wall(&[200.0; 40], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        start(&mut state, move_kind(), paths.clone());

        let _ = state.update(WallMsg::Escape);
        let batch = state.batch.as_ref().expect("still finishing");
        assert!(batch.pending.is_empty());
        // Files already handed to the runtime still land: half a move is a lost
        // photo.
        let in_flight = batch.in_flight;
        assert!(in_flight > 0);
        for path in paths.iter().take(in_flight) {
            landed(&mut state, path.clone(), FileDone::Gone);
        }
        assert!(state.batch.is_none());
    }

    #[test]
    fn the_selection_is_not_operable_while_painting_or_batching() {
        let mut state = wall(&[200.0; 6], 1);
        assert!(state.operable_selection().is_none()); // nothing selected

        let _ = state.update(WallMsg::SelectAll);
        assert_eq!(state.operable_selection().map(|s| s.len()), Some(6));

        enter_visual(&mut state, RangeOp::Add);
        assert!(state.operable_selection().is_none());

        let _ = state.update(WallMsg::Escape);
        rotate(&mut state);
        assert!(state.operable_selection().is_none());
    }

    // --- Favourites and the filter ---

    fn fav(state: &mut WallState) {
        let _ = state.update(WallMsg::ToggleFavourite);
    }

    fn filter(state: &mut WallState) {
        let _ = state.update(WallMsg::ToggleFilter);
    }

    fn starred(state: &WallState) -> Vec<usize> {
        state
            .library
            .all
            .iter()
            .enumerate()
            .filter(|(_, p)| state.library.is_tagged(tags::FAVOURITE, p))
            .map(|(i, _)| i)
            .collect()
    }

    #[test]
    fn f_in_normal_favourites_only_the_cursor_image() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(3);
        fav(&mut state);
        assert_eq!(starred(&state), vec![3]);
        // And again takes it back off.
        fav(&mut state);
        assert!(starred(&state).is_empty());
    }

    #[test]
    fn f_in_select_favourites_the_whole_selection() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);
        let _ = state.update(WallMsg::CommitVisual);

        fav(&mut state);
        assert_eq!(starred(&state), vec![0, 1]);
        // The selection is untouched, so it can be acted on again.
        assert_eq!(selected(&state), vec![0, 1]);
    }

    #[test]
    fn f_is_ignored_while_painting() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);
        fav(&mut state);
        // An uncommitted range has no settled meaning, so there is no honest
        // answer to "which images?".
        assert!(starred(&state).is_empty());
    }

    #[test]
    fn the_filter_narrows_the_wall_to_the_favourites() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(4);
        fav(&mut state);
        state.library.goto(1);
        fav(&mut state);

        filter(&mut state);
        assert_eq!(state.library.paths.len(), 2);
        // The cursor was on a favourite, so it stays on that photo.
        assert_eq!(state.library.current(), &state.library.all[1]);

        filter(&mut state);
        assert_eq!(state.library.paths.len(), 6);
    }

    #[test]
    fn the_filter_is_refused_while_anything_is_selected() {
        let mut state = wall(&[200.0; 6], 1);
        fav(&mut state);
        let _ = state.update(WallMsg::SelectAll);

        filter(&mut state);
        // A filter that hid a selected photo would put images the user cannot
        // see into the next batch. Esc clears the selection in one key.
        assert_eq!(state.library.filter, None);
        assert_eq!(state.library.paths.len(), 6);
    }

    #[test]
    fn the_filter_is_refused_when_nothing_is_favourited() {
        let mut state = wall(&[200.0; 6], 1);
        filter(&mut state);
        // A blank wall would claim the folder was empty.
        assert_eq!(state.library.filter, None);
        assert_eq!(state.library.paths.len(), 6);
    }

    #[test]
    fn unfavouriting_the_last_one_puts_the_whole_folder_back() {
        let mut state = wall(&[200.0; 6], 1);
        fav(&mut state);
        filter(&mut state);
        assert_eq!(state.library.paths.len(), 1);

        fav(&mut state);
        assert_eq!(state.library.filter, None);
        assert_eq!(state.library.paths.len(), 6);
    }

    #[test]
    fn unfavouriting_a_selected_photo_while_filtered_deselects_it() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::SelectAll);
        fav(&mut state);
        let _ = state.update(WallMsg::Escape); // clear, so the filter is allowed
        filter(&mut state);

        state.library.goto(2);
        let _ = state.update(WallMsg::ToggleCursor);
        fav(&mut state);

        // It left the wall, so it must leave the selection with it — otherwise
        // the next batch would touch a photo nobody can see.
        assert_eq!(state.library.paths.len(), 5);
        assert!(state.library.selection.is_empty());
        assert_eq!(state.mode, WallMode::Normal);
    }

    #[test]
    fn the_star_is_hidden_while_the_wall_is_already_all_favourites() {
        let mut state = wall(&[200.0; 6], 1);
        let path = state.library.current().clone();
        fav(&mut state);
        assert!(state.tile_look(0, &path, 0).star);

        filter(&mut state);
        // Every tile would carry one, which says nothing.
        assert!(!state.tile_look(0, &path, 0).star);
    }

    #[test]
    fn a_favourite_that_is_also_selected_shows_both_marks() {
        let mut state = wall(&[200.0; 6], 1);
        let path = state.library.current().clone();
        fav(&mut state);
        let _ = state.update(WallMsg::ToggleCursor);

        let look = state.tile_look(0, &path, 0);
        assert!(look.star);
        assert!(look.badge);
    }

    // --- Removal ---

    #[test]
    fn removing_images_drops_them_from_the_wall() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        state.thumbs.insert(paths[1].clone(), fake_thumb(200));
        let _ = state.update(WallMsg::SelectAll);

        let gone: HashSet<PathBuf> = [paths[1].clone()].into_iter().collect();
        let (alive, _) = state.removed(&gone);

        assert!(alive);
        assert_eq!(state.library.paths.len(), 5);
        // Nothing cached about a file that no longer exists.
        assert!(!state.thumbs.contains_key(&paths[1]));
        assert!(!state.ratios.contains_key(&paths[1]));
        assert_eq!(selected(&state).len(), 5);
    }

    #[test]
    fn removing_the_whole_selection_returns_to_normal() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        state.library.goto(2);
        let _ = state.update(WallMsg::ToggleCursor);

        let gone: HashSet<PathBuf> = [paths[2].clone()].into_iter().collect();
        let (alive, _) = state.removed(&gone);

        assert!(alive);
        assert!(state.library.selection.is_empty());
        assert_eq!(state.mode, WallMode::Normal);
    }

    #[test]
    fn removing_everything_reports_an_empty_wall() {
        let mut state = wall(&[200.0; 3], 1);
        let gone: HashSet<PathBuf> = state.library.paths.iter().cloned().collect();
        let (alive, _) = state.removed(&gone);
        // The caller has to fall back to the empty screen; a `PointedList`
        // cannot hold nothing.
        assert!(!alive);
    }

    #[test]
    fn a_decode_of_a_deleted_file_is_discarded() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        state.in_flight.insert(paths[1].clone());

        let gone: HashSet<PathBuf> = [paths[1].clone()].into_iter().collect();
        let _ = state.removed(&gone);
        assert!(state.stale.contains(&paths[1]));
    }

    // --- Mouse ---

    const PLAIN: Modifiers = Modifiers::NONE;

    fn click(state: &mut WallState, index: usize, modifiers: Modifiers) -> Click {
        state.click(index, modifiers)
    }

    fn opens(outcome: Click) -> bool {
        matches!(outcome, Click::Open)
    }

    #[test]
    fn a_plain_click_still_opens_in_normal() {
        let mut state = wall(&[200.0; 6], 1);
        assert!(opens(click(&mut state, 3, PLAIN)));
        // Nothing selected, and no mode change: the pre-selection behaviour is
        // untouched for anyone who never presses `v`.
        assert!(state.library.selection.is_empty());
        assert_eq!(state.mode, WallMode::Normal);
    }

    #[test]
    fn cmd_click_selects_without_the_keyboard() {
        let mut state = wall(&[200.0; 6], 1);
        assert!(!opens(click(&mut state, 3, Modifiers::COMMAND)));
        assert_eq!(selected(&state), vec![3]);
        assert_eq!(state.mode, WallMode::Select);
        assert_eq!(state.library.paths.index(), 3);
    }

    #[test]
    fn a_plain_click_selects_once_a_selection_is_live() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 1, Modifiers::COMMAND);
        // Now in `Select`, so the mouse is modal too: no modifier needed.
        assert!(!opens(click(&mut state, 4, PLAIN)));
        assert_eq!(selected(&state), vec![1, 4]);
    }

    #[test]
    fn a_plain_click_deselects_a_selected_tile() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 1, Modifiers::COMMAND);
        let _ = click(&mut state, 4, PLAIN);
        let _ = click(&mut state, 1, PLAIN);
        assert_eq!(selected(&state), vec![4]);
    }

    #[test]
    fn deselecting_the_last_tile_returns_to_normal() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 2, Modifiers::COMMAND);
        let _ = click(&mut state, 2, PLAIN);
        assert_eq!(state.mode, WallMode::Normal);
        // And clicks open again, as they did before.
        assert!(opens(click(&mut state, 2, PLAIN)));
    }

    #[test]
    fn shift_click_extends_from_the_cursor() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(1);
        assert!(!opens(click(&mut state, 4, Modifiers::SHIFT)));
        assert_eq!(selected(&state), vec![1, 2, 3, 4]);
    }

    #[test]
    fn shift_click_extends_from_the_last_click() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 1, Modifiers::COMMAND);
        // Every click moves the cursor, so the run starts where the last click
        // landed rather than wherever the keyboard was left.
        let _ = click(&mut state, 3, Modifiers::SHIFT);
        assert_eq!(selected(&state), vec![1, 2, 3]);
    }

    #[test]
    fn a_click_moves_the_cursor_so_v_anchors_there() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 4, Modifiers::COMMAND);
        enter_visual(&mut state, RangeOp::Add);
        assert_eq!(
            state.mode,
            WallMode::Visual {
                anchor: 4,
                op: RangeOp::Add
            }
        );
    }

    #[test]
    fn a_click_while_painting_extends_the_range() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        assert!(!opens(click(&mut state, 3, PLAIN)));

        // A motion, not a set edit: the range is still being painted.
        assert_eq!(
            state.mode,
            WallMode::Visual {
                anchor: 0,
                op: RangeOp::Add
            }
        );
        assert!(state.library.selection.is_empty());
        assert_eq!(state.library.paths.index(), 3);

        let _ = state.update(WallMsg::CommitVisual);
        assert_eq!(selected(&state), vec![0, 1, 2, 3]);
    }

    #[test]
    fn a_double_click_opens_and_leaves_the_selection_alone() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 0, Modifiers::COMMAND);
        // First of the pair selects 3...
        assert!(!opens(click(&mut state, 3, PLAIN)));
        assert_eq!(selected(&state), vec![0, 3]);
        // ...and the second toggles it back off, then opens it.
        assert!(opens(click(&mut state, 3, PLAIN)));
        assert_eq!(selected(&state), vec![0]);
    }

    #[test]
    fn two_clicks_on_different_tiles_are_not_a_double_click() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 0, Modifiers::COMMAND);
        assert!(!opens(click(&mut state, 3, PLAIN)));
        assert!(!opens(click(&mut state, 4, PLAIN)));
        assert_eq!(selected(&state), vec![0, 3, 4]);
    }

    #[test]
    fn a_slow_second_click_is_not_a_double_click() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 0, Modifiers::COMMAND);
        let _ = click(&mut state, 3, PLAIN);
        // Age the first click past the threshold.
        state.last_click = Some((3, Instant::now() - DOUBLE_CLICK * 2));
        assert!(!opens(click(&mut state, 3, PLAIN)));
        assert_eq!(selected(&state), vec![0]);
    }

    #[test]
    fn a_third_click_starts_a_fresh_pair() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 0, Modifiers::COMMAND);
        let _ = click(&mut state, 3, PLAIN); // select 3
        let _ = click(&mut state, 3, PLAIN); // double: deselect, opens
        // Without clearing the record, this would read as another double.
        assert!(!opens(click(&mut state, 3, PLAIN)));
        assert_eq!(selected(&state), vec![0, 3]);
    }

    #[test]
    fn a_click_past_the_end_of_the_library_is_ignored() {
        let mut state = wall(&[200.0; 6], 1);
        assert!(!opens(click(&mut state, 99, Modifiers::COMMAND)));
        assert!(state.library.selection.is_empty());
        assert_eq!(state.library.paths.index(), 0);
    }

    // --- Tile decoration ---

    #[test]
    fn the_cursor_ring_is_hidden_while_painting() {
        let mut state = wall(&[200.0; 6], 1);
        let path = state.library.current().clone();
        assert_eq!(state.tile_look(0, &path, 0).ring, Some(Accent::Cursor));

        enter_visual(&mut state, RangeOp::Add);
        // In `Visual` the leading edge of the range already shows the cursor;
        // a second highlight competing with the tint just reads as noise.
        assert_eq!(state.tile_look(0, &path, 0).ring, Some(Accent::Select));
    }

    #[test]
    fn a_pending_range_is_tinted_more_faintly_than_a_committed_one() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);

        let pending = state.tile_look(1, &paths[1], 1);
        assert_eq!(pending.tint, Some(Accent::Select));
        assert_eq!(pending.tint_alpha, TINT_PENDING);
        assert!(!pending.badge);

        let _ = state.update(WallMsg::CommitVisual);
        let committed = state.tile_look(1, &paths[1], 1);
        assert_eq!(committed.tint_alpha, TINT_COMMITTED);
        assert!(committed.badge);
    }

    #[test]
    fn a_tile_pending_removal_keeps_its_badge() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        let _ = state.update(WallMsg::SelectAll);
        enter_visual(&mut state, RangeOp::Remove);

        let look = state.tile_look(0, &paths[0], 0);
        assert_eq!(look.tint, Some(Accent::Remove));
        // Still selected until the range is committed; saying otherwise would
        // pre-empt the user.
        assert!(look.badge);
    }

    #[test]
    fn tiles_outside_the_range_are_undecorated() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        enter_visual(&mut state, RangeOp::Add);

        let look = state.tile_look(3, &paths[3], 0);
        assert_eq!(look.ring, None);
        assert_eq!(look.tint, None);
        assert!(!look.badge);
    }

    #[test]
    fn the_bar_counts_what_committing_would_leave_selected() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::ToggleCursor); // select 0

        state.library.goto(2);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down); // range 2..=3, neither selected
        assert_eq!(state.pending_counts(2, RangeOp::Add), (3, 2));
    }

    #[test]
    fn the_bar_does_not_double_count_an_overlapping_range() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::SelectAll);
        state.library.goto(1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);
        // Everything in 1..=2 is already selected: committing changes nothing.
        assert_eq!(state.pending_counts(1, RangeOp::Add), (6, 0));
    }

    #[test]
    fn the_bar_counts_a_removal_downwards() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::SelectAll);
        state.library.goto(1);
        enter_visual(&mut state, RangeOp::Remove);
        nav(&mut state, Dir::Down);
        assert_eq!(state.pending_counts(1, RangeOp::Remove), (4, -2));
    }

    /// A 1x1 stand-in for a decoded thumbnail of the given height.
    fn fake_thumb(height: u32) -> ThumbState {
        ThumbState {
            handle: image::Handle::from_rgba(1, 1, vec![0, 0, 0, 255]),
            height,
        }
    }

    /// Every slot as comparable data, for before/after layout diffs.
    fn placements(layout: &WallLayout) -> Vec<(usize, usize, usize, f32)> {
        let mut out: Vec<_> = layout
            .slots
            .iter()
            .map(|(i, s)| (*i, s.col, s.row, s.y))
            .collect();
        out.sort_by_key(|p| p.0);
        out
    }

    #[test]
    fn rotating_swaps_the_aspect_and_drops_the_thumbnail() {
        let mut state = wall(&SPREAD, 3);
        let path = state.library.current().clone();
        state.thumbs.insert(path.clone(), fake_thumb(200));

        let _ = state.update(WallMsg::Rotated {
            path: path.clone(),
            result: Ok(()),
        });

        // The cached decode is pre-rotation pixels, so it must go.
        assert!(!state.thumbs.contains_key(&path));
        // 300 wide over a 200-tall thumb becomes 300 * 300/200 = 450 tall.
        assert_eq!(state.ratios.get(&path), Some(&450.0));
        assert!(state.rotating.is_empty());
    }

    #[test]
    fn a_failed_rotate_leaves_the_thumbnail_alone() {
        let mut state = wall(&SPREAD, 3);
        let path = state.library.current().clone();
        state.thumbs.insert(path.clone(), fake_thumb(200));

        let _ = state.update(WallMsg::Rotated {
            path: path.clone(),
            result: Err("nope".into()),
        });

        assert!(state.thumbs.contains_key(&path));
        assert_eq!(state.ratios.get(&path), Some(&200.0));
        assert!(state.rotating.is_empty());
    }

    #[test]
    fn rotation_only_moves_slots_at_or_after_it() {
        let mut state = wall(&SPREAD, 3);
        let before = placements(&state.layout(width_for(3)));

        // Rotate index 2 — a 200-tall tile, so the swap to 450 actually
        // changes something (a 300-tall one is square and would not).
        let path = state.library.paths.iter().nth(2).unwrap().clone();
        state.library.goto(2);
        let _ = state.update(WallMsg::Rotated {
            path,
            result: Ok(()),
        });
        let after = placements(&state.layout(width_for(3)));

        // Everything the masonry placed before it is untouched: index `i` is
        // positioned from the column heights left by indices < `i`.
        assert_eq!(before[..2], after[..2]);
        // And it did actually reflow below, or this would prove nothing.
        assert_ne!(before[2..], after[2..]);
    }

    #[test]
    fn a_decode_landing_after_a_rotate_is_discarded() {
        let mut state = wall(&SPREAD, 3);
        let path = state.library.current().clone();
        state.in_flight.insert(path.clone());

        let _ = state.update(WallMsg::Rotated {
            path: path.clone(),
            result: Ok(()),
        });
        assert!(state.stale.contains(&path));

        // That decode began before the write, so its pixels are the old ones.
        let _ = state.update(WallMsg::ThumbDecoded {
            path: path.clone(),
            result: Ok((fake_thumb(200).handle, 200)),
        });
        assert!(!state.thumbs.contains_key(&path));
        assert!(state.stale.is_empty());
        // Left un-cached, so the same `schedule` that frees the slot puts it
        // straight back in flight — now reading the rotated file.
        assert!(state.in_flight.contains(&path));
    }

    #[test]
    fn a_decode_landing_without_a_rotate_is_cached() {
        let mut state = wall(&SPREAD, 3);
        let path = state.library.current().clone();
        state.in_flight.insert(path.clone());

        let _ = state.update(WallMsg::ThumbDecoded {
            path: path.clone(),
            result: Ok((fake_thumb(200).handle, 200)),
        });
        assert!(state.thumbs.contains_key(&path));
    }

    #[test]
    fn a_second_rotate_is_ignored_while_one_is_writing() {
        let mut state = wall(&SPREAD, 3);
        let path = state.library.current().clone();

        assert_eq!(state.claim_rotate(), Some(path.clone()));
        // Holding `r` down must not race two writes against the same file.
        assert_eq!(state.claim_rotate(), None);

        // Once the write lands the claim is released.
        let _ = state.update(WallMsg::Rotated {
            path: path.clone(),
            result: Ok(()),
        });
        assert_eq!(state.claim_rotate(), Some(path));
    }

    #[test]
    fn scrolling_moves_the_decode_focus() {
        // One column, so list order and visual order coincide.
        let mut state = wall(&[200.0; 20], 1);
        assert_eq!(state.focus, 0.0);
        let _ = state.update(WallMsg::Scrolled(2000.0));
        // 2000 + 500 (half the 1000px viewport) lands well down the column.
        assert!(state.focus > 5.0, "focus was {}", state.focus);
    }

    #[test]
    fn prioritises_thumbnails_nearest_the_focus() {
        let paths = paths(10);
        let refs: Vec<&PathBuf> = paths.iter().collect();
        // Focused midway (4.5): 4 and 5 are closest (0.5 each), then 3 (1.5,
        // pushed before 6 so it wins the tie).
        let chosen = prioritise(&refs, |_| true, 4.5, 3);
        assert_eq!(chosen, vec![&paths[4], &paths[5], &paths[3]]);
    }

    #[test]
    fn decodes_top_down_from_the_top() {
        let paths = paths(6);
        let refs: Vec<&PathBuf> = paths.iter().collect();
        let chosen = prioritise(&refs, |_| true, 0.0, 3);
        assert_eq!(chosen, vec![&paths[0], &paths[1], &paths[2]]);
    }

    #[test]
    fn skips_already_decoded_or_in_flight() {
        let paths = paths(6);
        let refs: Vec<&PathBuf> = paths.iter().collect();
        // 0 already done: from the top, the next two pending are 1 and 2.
        let chosen = prioritise(&refs, |p| p != &paths[0], 0.0, 2);
        assert_eq!(chosen, vec![&paths[1], &paths[2]]);
    }

    #[test]
    fn free_zero_yields_nothing() {
        let paths = paths(4);
        let refs: Vec<&PathBuf> = paths.iter().collect();
        let chosen = prioritise(&refs, |_| true, 0.0, 0);
        assert!(chosen.is_empty());
    }
}
