//! How one tile is decorated.
//!
//! Two independent channels, so a tile can say two things at once: the ring is
//! where the cursor is, the tint and badge are what is selected.

use iced::alignment::{Horizontal, Vertical};
use iced::theme::palette::lighten;
use iced::widget::{button, container, text};
use iced::{Background, Border, Color, Element, Length, Shadow, Theme};

use crate::core::library::RangeOp;
use crate::core::tags;
use crate::Message;

use super::layout::SEL_BORDER;
use super::select::WallMode;
use crate::screens::ICON_MARGIN;
use super::WallState;

/// How far an accent ring is lifted off its palette colour (OKLCH lightness).
/// On the dark theme this takes the primary `#5865F2` to `#8B8BFF`.
pub(super) const SEL_LIGHTEN: f32 = 0.25;
/// Tint alpha over a committed selection, and over a range still being painted.
/// The pending one is lighter so an uncommitted range never looks decided.
pub(super) const TINT_COMMITTED: f32 = 0.32;
pub(super) const TINT_PENDING: f32 = 0.18;

/// How one tile is decorated. `ring` is the border colour, `tint` a
/// translucent wash over the thumbnail, `badge` the corner checkmark.
#[derive(Clone, Copy)]
pub(super) struct TileLook {
    pub(super) ring: Option<Accent>,
    pub(super) tint: Option<Accent>,
    pub(super) tint_alpha: f32,
    pub(super) badge: bool,
    pub(super) star: bool,
}

/// The three things a tile can be saying, mapped to palette colours by
/// [`accent_color`]. Distinct hues, not shades of one: cursor and selection
/// have to be told apart at a glance across a wall of photos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Accent {
    Cursor,
    Select,
    Remove,
    Favourite,
}

pub(super) fn accent_color(theme: &Theme, accent: Accent) -> Color {
    let palette = theme.extended_palette();
    match accent {
        // `primary.strong` is only a 0.10 lift off the base; take a bigger one
        // so the ring reads clearly against dark thumbnails.
        Accent::Cursor => lighten(palette.primary.base.color, SEL_LIGHTEN),
        Accent::Select => lighten(palette.success.base.color, SEL_LIGHTEN),
        Accent::Remove => lighten(palette.danger.base.color, SEL_LIGHTEN),
        // Amber, from no palette slot — the four palette roles are spoken for,
        // and a favourite has to be told apart from a selection at a glance.
        Accent::Favourite => Color::from_rgb(0.98, 0.75, 0.18),
    }
}

/// The favourite marker. Public so the single view draws exactly the same one:
/// two screens disagreeing about what a favourite looks like would be worse
/// than either choice.
pub(crate) fn favourite_star() -> Element<'static, Message> {
    corner_badge("\u{2605}", Accent::Favourite, Horizontal::Left)
}

/// A symbol in a rounded pill in one corner of a tile.
///
/// A shape as well as a colour, so neither selection nor favouriting is carried
/// by hue alone.
pub(super) fn corner_badge(symbol: &'static str, accent: Accent, side: Horizontal) -> Element<'static, Message> {
    // The amber is light enough that white on it would not read.
    let fg = match accent {
        Accent::Favourite => Color::BLACK,
        _ => Color::WHITE,
    };
    container(
        container(text(symbol).size(18).color(fg))
            .padding([1, 7])
            .style(move |theme: &Theme| container::Style {
                background: Some(Background::Color(accent_color(theme, accent))),
                border: Border {
                    radius: 999.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(side)
    .align_y(Vertical::Top)
    .padding(ICON_MARGIN)
    .into()
}

impl WallState {
    /// How one tile is decorated.
    ///
    /// Two independent channels, so a tile can say two things at once: the ring
    /// is where the cursor is, the tint and badge are what is selected. The
    /// cursor ring is hidden in `Visual` — the leading edge of the painted
    /// range already shows where the cursor is, and a second highlight
    /// competing with the tint just reads as noise.
    pub(super) fn tile_look(&self, index: usize, path: &std::path::Path, current: usize) -> TileLook {
        let selected = self.library.is_selected(path);
        let pending = match self.mode {
            WallMode::Visual { anchor, op } => {
                let (lo, hi) = (anchor.min(current), anchor.max(current));
                (lo..=hi).contains(&index).then_some(op)
            }
            _ => None,
        };

        let cursor = index == current && !self.is_visual();
        let ring = if cursor {
            // The cursor wins the ring: it is the thing that moves, so it has
            // to stay findable. Selection still shows through tint and badge.
            Some(Accent::Cursor)
        } else {
            match pending {
                Some(RangeOp::Add) => Some(Accent::Select),
                Some(RangeOp::Remove) => Some(Accent::Remove),
                None => selected.then_some(Accent::Select),
            }
        };

        let (tint, tint_alpha) = match pending {
            Some(RangeOp::Add) => (Some(Accent::Select), TINT_PENDING),
            Some(RangeOp::Remove) => (Some(Accent::Remove), TINT_PENDING),
            None if selected => (Some(Accent::Select), TINT_COMMITTED),
            None => (None, 0.0),
        };

        TileLook {
            ring,
            tint,
            tint_alpha,
            // Kept on a tile pending removal: it is still selected until the
            // range is committed, and saying otherwise would pre-empt the user.
            badge: selected,
            // Redundant while the wall is filtered to the favourites — every
            // tile would carry one, which says nothing.
            star: self.library.filter.is_none() && self.library.is_tagged(tags::FAVOURITE, path),
        }
    }
}

pub(super) fn thumb_button_style(theme: &Theme, ring: Option<Accent>) -> button::Style {
    let palette = theme.extended_palette();
    button::Style {
        background: None,
        text_color: palette.background.base.text,
        // Always the same width — only the colour changes — so decorating a
        // thumbnail never shifts the masonry.
        border: Border {
            color: ring.map_or(Color::TRANSPARENT, |a| accent_color(theme, a)),
            width: SEL_BORDER,
            radius: 0.0.into(),
        },
        shadow: Shadow::default(),
        // 0.14 added pixel-grid snapping; keep the non-crisp default.
        snap: false,
    }
}

pub(super) fn placeholder_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(palette.background.weak.color)),
        ..container::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::wall::fixture::*;
    use std::path::PathBuf;
    use crate::core::library::RangeOp;
    use crate::screens::wall::message::Dir;
    use crate::screens::wall::message::WallMsg;

    #[test]
    fn the_cursor_ring_is_hidden_while_painting() {
        let mut state = wall(&[200.0; 6], 1);
        let path = state.library.current().clone();
        assert_eq!(state.tile_look(0, &path, 0).ring, Some(Accent::Cursor));

        enter_visual(&mut state, RangeOp::Add);
        // In `Visual` the leading edge of the range already shows the cursor;
        // a second highlight competing with the tint just reads as noise.
        assert_eq!(state.tile_look(0, &path, 0).ring, Some(Accent::Select));
    }

    #[test]
    fn a_pending_range_is_tinted_more_faintly_than_a_committed_one() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);

        let pending = state.tile_look(1, &paths[1], 1);
        assert_eq!(pending.tint, Some(Accent::Select));
        assert_eq!(pending.tint_alpha, TINT_PENDING);
        assert!(!pending.badge);

        let _ = state.update(WallMsg::CommitVisual);
        let committed = state.tile_look(1, &paths[1], 1);
        assert_eq!(committed.tint_alpha, TINT_COMMITTED);
        assert!(committed.badge);
    }

    #[test]
    fn a_tile_pending_removal_keeps_its_badge() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        let _ = state.update(WallMsg::SelectAll);
        enter_visual(&mut state, RangeOp::Remove);

        let look = state.tile_look(0, &paths[0], 0);
        assert_eq!(look.tint, Some(Accent::Remove));
        // Still selected until the range is committed; saying otherwise would
        // pre-empt the user.
        assert!(look.badge);
    }

    #[test]
    fn tiles_outside_the_range_are_undecorated() {
        let mut state = wall(&[200.0; 6], 1);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        enter_visual(&mut state, RangeOp::Add);

        let look = state.tile_look(3, &paths[3], 0);
        assert_eq!(look.ring, None);
        assert_eq!(look.tint, None);
        assert!(!look.badge);
    }
}
