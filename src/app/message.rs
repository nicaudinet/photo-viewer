//! Everything the app can be told.
//!
//! Several keys are ambiguous — the same press means one thing on the wall and
//! another in the single view — and `subscription` cannot see which screen is
//! live. Those stay neutral here and are disambiguated in [`super::update`].

use std::path::PathBuf;

use iced::keyboard;

use crate::core::library::RangeOp;
use crate::core::transfer::TransferKind;
use crate::screens::single::SingleMsg;
use crate::screens::wall::{Dir, WallMsg};

use super::destination::TransferPlan;

#[derive(Debug, Clone)]
pub(crate) enum Message {
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
    /// `f`: favourite the selection (wall) or the current image (single).
    ToggleFavourite,
    /// `Shift+F`: show only the favourites, or show everything again.
    ToggleFilter,
    /// The tag store was written. Only ever reported when it wasn't.
    TagsSaved(Result<(), String>),
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
    /// is only a trigger — `screens::wall::measure` reads the real number back.
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
