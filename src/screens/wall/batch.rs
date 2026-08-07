//! Driving a [`Batch`] from the wall: turning each release into a [`Task`],
//! and re-aiming the wall once the last file lands.
//!
//! Nothing here moves the view mid-batch. A file that lands is folded in and
//! the freed slot refilled, but `reveal` and `refocus` run once, at the end —
//! re-laying the masonry per file would shuffle the wall under whoever is
//! watching it.

use std::path::PathBuf;

use iced::Task;

use crate::Message;

use super::message::WallMsg;
use super::ops::run_one;
use super::queue::{Batch, BatchKind, FileDone};
use super::thumbs::max_in_flight;
use super::WallState;

impl WallState {
    /// Begin an operation over `paths`.
    ///
    /// Refused while a range is being painted or another batch is running, for
    /// the same reasons [`WallState::rotate`] is: an uncommitted range has no
    /// settled meaning, and two batches would fight over the same files.
    pub(super) fn start_batch(&mut self, kind: BatchKind, paths: Vec<PathBuf>) -> Task<Message> {
        if self.is_visual() || self.batch.is_some() || paths.is_empty() {
            return Task::none();
        }
        self.batch = Some(Batch::new(kind, paths));
        Task::batch([self.refill(), self.remeasure()])
    }

    /// Hand the runtime whatever the batch releases next, and no more.
    pub(super) fn refill(&mut self) -> Task<Message> {
        let Some(batch) = &mut self.batch else {
            return Task::none();
        };
        let kind = batch.kind().clone();
        let claimed = batch.claim(max_in_flight());

        let tasks: Vec<Task<Message>> = claimed
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
    pub(super) fn batch_progress(&mut self, path: PathBuf, result: Result<FileDone, String>) -> Task<Message> {
        self.rotating.remove(&path);
        let Some(batch) = &mut self.batch else {
            return Task::none();
        };
        let outcome = batch.record(&path, result);
        let finished = batch.is_finished();

        // Deliberately no `reveal` and no `refocus` here either: those move the
        // view. They run once, at the end.
        let invalidate = if outcome == FileDone::Reshaped {
            self.invalidate_rotated(path)
        } else {
            Task::none()
        };

        if !finished {
            return Task::batch([invalidate, self.refill(), self.schedule()]);
        }

        let gone = self.batch.take().expect("checked just above").finish();
        // Every tile the batch touched changed shape or left, so a sticky
        // centre from before it means nothing now.
        self.desired_y = None;
        self.refocus();
        // The files that left go back through `App`, which is the only place
        // that can fall back to the empty screen if none are left.
        let removed = if gone.is_empty() {
            Task::none()
        } else {
            Task::done(Message::Removed {
                gone,
                failed: Vec::new(),
            })
        };
        Task::batch([invalidate, removed, self.remeasure(), self.schedule()])
    }

    /// Stop dispatching new work. Files already handed to the runtime finish —
    /// abandoning a write half-done is how a photo gets corrupted.
    pub(super) fn cancel_batch(&mut self) -> Task<Message> {
        let Some(batch) = &mut self.batch else {
            return Task::none();
        };
        batch.cancel();
        if batch.is_finished() {
            self.batch = None;
            return self.remeasure();
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::wall::fixture::*;
    use crate::screens::wall::select::WallMode;
    use crate::library::RangeOp;
    use crate::transfer::{Collision, TransferKind};
    use std::path::PathBuf;
    use crate::screens::wall::message::WallMsg;

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
        assert_eq!(batch.total(), 40);
        // Bounded like the decode scheduler: the rest wait their turn rather
        // than all being handed to the runtime at once.
        assert_eq!(batch.in_flight(), max_in_flight().min(40));
        assert_eq!(batch.pending_len(), 40 - max_in_flight().min(40));
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
        assert_eq!(batch.done(), 1);
        // The freed slot was handed the next file, not left idle.
        assert_eq!(batch.pending_len(), before - 1);
        assert_eq!(batch.in_flight(), max_in_flight().min(40));
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
        assert_eq!(batch.failed_len(), 1);
        assert_eq!(batch.done(), 1);

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
        assert!(batch.remaining().is_empty());
        assert!(batch.is_cancelled());
        // ...but abandoning a write half-done is how a photo gets corrupted.
        assert!(batch.in_flight() > 0);

        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        for path in paths.iter().take(batch.in_flight()) {
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
        let done_before = state.batch.as_ref().unwrap().pending_len();

        rotate(&mut state);
        // Two batches over the same files would race each other's writes.
        assert_eq!(state.batch.as_ref().unwrap().pending_len(), done_before);
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

    #[test]
    fn a_transfer_runs_over_the_paths_it_was_handed() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        let _ = state.update(WallMsg::SelectAll);

        // The question the user answered named a count, so that count is what
        // runs — not whatever the selection happens to hold by now.
        start(&mut state, move_kind(), paths[..2].to_vec());
        assert_eq!(state.batch.as_ref().unwrap().total(), 2);
    }

    #[test]
    fn a_transfer_is_bounded_like_every_other_batch() {
        let mut state = wall(&[200.0; 40], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        start(&mut state, move_kind(), paths);

        let batch = state.batch.as_ref().expect("batch started");
        assert_eq!(batch.in_flight(), max_in_flight().min(40));
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
        assert_eq!(state.batch.as_ref().unwrap().gone(), [paths[0].clone()]);

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
            state.batch.as_ref().unwrap().kind(),
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
        assert!(batch.remaining().is_empty());
        // Files already handed to the runtime still land: half a move is a lost
        // photo.
        let in_flight = batch.in_flight();
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
}
