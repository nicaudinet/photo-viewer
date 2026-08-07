//! What the single view can be told. Every action here applies to the current
//! image and nothing else.

use std::path::PathBuf;

use iced::widget::image;

/// Messages produced only while the single view is on screen. Routed to
/// [`SingleState::update`] by `App::update`.
#[derive(Debug, Clone)]
pub(crate) enum SingleMsg {
    Next,
    Prev,
    /// `r`: rotate the current image anticlockwise, writing it to disk.
    RotateAnticlockwise,
    /// `Shift+R`: rotate the current image clockwise, writing it to disk.
    RotateClockwise,
    /// `f`: favourite the current image — and, as with everything on this
    /// screen, only the current image, whatever else is selected.
    ToggleFavourite,
    /// Result of a rotate: on success, re-decode the (now rotated) file.
    /// Carries its own path — the view may have moved on while the write was
    /// in flight.
    Rotated {
        path: PathBuf,
        result: Result<(), String>,
    },
    /// The tag store was written. Only ever reported when it wasn't.
    TagsSaved(Result<(), String>),
    /// A fit-to-window decode landed, tagged with the generation it began at.
    LargeDecoded {
        generation: u64,
        result: Result<image::Handle, String>,
    },
}
