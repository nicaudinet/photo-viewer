//! The mode bar along the bottom: what the wall is doing, and the keys that
//! apply to it. Hidden in `Normal` unless the wall is showing a subset of the
//! folder, which the user must never have to guess at.

use iced::alignment::Vertical;
use iced::widget::{container, text, Row, Space};
use iced::{Background, Border, Element, Length, Theme};

use crate::core::library::RangeOp;
use crate::Message;

use super::select::WallMode;
use super::WallState;

/// Height of the mode bar along the bottom (hidden in `Normal`).
pub(super) const BAR_HEIGHT: f32 = 34.0;

impl WallState {
    /// The mode indicator along the bottom, or `None` in `Normal` — where there
    /// is no mode to indicate and nothing to say.
    pub(super) fn mode_bar(&self) -> Option<Element<'_, Message>> {
        // A running batch takes the bar: it is the only thing telling the user
        // that work is under way, and how to stop it.
        if let Some(batch) = &self.batch {
            return Some(self.bar(batch.label(), "Esc cancel"));
        }
        // A fingerprint pass takes it for the same reason: it is the only thing
        // saying that `g` is working rather than ignoring the keypress.
        if let Some(hashing) = &self.hashing {
            return Some(self.bar(hashing.label(), "Esc cancel"));
        }

        let (label, hint) = match self.mode {
            // Nothing to say in `Normal` — unless the wall is showing a subset
            // of the folder, or standing for more photos than it has tiles.
            // Neither is something the user should have to guess at.
            WallMode::Normal => {
                return self.normal_label().map(|label| {
                    // Whichever of the two the user most recently asked for is
                    // the one whose keys they are looking for.
                    let hint = match self.library.grouping {
                        Some(_) => "+ looser \u{b7} - tighter \u{b7} g ungroup",
                        None => "F show all \u{b7} f unfavourite",
                    };
                    self.bar(label, hint)
                });
            }
            WallMode::Visual { anchor, op } => {
                let (total, delta) = self.pending_counts(anchor, op);
                let verb = match op {
                    RangeOp::Add => "add",
                    RangeOp::Remove => "remove",
                };
                (
                    format!("-- VISUAL ({verb}) -- {total} ({delta:+})"),
                    "enter commit \u{b7} Esc cancel",
                )
            }
            WallMode::Select => (
                format!("SELECT {}", self.library.selection.len()),
                "f fav \u{b7} r rotate \u{b7} m move \u{b7} c copy \u{b7} d trash \u{b7} v add \u{b7} x remove \u{b7} Esc clear",
            ),
        };
        Some(self.bar(label, hint))
    }

    /// What the bar says in `Normal`, or `None` when there is nothing to say —
    /// an unfiltered, ungrouped wall shows every photo in the folder, once, and
    /// needs no caption.
    ///
    /// Counted in photos rather than tiles, so the numbers mean files either
    /// way: with stacks the tile count is the smaller one, and it is not what
    /// any command acts on.
    pub(super) fn normal_label(&self) -> Option<String> {
        let head = match &self.library.filter {
            Some(tag) => format!("{} \u{2014} ", tag.to_uppercase()),
            None => String::new(),
        };
        let photos = self.library.photos().count();
        match &self.library.grouping {
            Some(grouping) => Some(format!(
                "{head}{} \u{b7} {} \u{b7} d\u{2264}{}",
                plural(grouping.len(), "stack"),
                plural(photos, "photo"),
                grouping.threshold().distance(),
            )),
            None => self
                .library
                .filter
                .as_ref()
                .map(|_| format!("{head}{photos} of {}", self.library.all.len())),
        }
    }

    /// The bar itself: a label on the left, the keys that apply on the right.
    pub(super) fn bar<'a>(&self, label: String, hint: &'a str) -> Element<'a, Message> {
        container(
            Row::with_children(vec![
                text(label).size(14).into(),
                Space::new().width(Length::Fill).into(),
                text(hint).size(13).style(text::secondary).into(),
            ])
            .align_y(Vertical::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(BAR_HEIGHT))
        .padding([0, 12])
        .align_y(Vertical::Center)
        .style(mode_bar_style)
        .into()
    }

    /// What the painted range would leave selected: the resulting total, and
    /// the signed change. Shown live so a range is never committed blind.
    pub(super) fn pending_counts(&self, anchor: usize, op: RangeOp) -> (usize, i64) {
        let cursor = self.library.paths.index();
        let (lo, hi) = (anchor.min(cursor), anchor.max(cursor));
        // Counted in photos, not tiles: a range covering one stack of four says
        // four, because four files are what the next command will act on.
        let touched = self
            .library
            .paths
            .iter()
            .skip(lo)
            .take(hi + 1 - lo)
            .flat_map(|p| self.library.members(p))
            .filter(|p| match op {
                RangeOp::Add => !self.library.is_selected(p),
                RangeOp::Remove => self.library.is_selected(p),
            })
            .count();
        let current = self.library.selection.len();
        match op {
            RangeOp::Add => (current + touched, touched as i64),
            RangeOp::Remove => (current - touched, -(touched as i64)),
        }
    }
}

/// `1 stack`, `2 stacks`. The bar is read at a glance and often enough that a
/// missing plural reads as a bug in the count.
fn plural(n: usize, thing: &str) -> String {
    match n {
        1 => format!("1 {thing}"),
        n => format!("{n} {thing}s"),
    }
}

pub(super) fn mode_bar_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(palette.background.weak.color)),
        text_color: Some(palette.background.base.text),
        border: Border {
            color: palette.background.strong.color,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use crate::core::library::RangeOp;
    use crate::screens::wall::fixture::*;
    use crate::screens::wall::message::Dir;
    use crate::screens::wall::message::WallMsg;

    #[test]
    fn the_bar_counts_what_committing_would_leave_selected() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::ToggleCursor); // select 0

        state.library.goto(2);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down); // range 2..=3, neither selected
        assert_eq!(state.pending_counts(2, RangeOp::Add), (3, 2));
    }

    #[test]
    fn the_bar_does_not_double_count_an_overlapping_range() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::SelectAll);
        state.library.goto(1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);
        // Everything in 1..=2 is already selected: committing changes nothing.
        assert_eq!(state.pending_counts(1, RangeOp::Add), (6, 0));
    }

    #[test]
    fn the_bar_says_nothing_about_a_plain_folder() {
        let state = wall(&[200.0; 6], 1);
        // Every photo, once: there is nothing the user could be surprised by.
        assert_eq!(state.normal_label(), None);
    }

    #[test]
    fn the_bar_counts_stacks_and_photos() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 1, 2, 40, 41, 42]);
        assert_eq!(
            state.normal_label().unwrap(),
            "2 stacks \u{b7} 6 photos \u{b7} d\u{2264}10"
        );
    }

    #[test]
    fn the_bar_says_which_rung_the_dial_is_on() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 1, 2, 40, 41, 42]);
        let _ = state.update(WallMsg::Retune { looser: true });
        assert!(state.normal_label().unwrap().ends_with("d\u{2264}14"));
    }

    #[test]
    fn the_filter_and_the_stacks_share_the_bar() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(4);
        fav(&mut state);
        state.library.goto(5);
        fav(&mut state);
        filter(&mut state);
        group(&mut state, &[0, 0]);

        // Both narrow what is on the wall, so both have to be visible at once.
        assert_eq!(
            state.normal_label().unwrap(),
            "FAVOURITE \u{2014} 1 stack \u{b7} 2 photos \u{b7} d\u{2264}10"
        );
    }

    #[test]
    fn the_bar_counts_a_removal_downwards() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::SelectAll);
        state.library.goto(1);
        enter_visual(&mut state, RangeOp::Remove);
        nav(&mut state, Dir::Down);
        assert_eq!(state.pending_counts(1, RangeOp::Remove), (4, -2));
    }
}
