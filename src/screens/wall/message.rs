//! What the wall can be told. Every variant is produced only while the wall is
//! on screen, and routed to `WallState::update` by `App::update`.

use std::path::PathBuf;

use iced::widget::image;
use iced::Size;

use crate::library::RangeOp;

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
