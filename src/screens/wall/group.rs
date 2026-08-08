//! `g`: the fingerprint pass, the stacks it turns into, and going inside one.
//!
//! Hashing a folder is the only expensive part of grouping, so it happens once,
//! here, and everything after it is a pass over numbers already in hand — see
//! `GROUP_MODE_PLAN.md`. The pass is shaped like the decode scheduler and the
//! batch queue before it: bounded work in flight, refilled as each result
//! lands, with how far along it is in the mode bar and Esc to stop it.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use iced::Task;

use crate::core::fingerprint::Fingerprint;
use crate::core::fingerprint_cache::{self, Cache, Entry};
use crate::core::grouping::Grouping;
use crate::core::library::Library;
use crate::core::pointed_list::PointedList;
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
        // Inside a stack there is nothing to group — the wall underneath is the
        // one with stacks on it. So `g` here explodes: back out, and ungroup,
        // which leaves those photographs spread across the wall in place.
        //
        // Exploding *only* this stack was the other reading. It would have to
        // survive a re-chain, and a re-chain happens after every trash, filter
        // and turn of the dial; keeping a manual split alive across those is
        // the machinery `GROUP_MODE_PLAN.md` set out not to build.
        if self.parent.is_some() {
            let popped = self.pop();
            return Task::batch([popped, self.toggle_grouping()]);
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

    /// Whether the tile at `index` stands for more than one photograph — which
    /// is what makes Enter, and a click, go into it rather than open it.
    pub(crate) fn stack_at(&self, index: usize) -> bool {
        self.library
            .paths
            .iter()
            .nth(index)
            .is_some_and(|path| self.library.stack_size(path) > 1)
    }

    /// Open a wall over the photographs the tile at `index` stands for, with
    /// this one hung underneath it.
    ///
    /// A second [`WallState`] over a narrowed [`Library`], which is what makes
    /// every wall command work inside a stack without a line of new code:
    /// navigation, selection, ranges, rotate, favourite, trash and move all
    /// see an ordinary folder that happens to hold four photographs.
    pub(super) fn descend(&mut self, index: usize) -> Task<Message> {
        if self.is_visual() || self.batch.is_some() || self.hashing.is_some() {
            return Task::none();
        }
        let Some(path) = self.library.paths.iter().nth(index).cloned() else {
            return Task::none();
        };
        let members = self.library.members(&path);
        if members.len() < 2 {
            return Task::none();
        }
        let Some(paths) = PointedList::new(members.clone()) else {
            return Task::none();
        };

        let inside = Library {
            // Its own, starting empty and discarded on the way out. The stack
            // is one thing to the wall underneath, so a selection in here has
            // nothing to say out there — and starting clean means Esc leaves in
            // one press rather than clearing something first.
            selection: HashSet::new(),
            all: members.clone(),
            paths,
            image_dir: self.library.image_dir.clone(),
            tags: self.library.tags.clone(),
            // A stack is already a handful of near-identical photographs.
            // Narrowing it further, or stacking it again, says nothing.
            filter: None,
            grouping: None,
        };

        let mut inside = WallState::new(inside);
        // The window has not changed size, so the wall inside can be laid out
        // on its first frame rather than after a round trip through `measure`.
        inside.viewport = self.viewport;
        let outside = std::mem::replace(self, inside);
        self.parent = Some(Box::new(outside));
        self.enter()
    }

    /// `p`: the photographs a stack would lose to keep the one under the
    /// cursor.
    ///
    /// `None` anywhere but inside a stack. Out on the folder there is nothing
    /// to pick between — a plain thumbnail has no rest to trash, and on a stack
    /// the user cannot see which member they would be keeping.
    pub(crate) fn rest_of_the_stack(&self) -> Option<Vec<PathBuf>> {
        if self.parent.is_none() || self.is_visual() || self.batch.is_some() {
            return None;
        }
        let keep = self.library.current();
        let rest: Vec<PathBuf> = self
            .library
            .photos()
            .filter(|path| *path != keep)
            .cloned()
            .collect();
        (!rest.is_empty()).then_some(rest)
    }

    /// Back out to the wall underneath, or do nothing if this is the folder.
    ///
    /// Its scroll position, cursor and selection were never dismantled, so they
    /// need no restoring. What does need carrying back is everything the wall
    /// inside changed about the folder they share.
    pub(super) fn pop(&mut self) -> Task<Message> {
        let Some(outside) = self.parent.take() else {
            return Task::none();
        };
        let inside = std::mem::replace(self, *outside);

        // The tags were edited against the same folder and already written to
        // disk, so the copy taken on the way in is the stale one — and the next
        // tag change out here would write it back over them. The selection is
        // the opposite case: it is this wall's own scratch, and the one out
        // here was never dismantled.
        self.library.tags = inside.library.tags;
        // Photographs may have been rotated in there, which makes the decodes
        // from inside the fresh ones.
        self.thumbs.extend(inside.thumbs);
        self.ratios.extend(inside.ratios);
        // Decodes dispatched before descending landed inside, if they landed at
        // all. Forget they were ever out, so `schedule` can ask again.
        self.in_flight.clear();

        // Tags may have changed under the filter, and photographs may have gone.
        self.library.refilter();
        Task::batch([self.settle(), self.resettled()])
    }

    /// Drop `gone` from a wall that is not on screen, and from every wall under
    /// it. `false` if nothing is left of it.
    ///
    /// Produces no tasks, deliberately. Nothing here is being drawn, and a task
    /// from a hidden wall would be answered by the live one — scrolling it, or
    /// re-laying it around a folder it is not showing.
    pub(crate) fn pruned(&mut self, gone: &HashSet<PathBuf>) -> bool {
        for path in gone {
            self.thumbs.remove(path);
            self.ratios.remove(path);
        }
        let orphaned = self
            .parent
            .as_mut()
            .is_some_and(|parent| !parent.pruned(gone));
        if orphaned {
            self.parent = None;
        }
        self.library.remove(gone)
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
    use crate::core::library::{RangeOp, Selected};
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

    // --- Inside a stack ---

    /// A wall of six, stacked three and three, with the first stack open.
    fn inside_a_stack() -> (WallState, Vec<PathBuf>) {
        let mut state = wall(&[200.0; 6], 1);
        let photos: Vec<PathBuf> = state.library.photos().cloned().collect();
        group(&mut state, &[0, 1, 2, 40, 41, 42]);
        descend(&mut state, 0);
        (state, photos)
    }

    #[test]
    fn enter_on_a_stack_opens_a_wall_of_its_photos() {
        let (state, photos) = inside_a_stack();
        assert_eq!(state.library.all, photos[..3].to_vec());
        assert!(state.parent.is_some());
        // An ordinary wall over a narrower folder, which is what makes every
        // command work in here without a line of new code.
        assert!(state.library.grouping.is_none());
        assert_eq!(state.library.filter, None);
    }

    #[test]
    fn enter_on_a_photograph_inside_a_stack_does_not_go_deeper() {
        let (mut state, _) = inside_a_stack();
        // There are no stacks in here to open, so nothing to descend into.
        descend(&mut state, 1);
        assert_eq!(state.library.all.len(), 3);
    }

    #[test]
    fn esc_backs_out_of_a_stack() {
        let (mut state, photos) = inside_a_stack();
        let _ = state.update(WallMsg::Escape);

        assert!(state.parent.is_none());
        assert_eq!(state.library.all, photos);
        assert_eq!(state.library.paths.len(), 2);
    }

    #[test]
    fn esc_clears_a_selection_before_it_leaves() {
        let (mut state, _) = inside_a_stack();
        let _ = state.update(WallMsg::SelectAll);

        let _ = state.update(WallMsg::Escape);
        // One keypress must not both discard a selection and leave the wall it
        // was made on.
        assert!(state.parent.is_some());
        assert!(state.library.selection.is_empty());

        let _ = state.update(WallMsg::Escape);
        assert!(state.parent.is_none());
    }

    #[test]
    fn a_stack_opens_with_nothing_selected() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 1, 2, 40, 41, 42]);
        let _ = state.update(WallMsg::ToggleCursor);
        descend(&mut state, 0);

        // The selection in here is this wall's own. Inheriting one would mean
        // Esc had to clear it before it could leave, so going in and straight
        // back out would take two presses and appear to have thrown something
        // away.
        assert!(state.library.selection.is_empty());
        assert_eq!(state.mode, WallMode::Normal);
    }

    #[test]
    fn a_selection_made_inside_stays_inside() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 1, 2, 40, 41, 42]);
        let tile = state.library.current().clone();
        let _ = state.update(WallMsg::ToggleCursor);
        descend(&mut state, 0);

        state.library.goto(1);
        let _ = state.update(WallMsg::ToggleCursor);
        let _ = state.update(WallMsg::Escape); // clears it
        let _ = state.update(WallMsg::Escape); // leaves

        // The wall underneath was never dismantled, so what was selected on it
        // is still selected on it.
        assert!(state.parent.is_none());
        assert_eq!(state.library.selected(&tile), Selected::All);
        assert_eq!(state.mode, WallMode::Select);
    }

    #[test]
    fn a_favourite_made_inside_is_still_there_outside() {
        let (mut state, _) = inside_a_stack();
        fav(&mut state);
        let _ = state.update(WallMsg::Escape);

        // The tags were written against the shared folder, so the copy the
        // outer wall carried in is the stale one — and the next tag change out
        // here would write it back over them.
        assert_eq!(starred(&state), vec![0]);
    }

    #[test]
    fn trashing_inside_a_stack_prunes_the_wall_underneath() {
        let (mut state, photos) = inside_a_stack();
        assert!(remove(&mut state, &photos[1..2]));
        assert_eq!(state.library.all.len(), 2);

        let _ = state.update(WallMsg::Escape);
        // Coming back out to a wall still listing it would show a photograph
        // that is not there.
        assert!(!state.library.all.contains(&photos[1]));
        assert_eq!(state.library.all.len(), 5);
    }

    #[test]
    fn trashing_the_last_photo_of_a_stack_backs_out_of_it() {
        let (mut state, photos) = inside_a_stack();
        // The stack is gone, but the folder underneath it is not: this is the
        // wall dying, not the app running out of photographs.
        assert!(remove(&mut state, &photos[..3]));
        assert!(state.parent.is_none());
        assert_eq!(state.library.all, photos[3..].to_vec());
        assert_eq!(state.library.paths.len(), 1);
    }

    #[test]
    fn trashing_the_whole_folder_from_inside_a_stack_reports_an_empty_wall() {
        let mut state = wall(&[200.0; 3], 1);
        let photos: Vec<PathBuf> = state.library.photos().cloned().collect();
        group(&mut state, &[0, 1, 2]);
        descend(&mut state, 0);
        // Nothing anywhere in the chain: the caller has to fall back to the
        // empty screen.
        assert!(!remove(&mut state, &photos));
    }

    #[test]
    fn p_names_every_photo_of_the_stack_but_the_one_under_the_cursor() {
        let (mut state, photos) = inside_a_stack();
        state.library.goto(1);
        assert_eq!(
            state.rest_of_the_stack(),
            Some(vec![photos[0].clone(), photos[2].clone()])
        );
    }

    #[test]
    fn p_means_nothing_out_on_the_folder() {
        let mut state = wall(&[200.0; 6], 1);
        // Not on a plain thumbnail, which has no rest to trash.
        assert_eq!(state.rest_of_the_stack(), None);

        group(&mut state, &[0, 1, 2, 40, 41, 42]);
        // And not on a stack from outside it either: the user cannot see which
        // of its photographs they would be keeping.
        assert_eq!(state.rest_of_the_stack(), None);
    }

    #[test]
    fn p_means_nothing_while_painting() {
        let (mut state, _) = inside_a_stack();
        enter_visual(&mut state, RangeOp::Add);
        // An uncommitted range has no settled meaning, and this one would trash
        // files either way.
        assert_eq!(state.rest_of_the_stack(), None);
    }

    #[test]
    fn keeping_one_photo_backs_out_of_the_stack() {
        let (mut state, photos) = inside_a_stack();
        let rest = state.rest_of_the_stack().unwrap();
        assert!(remove(&mut state, &rest));

        // What is left is one photograph, so there is no stack to be inside of.
        assert!(state.parent.is_none());
        assert_eq!(state.library.all.len(), 4);
        assert_eq!(state.library.stack_size(&photos[0]), 1);
        assert_eq!(state.library.paths.len(), 2);
    }

    #[test]
    fn a_stack_is_only_spent_once_it_is_down_to_one() {
        let (mut state, photos) = inside_a_stack();
        assert!(remove(&mut state, &photos[..1]));
        // Two photographs are still a pile worth being inside.
        assert!(state.parent.is_some());

        assert!(remove(&mut state, &photos[1..2]));
        assert!(state.parent.is_none());
    }

    #[test]
    fn g_inside_a_stack_backs_out_and_ungroups() {
        let (mut state, photos) = inside_a_stack();
        let _ = state.update(WallMsg::ToggleGrouping);

        assert!(state.parent.is_none());
        assert!(state.library.grouping.is_none());
        // Which leaves the photographs that were in the stack spread across the
        // wall where they belong.
        assert_eq!(shown_paths(&state), photos);
    }

    #[test]
    fn the_dial_does_nothing_inside_a_stack() {
        let (mut state, _) = inside_a_stack();
        let _ = state.update(WallMsg::Retune { looser: true });
        assert_eq!(state.library.paths.len(), 3);
        assert!(state.parent.is_some());
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
