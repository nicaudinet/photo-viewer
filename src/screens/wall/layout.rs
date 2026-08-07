//! The masonry: a pure function from (viewport width, thumbnail heights) to
//! where every tile sits, plus the geometry queries navigation asks of it.
//!
//! Nothing here builds a widget or reads a message. The view turns a
//! [`WallLayout`] into a tree; `update` reads the same one to answer "what is
//! to the left of the cursor?". Both call the same function over the same
//! state, so keyboard navigation always agrees with what is on screen.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;

use super::message::Dir;
use super::WallState;

/// Thumbnail column width and inter-item spacing (matches the Python wall).
pub(super) const THUMB_WIDTH: u32 = 300;
/// Inter-tile spacing, and the outer padding of the wall (the layout maths in
/// [`WallState::layout`] assumes the two are equal).
pub(super) const WALL_SPACING: f32 = 12.0;
/// Width of the selection ring drawn around the current thumbnail.
///
/// `button` fills its border quad at its full bounds and *then* draws the
/// content on top, so a zero-padding button hides its own border behind an
/// opaque child. Every thumbnail therefore carries this much padding — the ring
/// is only made visible (rather than added) when the thumbnail is selected, so
/// selection never reflows the wall.
pub(super) const SEL_BORDER: f32 = 4.0;
/// A thumbnail plus its selection ring: what the masonry actually places.
pub(super) const TILE_WIDTH: f32 = THUMB_WIDTH as f32 + 2.0 * SEL_BORDER;

/// Where the masonry put one thumbnail. `y` is absolute within the scrollable's
/// content, so it can be compared against the scroll offset directly.
pub(super) struct Slot {
    pub(super) col: usize,
    pub(super) row: usize,
    pub(super) y: f32,
    pub(super) height: f32,
}

/// The result of one masonry pass: pure data, no widgets.
pub(super) struct WallLayout {
    /// Per column, the library indices it holds, top to bottom.
    pub(super) columns: Vec<Vec<usize>>,
    /// Library index -> where it sits.
    pub(super) slots: HashMap<usize, Slot>,
    /// Library indices in library order — the space decode priority is
    /// expressed in.
    pub(super) order: Vec<usize>,
    /// Total scrollable content height, including the outer padding.
    pub(super) content_height: f32,
}

impl WallState {
    /// The height a thumbnail will occupy: its decoded height, else the
    /// header-derived one, else a square guess until either arrives.
    pub(super) fn tile_height(&self, path: &PathBuf) -> f32 {
        self.thumbs
            .get(path)
            .map(|t| t.height as f32)
            .or_else(|| self.ratios.get(path).copied())
            .unwrap_or(THUMB_WIDTH as f32)
    }

    /// Place every thumbnail into the shortest column, as data.
    ///
    /// Deterministic in (`width`, `thumbs`, `ratios`, library order) — which is
    /// exactly why the view and `update` can call it separately and still
    /// agree.
    pub(super) fn layout(&self, width: f32) -> WallLayout {
        // `n` tiles need `n * TILE_WIDTH + (n + 1) * WALL_SPACING` (spacing
        // between them, plus the equal outer padding) — hence the single
        // `WALL_SPACING` subtracted before the division.
        let item_width = WALL_SPACING + TILE_WIDTH;
        let col_count = (((width - WALL_SPACING) / item_width).floor() as usize).max(1);

        let mut columns: Vec<Vec<usize>> = vec![Vec::new(); col_count];
        let mut slots = HashMap::new();
        let mut order = Vec::new();
        // Seeded with the container's top padding, so every `y` is absolute
        // within the scrollable's content rather than relative to the grid.
        let mut heights = vec![WALL_SPACING; col_count];

        for (index, path) in self.library.paths.iter().enumerate() {
            let col = heights
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap_or(0);
            let height = self.tile_height(path);
            slots.insert(
                index,
                Slot {
                    col,
                    row: columns[col].len(),
                    y: heights[col],
                    height,
                },
            );
            columns[col].push(index);
            order.push(index);
            heights[col] += height + 2.0 * SEL_BORDER + WALL_SPACING;
        }

        // The tallest column's running total already includes the bottom
        // padding, since each placement adds a trailing `WALL_SPACING`.
        let content_height = heights.into_iter().fold(0.0_f32, f32::max);
        WallLayout {
            columns,
            slots,
            order,
            content_height,
        }
    }
}

/// The library index one step from `current` in `dir`, or `None` at an edge.
///
/// Up and down are simply the neighbouring row of the same column. Left and
/// right cross to the adjacent column and take whichever thumbnail's vertical
/// centre is nearest — masonry rows are not aligned, so "the same row index" in
/// the next column can be at an entirely different height. `desired_y` overrides
/// the reference centre so a run of sideways moves doesn't drift.
pub(super) fn neighbour(
    layout: &WallLayout,
    current: usize,
    dir: Dir,
    desired_y: Option<f32>,
) -> Option<usize> {
    let slot = layout.slots.get(&current)?;
    match dir {
        Dir::Up => layout.columns[slot.col]
            .get(slot.row.checked_sub(1)?)
            .copied(),
        Dir::Down => layout.columns[slot.col].get(slot.row + 1).copied(),
        Dir::Left | Dir::Right => {
            let col = if dir == Dir::Left {
                slot.col.checked_sub(1)?
            } else {
                slot.col + 1
            };
            let reference = desired_y.unwrap_or(slot.y + slot.height / 2.0);
            layout.columns.get(col)?.iter().copied().min_by(|a, b| {
                distance_to(layout, *a, reference)
                    .partial_cmp(&distance_to(layout, *b, reference))
                    .unwrap_or(Ordering::Equal)
            })
        }
    }
}

/// Vertical centre of `index`'s slot.
pub(super) fn centre_of(layout: &WallLayout, index: usize) -> Option<f32> {
    layout.slots.get(&index).map(|s| s.y + s.height / 2.0)
}

pub(super) fn distance_to(layout: &WallLayout, index: usize, y: f32) -> f32 {
    centre_of(layout, index).map_or(f32::MAX, |c| (c - y).abs())
}

/// Position in [`WallLayout::order`] of the thumbnail whose centre is nearest
/// `y` — decode priority is expressed in that space, not in pixels, so a decode
/// landing can't reshuffle it.
pub(super) fn nearest_position(layout: &WallLayout, y: f32) -> f32 {
    layout
        .order
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            distance_to(layout, **a, y)
                .partial_cmp(&distance_to(layout, **b, y))
                .unwrap_or(Ordering::Equal)
        })
        .map(|(pos, _)| pos as f32)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::wall::fixture::*;
    use crate::screens::wall::message::Dir;

    #[test]
    fn masonry_fills_the_shortest_column() {
        // Heights chosen so placement order is 0->c0, 1->c1, 2->c2, 3->c0
        // (shortest at 220), 4->c2, 5->c1.
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        assert_eq!(layout.columns, vec![vec![0, 3], vec![1, 5], vec![2, 4]]);
    }

    #[test]
    fn left_and_right_pick_the_nearest_vertical_centre() {
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        // 3 sits in column 0 at y=232, centre 382. Column 1 holds 1 (centre
        // 212) and 5 (centre 552) — 5 is nearer. List order would have said 4,
        // which is two columns away.
        assert_eq!(neighbour(&layout, 3, Dir::Right, None), Some(5));
        assert_eq!(neighbour(&layout, 5, Dir::Left, None), Some(3));
    }

    #[test]
    fn up_and_down_stay_in_the_column() {
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        assert_eq!(neighbour(&layout, 0, Dir::Down, None), Some(3));
        assert_eq!(neighbour(&layout, 3, Dir::Up, None), Some(0));
    }

    #[test]
    fn navigation_stops_at_the_edges() {
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        assert_eq!(neighbour(&layout, 0, Dir::Up, None), None);
        assert_eq!(neighbour(&layout, 0, Dir::Left, None), None);
        assert_eq!(neighbour(&layout, 3, Dir::Down, None), None);
        assert_eq!(neighbour(&layout, 4, Dir::Right, None), None);
    }

    #[test]
    fn desired_y_overrides_the_current_centre() {
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        // From 3 (centre 382) the nearest in column 1 is 5, but anchored near
        // the top the answer is 1 instead — this is what stops h/l drifting.
        assert_eq!(neighbour(&layout, 3, Dir::Right, Some(0.0)), Some(1));
    }

    #[test]
    fn single_column_layout_is_the_library_order() {
        let state = wall(&[200.0; 4], 1);
        let layout = state.layout(width_for(1));
        assert_eq!(layout.columns, vec![vec![0, 1, 2, 3]]);
        assert_eq!(neighbour(&layout, 1, Dir::Down, None), Some(2));
        assert_eq!(neighbour(&layout, 1, Dir::Right, None), None);
    }

    #[test]
    fn slot_y_is_absolute_within_the_scroll_content() {
        let state = wall(&[200.0, 200.0], 1);
        let layout = state.layout(width_for(1));
        // First tile starts below the container's top padding, not at 0.
        assert_eq!(layout.slots[&0].y, WALL_SPACING);
        // Second is a tile (200 + 2 * ring) and a gap further down.
        assert_eq!(
            layout.slots[&1].y,
            WALL_SPACING + 200.0 + 2.0 * SEL_BORDER + WALL_SPACING
        );
        // Content height includes the bottom padding.
        assert_eq!(
            layout.content_height,
            layout.slots[&1].y + 200.0 + 2.0 * SEL_BORDER + WALL_SPACING
        );
    }
}
