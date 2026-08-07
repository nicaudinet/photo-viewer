//! The wall view: async thumbnails laid out shortest-column masonry, with
//! keyboard navigation, rotate, click-to-open, a modal selection, and the
//! operations over it — favourite, move, copy, trash.
//!
//! Selection is vim-shaped: [`WallMode::Normal`] moves a cursor,
//! [`WallMode::Visual`] paints a range as the cursor moves, and
//! [`WallMode::Select`] holds the committed set. See `SELECT_MODE_PLAN.md`.
//!
//! This module owns [`WallState`] and the handful of things `App` calls on it.
//! Everything else is one concern per file:
//!
//! | module | what it owns |
//! |---|---|
//! | [`message`] | [`WallMsg`], the wall's whole input vocabulary |
//! | [`update`] | where a message turns into state and tasks |
//! | [`layout`] | the masonry, as pure data |
//! | [`navigate`] | moving the cursor and keeping it on screen |
//! | [`select`] | [`WallMode`] and the transitions between modes |
//! | [`keys`] | the wall's keyboard |
//! | [`mouse`] | clicks on a thumbnail |
//! | [`thumbs`] | the bounded decode scheduler |
//! | [`rotate`] | rotating one image, and invalidating what it changed |
//! | [`queue`] | an operation over the whole selection, as a plain queue |
//! | [`batch`] | driving that queue from the wall |
//! | [`ops`] | what that operation does to one file |
//! | [`view`], [`bar`], [`tile`] | the widget tree |

mod bar;
mod batch;
mod keys;
mod layout;
mod message;
mod mouse;
mod navigate;
mod ops;
mod queue;
mod rotate;
mod select;
mod thumbs;
mod tile;
mod update;
mod view;

#[cfg(test)]
mod fixture;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use iced::{Size, Task};

use crate::core::library::Library;
use crate::Message;

pub(crate) use keys::WallKey;
pub(crate) use message::WallMsg;
pub(crate) use mouse::Click;
pub(crate) use navigate::measure;
pub(crate) use queue::BatchKind;
pub(crate) use select::WallMode;
pub(crate) use tile::favourite_star;

use queue::Batch;
use thumbs::ThumbState;

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

    /// Everything the wall needs on entry: measure the viewport, read the
    /// thumbnail dimensions, and start decoding.
    pub(crate) fn enter(&mut self) -> Task<Message> {
        Task::batch([measure(), self.load_ratios(), self.schedule()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::wall::fixture::*;
    use crate::screens::wall::message::WallMsg;

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
}
