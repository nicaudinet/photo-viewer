//! The selection modes and the transitions between them.
//!
//! There is exactly one cursor — `library.paths.index()` — and it moves in
//! every mode. `Visual` adds only an anchor; the moving end of the painted
//! range *is* the cursor, which is why navigation needs no special case for it.

use iced::Task;

use crate::core::library::RangeOp;
use crate::Message;

use super::WallState;

/// Which selection mode the wall is in.
///
/// There is exactly one cursor — `library.paths.index()` — and it moves in
/// every mode. `Visual` adds only an anchor; the moving end of the painted
/// range *is* the cursor, which is why navigation needs no special case for it.
///
/// Invariant, restored by [`WallState::settle`] after every selection change:
/// outside `Visual`, the mode is `Select` exactly when the selection is
/// non-empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WallMode {
    Normal,
    Visual { anchor: usize, op: RangeOp },
    Select,
}

impl WallState {
    /// Start painting a range from the cursor. Pressing the same key again
    /// while painting cancels, as `v` does in vim.
    pub(super) fn enter_visual(&mut self, op: RangeOp) -> Task<Message> {
        match self.mode {
            WallMode::Visual { .. } => self.escape(),
            // `x` means "remove a run", which is meaningless with nothing
            // selected — and entering a mode that can only be a no-op would
            // just strand the user in it.
            WallMode::Normal if op == RangeOp::Remove => Task::none(),
            WallMode::Normal | WallMode::Select => {
                self.mode = WallMode::Visual {
                    anchor: self.library.paths.index(),
                    op,
                };
                self.remeasure()
            }
        }
    }

    /// Fold the painted range into the selection and leave `Visual`. The cursor
    /// stays where it is: the range's far end is where the user is looking.
    pub(super) fn commit_visual(&mut self) -> Task<Message> {
        let WallMode::Visual { anchor, op } = self.mode else {
            return Task::none();
        };
        self.library
            .apply_range(anchor, self.library.paths.index(), op);
        self.settle()
    }

    /// One rung down the Esc ladder. A running batch is the top rung — the
    /// user is most likely reaching for Esc to stop it. From `Visual` it drops
    /// the painted range
    /// and returns the cursor to the anchor — the trip was cancelled, so it
    /// ends where it began. From `Select` it clears the set and leaves the
    /// cursor alone, because there the user moved it deliberately.
    ///
    /// `Visual` entered from `Select` therefore takes two presses to leave
    /// entirely: one keypress must not discard a large selection.
    pub(super) fn escape(&mut self) -> Task<Message> {
        if self.batch.is_some() {
            return self.cancel_batch();
        }
        match self.mode {
            WallMode::Visual { anchor, .. } => {
                self.library.goto(anchor);
                self.desired_y = None;
                let settle = self.settle();
                let reveal = match self.viewport {
                    Some(viewport) => self.reveal(&self.layout(viewport.width)),
                    None => Task::none(),
                };
                Task::batch([settle, reveal, self.schedule()])
            }
            WallMode::Select => {
                self.library.clear_selection();
                self.settle()
            }
            WallMode::Normal => Task::none(),
        }
    }

    /// Restore the mode invariant after a change to the selection, and
    /// re-measure in case the mode bar appeared or disappeared.
    pub(super) fn settle(&mut self) -> Task<Message> {
        self.mode = if self.library.selection.is_empty() {
            WallMode::Normal
        } else {
            WallMode::Select
        };
        self.remeasure()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::library::RangeOp;
    use crate::screens::wall::fixture::*;
    use crate::screens::wall::message::Dir;
    use crate::screens::wall::message::WallMsg;
    use crate::screens::wall::select::WallMode;

    #[test]
    fn a_committed_range_selects_the_whole_run() {
        // One column, so `j` moves one library index at a time.
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);
        nav(&mut state, Dir::Down);
        let _ = state.update(WallMsg::CommitVisual);

        assert_eq!(selected(&state), vec![0, 1, 2]);
        assert_eq!(state.mode, WallMode::Select);
        // The cursor stays at the far end of the range, where the user is
        // looking — it does not snap back to the anchor.
        assert_eq!(state.library.paths.index(), 2);
    }

    #[test]
    fn a_range_painted_upwards_covers_the_same_run() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(4);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Up);
        nav(&mut state, Dir::Up);
        let _ = state.update(WallMsg::CommitVisual);
        assert_eq!(selected(&state), vec![2, 3, 4]);
    }

    #[test]
    fn escape_from_visual_cancels_and_returns_the_cursor() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);
        nav(&mut state, Dir::Down);
        let _ = state.update(WallMsg::Escape);

        assert!(state.library.selection.is_empty());
        assert_eq!(state.mode, WallMode::Normal);
        // The trip was cancelled, so it ends where it began.
        assert_eq!(state.library.paths.index(), 1);
    }

    #[test]
    fn v_pressed_twice_cancels_like_vim() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        enter_visual(&mut state, RangeOp::Add);
        assert_eq!(state.mode, WallMode::Normal);
        assert!(state.library.selection.is_empty());
    }

    #[test]
    fn a_second_range_unions_into_the_selection() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);
        let _ = state.update(WallMsg::CommitVisual); // 0..=1

        state.library.goto(4);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);
        let _ = state.update(WallMsg::CommitVisual); // 4..=5

        // Editing a selection is re-entering visual: runs accumulate.
        assert_eq!(selected(&state), vec![0, 1, 4, 5]);
    }

    #[test]
    fn x_paints_a_range_that_subtracts() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::SelectAll);

        state.library.goto(1);
        enter_visual(&mut state, RangeOp::Remove);
        nav(&mut state, Dir::Down);
        let _ = state.update(WallMsg::CommitVisual);

        assert_eq!(selected(&state), vec![0, 3, 4, 5]);
        assert_eq!(state.mode, WallMode::Select);
    }

    #[test]
    fn x_does_nothing_with_an_empty_selection() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Remove);
        // Entering a mode whose only possible outcome is a no-op would just
        // strand the user in it.
        assert_eq!(state.mode, WallMode::Normal);
    }

    #[test]
    fn set_edits_are_ignored_mid_paint() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);

        // Each of these edits the committed set, so honouring it would settle
        // the mode and end the range without the user saying so.
        for msg in [
            WallMsg::ToggleCursor,
            WallMsg::SelectAll,
            WallMsg::InvertSelection,
        ] {
            let _ = state.update(msg);
            assert_eq!(
                state.mode,
                WallMode::Visual {
                    anchor: 0,
                    op: RangeOp::Add
                }
            );
            assert!(state.library.selection.is_empty());
        }
    }

    #[test]
    fn escape_from_select_clears_but_leaves_the_cursor() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(3);
        let _ = state.update(WallMsg::ToggleCursor);
        assert_eq!(state.mode, WallMode::Select);

        let _ = state.update(WallMsg::Escape);
        assert!(state.library.selection.is_empty());
        assert_eq!(state.mode, WallMode::Normal);
        // In `Select` the user moves the cursor deliberately, so it stays put.
        assert_eq!(state.library.paths.index(), 3);
    }

    #[test]
    fn visual_over_select_takes_two_escapes_to_leave() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::ToggleCursor); // select 0
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);

        // One press must not discard a selection built up over many moves.
        let _ = state.update(WallMsg::Escape);
        assert_eq!(state.mode, WallMode::Select);
        assert_eq!(selected(&state), vec![0]);

        let _ = state.update(WallMsg::Escape);
        assert_eq!(state.mode, WallMode::Normal);
        assert!(state.library.selection.is_empty());
    }

    #[test]
    fn toggling_the_last_selected_image_returns_to_normal() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::ToggleCursor);
        assert_eq!(state.mode, WallMode::Select);
        let _ = state.update(WallMsg::ToggleCursor);
        // The invariant: no mode bar and no `Select` with nothing selected.
        assert_eq!(state.mode, WallMode::Normal);
    }

    #[test]
    fn select_all_and_invert_move_into_and_out_of_select() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::SelectAll);
        assert_eq!(selected(&state), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(state.mode, WallMode::Select);

        let _ = state.update(WallMsg::InvertSelection);
        assert!(state.library.selection.is_empty());
        assert_eq!(state.mode, WallMode::Normal);
    }

    #[test]
    fn a_selection_survives_a_trip_through_the_single_view() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::ToggleCursor);

        // `w` moves the library across and rebuilds the wall from it; the mode
        // is derived from the selection rather than carried.
        let rebuilt = WallState::new(state.library);
        assert_eq!(rebuilt.mode, WallMode::Select);
        assert_eq!(selected(&rebuilt), vec![0]);
    }

    #[test]
    fn a_painted_range_does_not_survive_that_trip() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);

        let rebuilt = WallState::new(state.library);
        assert_eq!(rebuilt.mode, WallMode::Normal);
        assert!(rebuilt.library.selection.is_empty());
    }
}
