//! `g`: the fingerprint pass, and the stacks it turns into.
//!
//! Hashing a folder is the only expensive part of grouping, so it happens once,
//! here, and everything after it is a pass over numbers already in hand — see
//! `GROUP_MODE_PLAN.md`. The pass is shaped like the decode scheduler and the
//! batch queue before it: bounded work in flight, refilled as each result
//! lands, with how far along it is in the mode bar and Esc to stop it.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use iced::Task;

use crate::core::fingerprint::Fingerprint;
use crate::core::fingerprint_cache::{self, Cache, Entry};
use crate::core::grouping::Grouping;
use crate::Message;

use super::message::WallMsg;
use super::thumbs::max_in_flight;
use super::WallState;

/// A run over one folder's photos, hashing whatever the cache cannot answer.
///
/// Holds the cache because it is the one thing that must not be shared: photos
/// are hashed on every core at once, and the answers are folded in here, on the
/// thread that owns the wall.
pub(super) struct Hashing {
    cache: Cache,
    /// Photos still to hash — the cache had nothing fresh for these.
    pending: VecDeque<PathBuf>,
    /// What is known so far, cache hits included.
    prints: HashMap<PathBuf, Fingerprint>,
    in_flight: usize,
    done: usize,
    /// How many photos have to be hashed, which is *not* how many there are:
    /// the second `g` in a folder has almost nothing left to do, and a bar
    /// counting to the whole folder would look stuck at the end.
    total: usize,
    cancelled: bool,
}

impl Hashing {
    /// Begin a pass over `photos`, answering from `cache` everything it can.
    pub(super) fn new(cache: Cache, photos: &[PathBuf]) -> Self {
        let mut prints = HashMap::new();
        let mut pending = VecDeque::new();
        for path in photos {
            match cache.get(path) {
                Some(print) => {
                    prints.insert(path.clone(), print);
                }
                None => pending.push_back(path.clone()),
            }
        }
        Self {
            cache,
            total: pending.len(),
            pending,
            prints,
            in_flight: 0,
            done: 0,
            cancelled: false,
        }
    }

    /// Take the next photos to hash, up to a total of `cap` at once.
    pub(super) fn claim(&mut self, cap: usize) -> Vec<PathBuf> {
        if self.cancelled {
            return Vec::new();
        }
        let free = cap.saturating_sub(self.in_flight);
        let chosen: Vec<PathBuf> = (0..free).map_while(|_| self.pending.pop_front()).collect();
        self.in_flight += chosen.len();
        chosen
    }

    /// Fold one hashed photo in. `None` is a photo that could not be read, and
    /// it is simply not recorded — nothing is known about it, so it stacks with
    /// nothing.
    pub(super) fn record(&mut self, path: &Path, entry: Option<Entry>) {
        self.in_flight = self.in_flight.saturating_sub(1);
        self.done += 1;
        if let Some(entry) = entry {
            self.cache.remember(path, entry);
            self.prints.insert(path.to_path_buf(), entry.print());
        }
    }

    pub(super) fn is_finished(&self) -> bool {
        self.in_flight == 0 && (self.pending.is_empty() || self.cancelled)
    }

    /// Stop handing out photos. The ones already out still land — they cost
    /// nothing now and are worth caching.
    pub(super) fn cancel(&mut self) {
        self.cancelled = true;
        self.pending.clear();
    }

    /// Consume a finished pass: the cache to write back, and the fingerprints
    /// to group by.
    ///
    /// A cancelled pass yields no fingerprints. Half a folder's worth would
    /// stack the photos that happened to finish and leave the rest looking
    /// unrelated, which is a worse answer than not grouping at all — but what it
    /// did hash is still cached, so pressing `g` again picks up where it left
    /// off.
    pub(super) fn finish(self) -> (Cache, Option<HashMap<PathBuf, Fingerprint>>) {
        let prints = (!self.cancelled).then_some(self.prints);
        (self.cache, prints)
    }

    /// What to show while this is running.
    pub(super) fn label(&self) -> String {
        format!("Hashing {}/{}", self.done, self.total)
    }
}

impl WallState {
    /// `g`: stack runs of near-identical photos, or take the stacks apart.
    ///
    /// Refused while painting, for the reason everything is — an uncommitted
    /// range has no settled meaning — and while a batch is running, so that the
    /// wall is never re-laid under files that are being written.
    pub(super) fn toggle_grouping(&mut self) -> Task<Message> {
        if self.is_visual() || self.batch.is_some() {
            return Task::none();
        }
        // Pressed again mid-pass, `g` stops it rather than starting a second
        // one over the same folder.
        if self.hashing.is_some() {
            return self.cancel_hashing();
        }
        if self.library.grouping.is_some() {
            self.grouping = self.library.set_grouping(None);
            return self.regrouped();
        }
        // Grouped once already in this folder: the fingerprints — and the rung
        // the user left the dial on — are still in hand.
        if let Some(grouping) = self.grouping.take() {
            self.library.set_grouping(Some(grouping));
            return self.regrouped();
        }
        self.start_hashing()
    }

    /// `+` / `-`: change how alike two photos have to be, and re-chain.
    ///
    /// The hashes are already in hand, so this is a pass over numbers — which is
    /// the whole reason the dial can be turned live rather than guessed at once.
    pub(super) fn retune(&mut self, looser: bool) -> Task<Message> {
        if self.is_visual() || self.library.grouping.is_none() {
            return Task::none();
        }
        if looser {
            self.library.loosen();
        } else {
            self.library.tighten();
        }
        self.regrouped()
    }

    fn start_hashing(&mut self) -> Task<Message> {
        let photos: Vec<PathBuf> = self.library.photos().cloned().collect();
        let cache = Cache::load(&self.library.image_dir, &photos);
        let hashing = Hashing::new(cache, &photos);
        // Nothing to wait for: every photo was in the cache.
        if hashing.is_finished() {
            self.hashing = Some(hashing);
            return self.finish_hashing();
        }
        self.hashing = Some(hashing);
        // The bar has appeared, so the wall has less room than it had.
        Task::batch([self.refill_hashing(), self.remeasure()])
    }

    /// Hand the runtime the next photos to hash, and no more.
    fn refill_hashing(&mut self) -> Task<Message> {
        let Some(hashing) = &mut self.hashing else {
            return Task::none();
        };
        let tasks: Vec<Task<Message>> = hashing
            .claim(max_in_flight())
            .into_iter()
            .map(|path| {
                let key = path.clone();
                Task::perform(fingerprint_cache::take_async(path), move |entry| {
                    Message::Wall(WallMsg::Fingerprinted {
                        path: key.clone(),
                        entry,
                    })
                })
            })
            .collect();
        Task::batch(tasks)
    }

    /// One photo has been hashed: fold it in, refill the slot it freed, and
    /// group if it was the last.
    ///
    /// Deliberately no re-measure per photo. The bar redraws from state as it
    /// is, and the wall itself has not moved — nothing here changes what is on
    /// it until the pass is over.
    pub(super) fn fingerprinted(&mut self, path: PathBuf, entry: Option<Entry>) -> Task<Message> {
        let Some(hashing) = &mut self.hashing else {
            return Task::none();
        };
        hashing.record(&path, entry);
        if hashing.is_finished() {
            return self.finish_hashing();
        }
        self.refill_hashing()
    }

    /// Stop hashing. Photos already handed out still land, so a pass with
    /// nothing in flight ends here rather than waiting for a result that will
    /// never come.
    pub(super) fn cancel_hashing(&mut self) -> Task<Message> {
        let Some(hashing) = &mut self.hashing else {
            return Task::none();
        };
        hashing.cancel();
        if hashing.is_finished() {
            return self.finish_hashing();
        }
        Task::none()
    }

    /// Take the finished pass down, write what it learned, and group by it.
    fn finish_hashing(&mut self) -> Task<Message> {
        let Some(hashing) = self.hashing.take() else {
            return Task::none();
        };
        let (cache, prints) = hashing.finish();

        // Saved whether or not the pass was cancelled: a hash already taken is
        // worth keeping, and the next `g` then has that much less to do.
        let save = Task::perform(
            fingerprint_cache::save_async(self.library.image_dir.clone(), cache),
            |result| Message::Wall(WallMsg::FingerprintsSaved(result)),
        );
        let group = match prints {
            Some(prints) => {
                self.library.set_grouping(Some(Grouping::new(prints)));
                self.regrouped()
            }
            // Cancelled, so the wall is unchanged — but the bar has to go.
            None => self.remeasure(),
        };
        Task::batch([save, group])
    }

    /// Everything the wall redoes after the tiles change. The same work a
    /// filter earns, for the same reason: the masonry has a different number of
    /// tiles to place, and the cursor may have landed somewhere else.
    fn regrouped(&mut self) -> Task<Message> {
        Task::batch([self.resettled(), self.remeasure()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::library::RangeOp;
    use crate::screens::wall::fixture::*;
    use crate::screens::wall::select::WallMode;

    #[test]
    fn g_hashes_the_folder_and_stacks_it() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 1, 2, 40, 41, 42]);

        // Two runs of three, so two tiles standing for six photos.
        assert_eq!(state.library.paths.len(), 2);
        assert_eq!(state.library.photos().count(), 6);
        assert!(state.hashing.is_none());
    }

    #[test]
    fn nothing_alike_leaves_the_wall_as_it_was() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 12, 24, 36, 48, 60]);
        assert_eq!(state.library.paths.len(), 6);
        // Grouping is still on, though: the dial is there to be turned.
        assert!(state.library.grouping.is_some());
    }

    #[test]
    fn the_second_press_puts_every_photo_back() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 1, 2, 40, 41, 42]);

        let _ = state.update(WallMsg::ToggleGrouping);
        assert_eq!(state.library.paths.len(), 6);
        assert!(state.library.grouping.is_none());
    }

    #[test]
    fn the_third_press_hashes_nothing() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 1, 2, 40, 41, 42]);
        let _ = state.update(WallMsg::ToggleGrouping);

        let _ = state.update(WallMsg::ToggleGrouping);
        // The fingerprints never left, so there is nothing to wait for.
        assert!(state.hashing.is_none());
        assert_eq!(state.library.paths.len(), 2);
    }

    #[test]
    fn the_dial_survives_a_trip_through_ungrouped() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 4, 8, 12, 16, 20]);
        let _ = state.update(WallMsg::Retune { looser: true });
        let _ = state.update(WallMsg::Retune { looser: true });
        assert_eq!(state.library.paths.len(), 1);

        let _ = state.update(WallMsg::ToggleGrouping);
        let _ = state.update(WallMsg::ToggleGrouping);
        // Coming back to a wall re-tightened to the default would undo work the
        // user did by eye.
        assert_eq!(state.library.paths.len(), 1);
    }

    #[test]
    fn a_photo_that_cannot_be_read_stacks_with_nothing() {
        let mut state = wall(&[200.0; 4], 1);
        let paths: Vec<PathBuf> = state.library.photos().cloned().collect();
        let _ = state.update(WallMsg::ToggleGrouping);
        for (i, path) in paths.into_iter().enumerate() {
            // The middle one is unreadable: nothing is known about it.
            let entry = (i != 1).then(|| Entry::for_test(print(0)));
            let _ = state.update(WallMsg::Fingerprinted { path, entry });
        }
        // 0 alone, 1 unknown, then 2 and 3 together.
        assert_eq!(state.library.paths.len(), 3);
    }

    #[test]
    fn the_pass_hands_out_only_so_much_at_a_time() {
        let mut state = wall(&[200.0; 40], 1);
        let _ = state.update(WallMsg::ToggleGrouping);

        let hashing = state.hashing.as_ref().expect("pass started");
        assert_eq!(hashing.total, 40);
        assert_eq!(hashing.in_flight, max_in_flight().min(40));
        assert_eq!(hashing.pending.len(), 40 - max_in_flight().min(40));
    }

    #[test]
    fn esc_stops_the_pass_and_leaves_the_wall_alone() {
        let mut state = wall(&[200.0; 40], 1);
        let paths: Vec<PathBuf> = state.library.photos().cloned().collect();
        let _ = state.update(WallMsg::ToggleGrouping);
        let _ = state.update(WallMsg::Fingerprinted {
            path: paths[0].clone(),
            entry: Some(Entry::for_test(print(0))),
        });

        let _ = state.update(WallMsg::Escape);
        // Photos still in flight have to land before it is over — including the
        // one claimed into the slot the first result freed.
        for path in paths.iter().take(max_in_flight().min(40) + 1).skip(1) {
            let _ = state.update(WallMsg::Fingerprinted {
                path: path.clone(),
                entry: Some(Entry::for_test(print(0))),
            });
        }

        assert!(state.hashing.is_none());
        // Half a folder's worth of hashes would stack the photos that happened
        // to finish and leave the rest looking unrelated.
        assert!(state.library.grouping.is_none());
        assert_eq!(state.library.paths.len(), 40);
    }

    #[test]
    fn g_pressed_again_mid_pass_stops_it() {
        let mut state = wall(&[200.0; 40], 1);
        let _ = state.update(WallMsg::ToggleGrouping);
        let _ = state.update(WallMsg::ToggleGrouping);
        // Cancelled, not started twice — the ones in flight are still landing.
        assert!(state.hashing.as_ref().is_some_and(|h| h.cancelled));
    }

    #[test]
    fn g_is_refused_while_painting() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        let _ = state.update(WallMsg::ToggleGrouping);
        assert!(state.hashing.is_none());
        assert!(state.library.grouping.is_none());
    }

    #[test]
    fn g_is_refused_while_a_batch_runs() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::SelectAll);
        rotate(&mut state);

        let _ = state.update(WallMsg::ToggleGrouping);
        // The wall must not be re-laid under files that are being written.
        assert!(state.hashing.is_none());
    }

    #[test]
    fn the_dial_regroups_without_hashing_again() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 4, 8, 12, 16, 20]);
        assert_eq!(state.library.paths.len(), 2);

        let _ = state.update(WallMsg::Retune { looser: true });
        let _ = state.update(WallMsg::Retune { looser: true });
        // Nothing was re-read to do that: the hashes were already in hand.
        assert!(state.hashing.is_none());
        assert_eq!(state.library.paths.len(), 1);

        let _ = state.update(WallMsg::Retune { looser: false });
        let _ = state.update(WallMsg::Retune { looser: false });
        assert_eq!(state.library.paths.len(), 2);
    }

    #[test]
    fn the_dial_does_nothing_before_g() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::Retune { looser: true });
        assert_eq!(state.library.paths.len(), 6);
        assert!(state.library.grouping.is_none());
    }

    #[test]
    fn a_selection_survives_grouping() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::SelectAll);
        group(&mut state, &[0, 1, 2, 40, 41, 42]);

        // Two tiles now, and both of them wholly selected: no photo has left
        // the wall, so none may leave the selection.
        assert_eq!(state.library.selection.len(), 6);
        assert_eq!(state.mode, WallMode::Select);
        assert_eq!(selected(&state), vec![0, 1]);
    }
}
