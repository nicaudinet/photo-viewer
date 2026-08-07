//! Clicks on a thumbnail.
//!
//! The mouse is modal in the same way the keyboard is: in `Normal` a plain
//! click opens the image, exactly as it did before selection existed. Once a
//! selection is live, plain clicks select instead and opening moves to a
//! double click.

use std::time::{Duration, Instant};

use iced::keyboard::Modifiers;
use iced::Task;

use crate::core::library::RangeOp;
use crate::Message;

use super::select::WallMode;
use super::WallState;

/// How close together two clicks on the same thumbnail count as a double
/// click. macOS's own default threshold is 500ms; this is a little tighter so
/// two deliberate toggles of the same tile aren't read as one.
pub(super) const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// What a click on a thumbnail asks for. `Open` leaves the wall, so any task
/// the wall would have run is moot — hence no payload.
pub(crate) enum Click {
    Open,
    Handled(Task<Message>),
}

impl WallState {
    /// Handle a click on thumbnail `index`, carrying the modifiers live at the
    /// time (a `button` reports none of its own — see `App::modifiers`).
    ///
    /// The mouse is modal in the same way the keyboard is. In `Normal` a plain
    /// click still opens the image, exactly as it did before selection existed;
    /// `Cmd` is the way in without touching the keyboard. Once a selection is
    /// live, plain clicks select instead, and opening moves to a double click.
    ///
    /// Every click also moves the cursor to what it hit, so a `v` pressed after
    /// a click anchors where the user is actually looking.
    pub(crate) fn click(&mut self, index: usize, modifiers: Modifiers) -> Click {
        if index >= self.library.paths.len() {
            return Click::Handled(Task::none());
        }
        let cursor = self.library.paths.index();
        let double = self.register_click(index);

        // While painting, a click is a motion: the cursor is the moving end of
        // the range, so clicking a thumbnail extends the range to it, exactly
        // as `j` would. Editing the committed set here would settle the mode
        // and end the range behind the user's back, as it does for `Space`.
        if self.is_visual() {
            return Click::Handled(self.move_cursor_to(index));
        }

        // In `Normal` a bare click keeps its old meaning; a modifier is the
        // request to select instead.
        let selecting = self.mode == WallMode::Select || modifiers.command() || modifiers.shift();
        if !selecting {
            return Click::Open;
        }

        if modifiers.shift() {
            // Extend from wherever the cursor is — which, after any earlier
            // click, is the last thumbnail clicked.
            self.library.apply_range(cursor, index, RangeOp::Add);
        } else {
            self.library.toggle_selected(index);
        }

        let task = Task::batch([self.move_cursor_to(index), self.settle()]);
        if double {
            // The first click of the pair toggled this tile and the second
            // toggled it back, so opening leaves the selection as it was.
            Click::Open
        } else {
            Click::Handled(task)
        }
    }

    /// Whether this click completes a double click, and record it either way.
    /// A completed pair clears the record, so a third click starts afresh
    /// rather than reading as another double.
    pub(super) fn register_click(&mut self, index: usize) -> bool {
        let now = Instant::now();
        let double = matches!(
            self.last_click,
            Some((last, at)) if last == index && now.duration_since(at) < DOUBLE_CLICK
        );
        self.last_click = (!double).then_some((index, now));
        double
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::library::RangeOp;
    use crate::screens::wall::fixture::*;
    use crate::screens::wall::message::WallMsg;

    #[test]
    fn a_plain_click_still_opens_in_normal() {
        let mut state = wall(&[200.0; 6], 1);
        assert!(opens(click(&mut state, 3, PLAIN)));
        // Nothing selected, and no mode change: the pre-selection behaviour is
        // untouched for anyone who never presses `v`.
        assert!(state.library.selection.is_empty());
        assert_eq!(state.mode, WallMode::Normal);
    }

    #[test]
    fn cmd_click_selects_without_the_keyboard() {
        let mut state = wall(&[200.0; 6], 1);
        assert!(!opens(click(&mut state, 3, Modifiers::COMMAND)));
        assert_eq!(selected(&state), vec![3]);
        assert_eq!(state.mode, WallMode::Select);
        assert_eq!(state.library.paths.index(), 3);
    }

    #[test]
    fn a_plain_click_selects_once_a_selection_is_live() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 1, Modifiers::COMMAND);
        // Now in `Select`, so the mouse is modal too: no modifier needed.
        assert!(!opens(click(&mut state, 4, PLAIN)));
        assert_eq!(selected(&state), vec![1, 4]);
    }

    #[test]
    fn a_plain_click_deselects_a_selected_tile() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 1, Modifiers::COMMAND);
        let _ = click(&mut state, 4, PLAIN);
        let _ = click(&mut state, 1, PLAIN);
        assert_eq!(selected(&state), vec![4]);
    }

    #[test]
    fn deselecting_the_last_tile_returns_to_normal() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 2, Modifiers::COMMAND);
        let _ = click(&mut state, 2, PLAIN);
        assert_eq!(state.mode, WallMode::Normal);
        // And clicks open again, as they did before.
        assert!(opens(click(&mut state, 2, PLAIN)));
    }

    #[test]
    fn shift_click_extends_from_the_cursor() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(1);
        assert!(!opens(click(&mut state, 4, Modifiers::SHIFT)));
        assert_eq!(selected(&state), vec![1, 2, 3, 4]);
    }

    #[test]
    fn shift_click_extends_from_the_last_click() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 1, Modifiers::COMMAND);
        // Every click moves the cursor, so the run starts where the last click
        // landed rather than wherever the keyboard was left.
        let _ = click(&mut state, 3, Modifiers::SHIFT);
        assert_eq!(selected(&state), vec![1, 2, 3]);
    }

    #[test]
    fn a_click_moves_the_cursor_so_v_anchors_there() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 4, Modifiers::COMMAND);
        enter_visual(&mut state, RangeOp::Add);
        assert_eq!(
            state.mode,
            WallMode::Visual {
                anchor: 4,
                op: RangeOp::Add
            }
        );
    }

    #[test]
    fn a_click_while_painting_extends_the_range() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        assert!(!opens(click(&mut state, 3, PLAIN)));

        // A motion, not a set edit: the range is still being painted.
        assert_eq!(
            state.mode,
            WallMode::Visual {
                anchor: 0,
                op: RangeOp::Add
            }
        );
        assert!(state.library.selection.is_empty());
        assert_eq!(state.library.paths.index(), 3);

        let _ = state.update(WallMsg::CommitVisual);
        assert_eq!(selected(&state), vec![0, 1, 2, 3]);
    }

    #[test]
    fn a_double_click_opens_and_leaves_the_selection_alone() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 0, Modifiers::COMMAND);
        // First of the pair selects 3...
        assert!(!opens(click(&mut state, 3, PLAIN)));
        assert_eq!(selected(&state), vec![0, 3]);
        // ...and the second toggles it back off, then opens it.
        assert!(opens(click(&mut state, 3, PLAIN)));
        assert_eq!(selected(&state), vec![0]);
    }

    #[test]
    fn two_clicks_on_different_tiles_are_not_a_double_click() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 0, Modifiers::COMMAND);
        assert!(!opens(click(&mut state, 3, PLAIN)));
        assert!(!opens(click(&mut state, 4, PLAIN)));
        assert_eq!(selected(&state), vec![0, 3, 4]);
    }

    #[test]
    fn a_slow_second_click_is_not_a_double_click() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 0, Modifiers::COMMAND);
        let _ = click(&mut state, 3, PLAIN);
        // Age the first click past the threshold.
        state.last_click = Some((3, Instant::now() - DOUBLE_CLICK * 2));
        assert!(!opens(click(&mut state, 3, PLAIN)));
        assert_eq!(selected(&state), vec![0]);
    }

    #[test]
    fn a_third_click_starts_a_fresh_pair() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = click(&mut state, 0, Modifiers::COMMAND);
        let _ = click(&mut state, 3, PLAIN); // select 3
        let _ = click(&mut state, 3, PLAIN); // double: deselect, opens
                                             // Without clearing the record, this would read as another double.
        assert!(!opens(click(&mut state, 3, PLAIN)));
        assert_eq!(selected(&state), vec![0, 3]);
    }

    #[test]
    fn a_click_past_the_end_of_the_library_is_ignored() {
        let mut state = wall(&[200.0; 6], 1);
        assert!(!opens(click(&mut state, 99, Modifiers::COMMAND)));
        assert!(state.library.selection.is_empty());
        assert_eq!(state.library.paths.index(), 0);
    }
}
