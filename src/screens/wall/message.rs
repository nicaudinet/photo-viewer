//! What the wall can be told. Every variant is produced only while the wall is
//! on screen, and routed to `WallState::update` by `App::update`.

use std::path::PathBuf;

use iced::widget::image;
use iced::Size;

use crate::core::fingerprint_cache::Entry;
use crate::core::library::RangeOp;

use super::queue::{BatchKind, FileDone};

/// A keyboard navigation direction, shared with the single view (where left and
/// right mean previous and next).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dir {
    Up,
    Down,
    Left,
    Right,
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
    /// `a`.
    SelectAll,
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
    /// `g`: stack runs of near-identical photos, or take the stacks apart.
    ToggleGrouping,
    /// One photo of the fingerprint pass has been hashed. `None` if it could
    /// not be read — the pass has to hear about that too, or it never ends.
    Fingerprinted { path: PathBuf, entry: Option<Entry> },
    /// The fingerprint cache was written. Only ever reported when it wasn't.
    FingerprintsSaved(Result<(), String>),
    /// `+` / `-`: change how alike two photos have to be to stack.
    Retune { looser: bool },
    /// Enter, or a click, on a stack: open a wall over the photos in it.
    Descend { index: usize },
    /// The tag store was written. Only ever reported when it wasn't.
    TagsSaved(Result<(), String>),
    /// One file of a batch finished.
    BatchProgress {
        path: PathBuf,
        result: Result<FileDone, String>,
    },
}
