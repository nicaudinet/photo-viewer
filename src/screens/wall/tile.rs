//! How one tile is decorated.
//!
//! Independent channels, so a tile can say several things at once: the ring is
//! where the cursor is, the tint and badge are what is selected, and the fanned
//! cards and `×n` badge are how many photographs the tile stands for.

use iced::alignment::{Horizontal, Vertical};
use iced::theme::palette::lighten;
use iced::widget::{button, container, image, text};
use iced::{
    Background, Border, Color, ContentFit, Element, Length, Radians, Rotation, Shadow, Theme,
};

use crate::core::library::{RangeOp, Selected};
use crate::core::tags;
use crate::Message;

use super::layout::{SEL_BORDER, THUMB_WIDTH};
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

/// How far the cards behind a stack lean, in degrees. Small: enough that the
/// corners show past the photograph, not so much that the pile looks dropped.
const FAN_DEGREES: f32 = 5.0;
/// How far the photograph is pulled in from the edge of its tile to leave room
/// for the cards behind it. See [`fan_inset`].
///
/// A stack cannot simply overhang its tile. iced clips a rotated image to its
/// *unrotated* layout bounds, and a `Stack` gives every layer at most the size
/// of its base layer, so a card can never be drawn outside the tile it is in:
/// left at full size it would be clipped back to the photograph's own rect and
/// then hidden behind it. The pile is therefore made room for rather than
/// allowed to spill — which also means it costs the masonry nothing, since the
/// tile is exactly the size it always was.
const FAN_INSET: f32 = 10.0;
/// The colour of a blank card, and how solid it is drawn. Grey rather than
/// paper-white so it does not glare on the dark theme, and slightly transparent
/// so it settles behind the photograph instead of competing with it.
const CARD_RGBA: [u8; 4] = [0x8a, 0x8a, 0x8e, 0xff];
const CARD_OPACITY: f32 = 0.75;

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
        Accent::Stack => lighten(palette.secondary.base.color, SEL_LIGHTEN),
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
pub(super) fn count_badge(size: usize) -> Element<'static, Message> {
    corner_badge(
        format!("\u{d7}{size}"),
        Accent::Stack,
        Horizontal::Right,
        Vertical::Bottom,
    )
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
    .align_y(edge)
    .padding(ICON_MARGIN)
    .into()
}

/// One blank card behind a stack's thumbnail, filling the tile and leaning
/// `degrees`. What shows of it is the wedge between its edge and the inset
/// photograph, which is wider at one pair of corners than the other — the whole
/// of the effect.
///
/// A stretched single pixel rather than a styled container, because rotation in
/// iced belongs to images alone — which also means the colour is baked into the
/// pixel instead of coming from the palette. A neutral grey is the one choice
/// that reads against a light theme and a dark one, and the card is a backdrop:
/// all it has to do is not be the photograph.
///
/// [`Rotation::Floating`] keeps the layout bounds it was given, so a leaning
/// card never reflows the masonry around it.
pub(super) fn back_card(height: f32, degrees: f32) -> Element<'static, Message> {
    image(card_pixel())
        .width(Length::Fixed(THUMB_WIDTH as f32))
        .height(Length::Fixed(height))
        .content_fit(ContentFit::Fill)
        // Nothing to interpolate in one pixel, and nearest keeps the edges
        // crisp where the card leans out from behind the photo.
        .filter_method(image::FilterMethod::Nearest)
        .rotation(Rotation::Floating(Radians(degrees.to_radians())))
        .opacity(CARD_OPACITY)
        .into()
}

/// The one pixel every card is drawn from.
///
/// Built once and cloned. `Handle::from_rgba` mints a fresh id per call, so a
/// handle built per tile would upload a new texture for every stack on every
/// frame.
fn card_pixel() -> image::Handle {
    thread_local! {
        static CARD: image::Handle = image::Handle::from_rgba(1, 1, CARD_RGBA.to_vec());
    }
    CARD.with(image::Handle::clone)
}

/// How far the cards behind a stack lean, and how many of them there are.
///
/// Two at most however deep the pile is: a third adds no information the `×n`
/// badge is not already giving, and every card is one more thing leaning into
/// the tiles beside it. A pair of photos gets a single card, so that what is
/// drawn matches what is there.
pub(super) fn fan_angles(size: usize) -> Vec<f32> {
    match size {
        0 | 1 => Vec::new(),
        2 => vec![FAN_DEGREES],
        // Opposite ways, which is what makes it read as a pile pushed about
        // rather than as one photo printed twice.
        _ => vec![-FAN_DEGREES, FAN_DEGREES],
    }
}

/// How far in from its tile the photograph of a stack of `size` is drawn.
pub(super) fn fan_inset(size: usize) -> f32 {
    if fan_angles(size).is_empty() {
        0.0
    } else {
        FAN_INSET
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
        // Which is what keeps the cards and the badge off an ungrouped wall:
        // both are drawn from this number.
        assert_eq!(state.tile_look(0, &path, 0).stack, 1);
        assert!(fan_angles(1).is_empty());
    }

    #[test]
    fn a_pair_of_photos_shows_one_card() {
        // Two photos, two rectangles: a second card would draw a third photo
        // that is not there.
        assert_eq!(fan_angles(2), vec![FAN_DEGREES]);
    }

    #[test]
    fn only_a_stack_makes_room_for_cards() {
        // A lone photograph fills its tile exactly as it did before stacks
        // existed, so grouping never changes the size of anything that is not
        // a stack.
        assert_eq!(fan_inset(1), 0.0);
        assert!(fan_inset(2) > 0.0);
    }

    #[test]
    fn a_deeper_pile_shows_two_cards_leaning_opposite_ways() {
        assert_eq!(fan_angles(3), vec![-FAN_DEGREES, FAN_DEGREES]);
        // However deep it gets: past two, the `\u{d7}n` badge is what says how
        // many, and more cards only lean further into the neighbouring tiles.
        assert_eq!(fan_angles(40), fan_angles(3));
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
