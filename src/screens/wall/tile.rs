//! How one tile is decorated.
//!
//! Independent channels, so a tile can say several things at once: the ring is
//! where the cursor is, the tint and badge are what is selected, and the count
//! badge is how many photographs the tile stands for.

use iced::alignment::{Horizontal, Vertical};
use iced::theme::palette::lighten;
use iced::widget::{button, container, text};
use iced::{Background, Border, Color, Element, Length, Shadow, Theme};

use crate::core::library::{RangeOp, Selected};
use crate::core::tags;
use crate::Message;

use super::layout::SEL_BORDER;
use super::select::WallMode;
use super::WallState;
use crate::screens::ICON_MARGIN;

/// How far an accent ring is lifted off its palette colour (OKLCH lightness).
/// On the dark theme this takes the primary `#5865F2` to `#8B8BFF`.
pub(super) const SEL_LIGHTEN: f32 = 0.25;
/// Tint alpha over a committed selection, and over a range still being painted.
/// The pending one is lighter so an uncommitted range never looks decided.
pub(super) const TINT_COMMITTED: f32 = 0.32;
pub(super) const TINT_PENDING: f32 = 0.18;
/// Tint over a stack only some of whose photos are selected. Half of a whole
/// one, so a partly-selected pile cannot be mistaken for a decided one.
pub(super) const TINT_PARTIAL: f32 = 0.16;

/// The diameter of the count disc, how much wider each digit past the first
/// makes it, and how big the number inside it is drawn. A disc that grew to fit
/// a three-digit count would swallow the corner of the photograph, so it
/// stretches sideways into a pill instead.
const COUNT_SIZE: f32 = 30.0;
const COUNT_DIGIT: f32 = 12.0;
const COUNT_TEXT: f32 = 19.0;

/// How one tile is decorated. `ring` is the border colour, `tint` a
/// translucent wash over the thumbnail, `badge` the corner checkmark.
#[derive(Clone, Copy)]
pub(super) struct TileLook {
    pub(super) ring: Option<Accent>,
    pub(super) tint: Option<Accent>,
    pub(super) tint_alpha: f32,
    pub(super) badge: bool,
    pub(super) star: bool,
    /// How many photos the tile stands for: one, or the depth of its stack.
    pub(super) stack: usize,
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
    /// How deep a stack is, which is a fact about the folder rather than
    /// anything the user has done to it — so, alone among these, neutral.
    Stack,
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
        // Near-white, from no palette slot either: the count sits on the
        // photograph rather than on the background, and a photograph is as
        // likely to be dark under one theme as the other. A light disc with a
        // dark number on it is the one pairing that reads over both.
        Accent::Stack => Color::from_rgb(0.95, 0.95, 0.97),
    }
}

/// What a mark's symbol is drawn in: black on the pale accents, which white
/// would disappear into, white on the saturated ones.
fn badge_ink(accent: Accent) -> Color {
    match accent {
        Accent::Favourite | Accent::Stack => Color::BLACK,
        _ => Color::WHITE,
    }
}

/// The favourite marker. Public so the single view draws exactly the same one:
/// two screens disagreeing about what a favourite looks like would be worse
/// than either choice.
pub(crate) fn favourite_star() -> Element<'static, Message> {
    corner_badge(
        "\u{2605}".to_string(),
        Accent::Favourite,
        Horizontal::Left,
        Vertical::Top,
    )
}

/// How many photos a stack stands for, in the corner the other two marks leave
/// free — the star is top-left and the selection tick top-right.
///
/// The count alone on a disc. With nothing drawn behind the thumbnail this is
/// the only thing saying a tile stands for more than one photograph, so it is a
/// filled circle rather than a `×n` pill: a shape the eye picks out while
/// scanning a wall, which is how Google Photos marks a stack.
pub(super) fn count_badge(size: usize) -> Element<'static, Message> {
    let label = size.to_string();
    in_corner(
        container(
            text(label.clone())
                .size(COUNT_TEXT)
                .color(badge_ink(Accent::Stack)),
        )
        .center_x(Length::Fixed(count_width(label.len())))
        .center_y(Length::Fixed(COUNT_SIZE))
        .style(|theme: &Theme| pill_style(theme, Accent::Stack))
        .into(),
        Horizontal::Right,
        Vertical::Bottom,
    )
}

/// How wide the count disc is for a count of `digits` digits: a circle at one
/// digit, stretched by [`COUNT_DIGIT`] for each one after it.
fn count_width(digits: usize) -> f32 {
    COUNT_SIZE + COUNT_DIGIT * digits.saturating_sub(1) as f32
}

/// A symbol in a rounded pill in one corner of a tile.
///
/// A shape as well as a colour, so neither selection nor favouriting is carried
/// by hue alone.
pub(super) fn corner_badge(
    symbol: String,
    accent: Accent,
    side: Horizontal,
    edge: Vertical,
) -> Element<'static, Message> {
    in_corner(
        container(text(symbol).size(18).color(badge_ink(accent)))
            .padding([1, 7])
            .style(move |theme: &Theme| pill_style(theme, accent))
            .into(),
        side,
        edge,
    )
}

/// A mark pinned to one corner of a tile, inside the margin every icon keeps.
fn in_corner(
    mark: Element<'static, Message>,
    side: Horizontal,
    edge: Vertical,
) -> Element<'static, Message> {
    container(mark)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(side)
        .align_y(edge)
        .padding(ICON_MARGIN)
        .into()
}

/// The filled, fully rounded background every corner mark is drawn on.
fn pill_style(theme: &Theme, accent: Accent) -> container::Style {
    container::Style {
        background: Some(Background::Color(accent_color(theme, accent))),
        border: Border {
            radius: 999.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

impl WallState {
    /// How one tile is decorated.
    ///
    /// Independent channels, so a tile can say several things at once: the
    /// ring is where the cursor is, the tint and badge are what is selected,
    /// and `stack` is how many photographs it stands for. The
    /// cursor ring is hidden in `Visual` — the leading edge of the painted
    /// range already shows where the cursor is, and a second highlight
    /// competing with the tint just reads as noise.
    pub(super) fn tile_look(
        &self,
        index: usize,
        path: &std::path::Path,
        current: usize,
    ) -> TileLook {
        let selected = self.library.selected(path);
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
                None => (selected != Selected::None).then_some(Accent::Select),
            }
        };

        let (tint, tint_alpha) = match (pending, selected) {
            (Some(RangeOp::Add), _) => (Some(Accent::Select), TINT_PENDING),
            (Some(RangeOp::Remove), _) => (Some(Accent::Remove), TINT_PENDING),
            (None, Selected::All) => (Some(Accent::Select), TINT_COMMITTED),
            (None, Selected::Some) => (Some(Accent::Select), TINT_PARTIAL),
            (None, Selected::None) => (None, 0.0),
        };

        TileLook {
            ring,
            tint,
            tint_alpha,
            // Kept on a tile pending removal: it is still selected until the
            // range is committed, and saying otherwise would pre-empt the user.
            // Only a whole stack earns one — half of one is not a decision the
            // badge can state.
            badge: selected == Selected::All,
            // Redundant while the wall is filtered to the favourites — every
            // tile would carry one, which says nothing.
            star: self.library.filter.is_none() && self.library.is_tagged(tags::FAVOURITE, path),
            stack: self.library.stack_size(path),
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
    use crate::core::library::RangeOp;
    use crate::screens::wall::fixture::*;
    use crate::screens::wall::message::Dir;
    use crate::screens::wall::message::WallMsg;
    use std::path::PathBuf;

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

    // --- Stacks ---

    #[test]
    fn a_tile_says_how_many_photos_it_stands_for() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 1, 2, 40, 41, 42]);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        assert_eq!(state.tile_look(0, &paths[0], 0).stack, 3);
    }

    #[test]
    fn a_photo_on_its_own_stands_for_one() {
        let state = wall(&[200.0; 6], 1);
        let path = state.library.current().clone();
        // Which is what keeps the count badge off an ungrouped wall: it is
        // drawn from this number.
        assert_eq!(state.tile_look(0, &path, 0).stack, 1);
    }

    #[test]
    fn a_one_digit_count_is_drawn_on_a_circle() {
        assert_eq!(count_width(1), COUNT_SIZE);
    }

    #[test]
    fn a_longer_count_stretches_the_disc_rather_than_spilling_out_of_it() {
        assert!(count_width(2) > count_width(1));
        assert!(count_width(3) > count_width(2));
    }

    #[test]
    fn a_half_selected_stack_is_tinted_but_not_badged() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 1, 2, 40, 41, 42]);
        let paths: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        let members = state.library.members(&paths[0]);
        state.library.selection.insert(members[1].clone());

        let look = state.tile_look(0, &paths[0], 0);
        assert_eq!(look.tint_alpha, TINT_PARTIAL);
        // Half a pile is not a decision the badge can state.
        assert!(!look.badge);
        assert_eq!(look.ring, Some(Accent::Cursor));
    }
}
