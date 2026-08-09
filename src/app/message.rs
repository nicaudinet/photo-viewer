//! Everything the app can be told.
//!
//! Several keys are ambiguous — the same press means one thing on the wall and
//! another in the single view — and `subscription` cannot see which screen is
//! live. Those stay neutral here and are disambiguated in [`super::update`].

use std::path::PathBuf;

use iced::keyboard;

use crate::screens::single::SingleMsg;
use crate::screens::wall::WallMsg;

use super::destination::TransferPlan;

#[derive(Debug, Clone)]
pub(crate) enum Message {
    // Global — handled at the top level regardless of screen.
    /// `q` — ask before leaving. `Quit` is what a yes to that question sends.
    AskQuit,
    Quit,
    ToggleFullscreen,
    ToggleHelp,
    /// The first frame has rendered; reveal the (initially hidden) window.
    WindowReady,
    /// A modifier key went down or up; remembered for the next click.
    ModifiersChanged(keyboard::Modifiers),
    /// A raw key press, for [`super::keys`] to offer to the app and then to
    /// whichever screen is live. The app has no opinion about most keys, so it
    /// has no word for them either.
    Key(keyboard::Event),

    /// The folder picker closed, and the destination has been looked at.
    /// `None` if the picker was cancelled.
    TransferTarget(Option<TransferPlan>),
    /// Trash these files. Named by the question the user answered, not re-read
    /// from the selection, so what happens is what they were shown.
    Trash(Vec<PathBuf>),
    /// Images have left the library's folder — trashed, or moved elsewhere.
    /// Carries what actually went, so a failure leaves its image on the wall.
    Removed {
        gone: Vec<PathBuf>,
        failed: Vec<(PathBuf, String)>,
    },
    /// A key naming one of the answers on offer.
    ConfirmChoice(char),
    /// `n` — decline, whatever was asked, or dismiss a statement.
    ConfirmNo,
    /// The window resized: re-measure the wall's viewport if it is on screen.
    /// The event's own size is the *window's*, not the scroll viewport's, so it
    /// is only a trigger — `screens::wall::measure` reads the real number back.
    WallMeasure,

    // Transitions between screens — owned by `App` because a per-screen update
    // can't reassign `self.screen`.
    /// `w` from the single view: back out to the wall the photo came from.
    /// There is no key the other way — the wall opens a photograph by naming
    /// it, with Enter or a click.
    ShowWall,
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
