//! Moving the cursor, and keeping it on screen.
//!
//! The viewport size the masonry needs only exists once iced has laid the tree
//! out, so it is read back off the finished widget tree with an [`Operation`]
//! (see [`measure`]) rather than computed. The view never writes state.

use iced::advanced::widget::operation::scrollable::Scrollable;
use iced::advanced::widget::operation::Outcome;
use iced::advanced::widget::{operate, Id, Operation};
use iced::widget::operation::{scroll_to, AbsoluteOffset};
use iced::{Rectangle, Size, Task, Vector};

use crate::Message;

use super::layout::{centre_of, nearest_position, neighbour, WallLayout, SEL_BORDER, WALL_SPACING};
use super::message::{Dir, WallMsg};
use super::WallState;

/// Identifies the wall's scrollable so [`measure`] and [`WallState::reveal`]
/// can target it.
pub(super) const WALL_ID: Id = Id::new("wall");

impl WallState {
    /// Put the cursor on `index` and bring it into view.
    pub(super) fn move_cursor_to(&mut self, index: usize) -> Task<Message> {
        self.library.goto(index);
        // The move was to an arbitrary point, so any sticky centre a run of
        // `h`/`l` had built up no longer describes where the cursor sits.
        self.desired_y = None;
        let reveal = match self.viewport {
            Some(viewport) => self.reveal(&self.layout(viewport.width)),
            None => Task::none(),
        };
        self.refocus();
        Task::batch([reveal, self.schedule()])
    }

    /// The mode bar takes height from the scroll viewport, and `reveal` scrolls
    /// against that height — so showing or hiding it has to be measured again.
    pub(super) fn remeasure(&self) -> Task<Message> {
        measure()
    }

    /// Move the selection one thumbnail in `dir`, scroll it into view, and
    /// re-aim the decode scheduler at it. A no-op at the edges of the wall.
    pub(super) fn navigate(&mut self, dir: Dir) -> Task<Message> {
        let Some(viewport) = self.viewport else {
            return Task::none();
        };
        let layout = self.layout(viewport.width);
        let current = self.library.paths.index();
        let Some(next) = neighbour(&layout, current, dir, self.desired_y) else {
            return Task::none();
        };
        self.library.goto(next);
        // Left/right keeps the vertical centre it started from; up/down resets
        // it, since those moves redefine where "beside" is.
        self.desired_y = match dir {
            Dir::Left | Dir::Right => Some(
                self.desired_y
                    .unwrap_or_else(|| centre_of(&layout, current).unwrap_or(0.0)),
            ),
            Dir::Up | Dir::Down => None,
        };
        let reveal = self.reveal(&layout);
        self.refocus();
        Task::batch([reveal, self.schedule()])
    }

    /// Scroll the selected thumbnail into view, but only if it isn't already —
    /// and only far enough to reach the nearer edge.
    ///
    /// Takes `&mut self` because a programmatic scroll doesn't fire `on_scroll`,
    /// so the new offset has to be recorded here or the next call would decide
    /// from a stale one.
    pub(super) fn reveal(&mut self, layout: &WallLayout) -> Task<Message> {
        let Some(viewport) = self.viewport else {
            return Task::none();
        };
        let Some(slot) = layout.slots.get(&self.library.paths.index()) else {
            return Task::none();
        };
        // Include the selection ring, so it never sits half off-screen.
        let top = slot.y - SEL_BORDER;
        let bottom = slot.y + slot.height + SEL_BORDER;

        let target = if top < self.scroll_y {
            top - WALL_SPACING
        } else if bottom > self.scroll_y + viewport.height {
            bottom + WALL_SPACING - viewport.height
        } else {
            return Task::none();
        };

        let max = (layout.content_height - viewport.height).max(0.0);
        let y = target.clamp(0.0, max);
        self.scroll_y = y;
        scroll_to(WALL_ID, AbsoluteOffset { x: 0.0, y })
    }

    /// Re-aim decode priority at whatever is now in the middle of the viewport.
    pub(super) fn refocus(&mut self) {
        let Some(viewport) = self.viewport else {
            self.focus = 0.0;
            return;
        };
        let layout = self.layout(viewport.width);
        self.focus = nearest_position(&layout, self.scroll_y + viewport.height / 2.0);
    }
}

/// Read the wall scrollable's viewport size back out of the laid-out widget
/// tree, as a message for `update` to store.
///
/// Yields nothing if the wall isn't on screen — the operation simply finds no
/// scrollable with [`WALL_ID`] and no message is sent.
pub(crate) fn measure() -> Task<Message> {
    operate(measure_wall(WALL_ID)).map(|size| Message::Wall(WallMsg::Measured(size)))
}

pub(super) fn measure_wall(target: Id) -> impl Operation<Size> {
    struct Measure {
        target: Id,
        found: Option<Size>,
    }

    impl Operation<Size> for Measure {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Size>)) {
            operate(self);
        }

        fn scrollable(
            &mut self,
            id: Option<&Id>,
            bounds: Rectangle,
            _content_bounds: Rectangle,
            _translation: Vector,
            _state: &mut dyn Scrollable,
        ) {
            if Some(&self.target) == id {
                self.found = Some(bounds.size());
            }
        }

        fn finish(&self) -> Outcome<Size> {
            self.found.map_or(Outcome::None, Outcome::Some)
        }
    }

    Measure {
        target,
        found: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::wall::fixture::*;
    use crate::screens::wall::message::Dir;

    #[test]
    fn measuring_unblocks_navigation() {
        let mut state = wall(&SPREAD, 3);
        // Before the first measurement there is no layout, so nav is inert.
        state.viewport = None;
        let _ = state.update(WallMsg::Nav(Dir::Right));
        assert_eq!(state.library.paths.index(), 0);

        let size = Size::new(width_for(3), 1000.0);
        let _ = state.update(WallMsg::Measured(size));
        assert_eq!(state.viewport, Some(size));

        let _ = state.update(WallMsg::Nav(Dir::Right));
        assert_eq!(state.library.paths.index(), 1);
        let _ = state.update(WallMsg::Nav(Dir::Down));
        assert_eq!(state.library.paths.index(), 5);
        let _ = state.update(WallMsg::Nav(Dir::Left));
        assert_eq!(state.library.paths.index(), 3);
    }

    #[test]
    fn sideways_moves_round_trip() {
        let mut state = wall(&SPREAD, 3);
        let _ = state.update(WallMsg::Nav(Dir::Down)); // 0 -> 3
        assert_eq!(state.library.paths.index(), 3);
        // 3's nearest neighbour rightwards is 5, but 5's is 3 only because the
        // sticky centre survives the second press.
        let _ = state.update(WallMsg::Nav(Dir::Right));
        assert_eq!(state.library.paths.index(), 5);
        let _ = state.update(WallMsg::Nav(Dir::Left));
        assert_eq!(state.library.paths.index(), 3);
    }

    #[test]
    fn up_or_down_clears_the_sticky_centre() {
        let mut state = wall(&SPREAD, 3);
        let _ = state.update(WallMsg::Nav(Dir::Down)); // 0 -> 3, centre 382
        let _ = state.update(WallMsg::Nav(Dir::Right)); // -> 5, sticky 382
        let _ = state.update(WallMsg::Nav(Dir::Up)); // -> 1, sticky cleared
        assert_eq!(state.library.paths.index(), 1);
        assert_eq!(state.desired_y, None);
        // Now anchored on 1 (centre 212), not the stale 382, so left gives 0.
        let _ = state.update(WallMsg::Nav(Dir::Left));
        assert_eq!(state.library.paths.index(), 0);
    }
}
