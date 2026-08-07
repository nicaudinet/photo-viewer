//! The decode scheduler: which thumbnails to decode next, and how many at once.
//!
//! A wall of N images does not fire N tasks up front. At most
//! [`max_in_flight`] decodes run at a time, refilled as each lands, and the
//! ones nearest what you are looking at go first.

use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::OnceLock;

use iced::widget::image;
use iced::Task;

use crate::Message;

use super::layout::THUMB_WIDTH;
use super::message::WallMsg;
use super::WallState;

/// Max thumbnail decodes running at once. A wall of N images no longer fires N
/// tasks up front; the scheduler keeps at most this many in flight and refills
/// as each lands (see [`WallState::schedule`]). Sized to the CPU count, since
/// decode is CPU-bound; bounding it caps thrash and peak memory.
pub(super) fn max_in_flight() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    })
}

/// A decoded thumbnail: the RGBA handle plus its scaled pixel size (needed for
/// masonry column-height bookkeeping).
pub(super) struct ThumbState {
    pub(super) handle: image::Handle,
    pub(super) height: u32,
}

impl WallState {
    /// Read the library's thumbnail dimensions from their headers, off-thread.
    /// One task for the whole library: the cost is per-file IO, and a single
    /// task keeps it from competing with the decode scheduler.
    pub(super) fn load_ratios(&self) -> Task<Message> {
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
    pub(super) fn needs_decode(&self, path: &PathBuf) -> bool {
        !self.thumbs.contains_key(path) && !self.in_flight.contains(path)
    }
}

/// Order `paths` for decode and return the first `free`.
///
/// `pending` marks which paths still need decoding; those are taken nearest
/// `focus` (a position in the list) first. Priority is by list position, not
/// pixel height, so a decode landing never reshuffles it. The sort is stable,
/// so equal-distance items keep list order.
pub(super) fn prioritise<'a>(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::wall::fixture::*;
    use std::path::PathBuf;

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
