//! What is on screen: the live screen's own view, plus the overlays that can
//! sit over any of them.

use iced::alignment::Horizontal;
use iced::font::Weight;
use iced::theme::palette::lighten;
use iced::widget::{center, column, container, row, text, Row as WRow, Stack};
use iced::{Background, Border, Element, Font, Length, Theme};

use crate::keymap::Row;
use crate::screens::empty::empty_view;

use super::help::help_overlay;
use super::{App, Message, Screen};

impl App {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match &self.screen {
            Screen::Empty => empty_view(),
            Screen::Single(s) => s.view(),
            Screen::Wall(w) => w.view(),
        };

        let mut layers: Vec<Element<'_, Message>> = vec![content];
        if self.help_open {
            layers.push(help_overlay(self));
        }
        if let Some(confirm) = &self.confirm {
            layers.push(confirm_overlay(
                &confirm.prompt,
                confirm.detail.as_deref(),
                confirm.answers(),
            ));
        }

        // Always a stack, even with nothing over the screen. iced keeps widget
        // state by position in the tree and throws it away when the widget
        // there changes kind, so a root that turned into a `Stack` the moment
        // an overlay appeared would take the wall's scroll position with it —
        // the wall would jump to the top behind the help. Layers are added and
        // removed above it; the screen itself stays the first child throughout.
        Stack::with_children(layers).into()
    }
}

/// Shared panel styling for the overlays: opaque, so nothing underneath
/// competes with what the panel has to say.
pub(super) fn overlay_box(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(palette.background.weak.color)),
        border: Border {
            color: palette.background.strong.color,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    }
}

/// Width of the help's key column. Wide enough for `space` and for a movement
/// row's four keys, narrow enough that the sentences beside them stay in one
/// block. The keys are set flush right inside it, so however long the spelling,
/// it ends up against the sentence it belongs to rather than adrift from it.
const KEY_WIDTH: f32 = 84.0;

/// Width of a question's key column. Every answer is a single letter, so the
/// column only has to hold one — and a wide one would push the answers off
/// centre under the question, since the empty half of it is still laid out.
const ANSWER_KEY_WIDTH: f32 = 14.0;

/// How far the key colour is lifted off the palette's primary — the same lift
/// the wall gives the cursor ring, so the accent means one thing in the app.
const KEY_LIGHTEN: f32 = 0.25;

/// Bold, so a key is told from the sentence about it by weight as well as by
/// colour — an overlay is read by scanning the left edge for a key.
const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};

/// One key and what it does. The help lists what can be done; a question lists
/// what can be answered — the same shape, so a key looks the same wherever the
/// user meets it, and the answers to a question read as keys rather than as a
/// sentence about keys.
///
/// `key_width` is how much room the key column takes: the two overlays hold
/// different keys, and the column is only as wide as its own list needs.
fn key_row(Row { keys, desc }: Row, key_width: f32) -> WRow<'static, Message> {
    row![
        text(keys)
            .size(15)
            .font(BOLD)
            .style(key_colour)
            .width(Length::Fixed(key_width))
            .align_x(Horizontal::Right),
        text(desc).size(15),
    ]
    .spacing(12)
}

/// One line of the help: a key, and the sentence about it.
pub(super) fn help_row(row: Row) -> WRow<'static, Message> {
    key_row(row, KEY_WIDTH)
}

/// The accent the wall paints the cursor in, applied to the keys themselves.
fn key_colour(theme: &Theme) -> text::Style {
    let palette = theme.extended_palette();
    text::Style {
        color: Some(lighten(palette.primary.base.color, KEY_LIGHTEN)),
    }
}

/// A modal question. Everything else is swallowed while it is up (see
/// `App::update` and `App::subscription`), so the only ways out are the keys
/// `answers` names — which is why every answer has to appear there.
///
/// `detail` is for what the user needs in order to answer: how many files
/// already exist at the destination, or that moved images will leave the wall.
pub(crate) fn confirm_overlay(
    prompt: &str,
    detail: Option<&str>,
    answers: Vec<Row>,
) -> Element<'static, Message> {
    let mut asked = column![text(prompt.to_string()).size(20)].spacing(8);
    if let Some(detail) = detail {
        asked = asked.push(text(detail.to_string()).size(14).style(text::secondary));
    }

    // Ranged against each other, and centred under the question as one block:
    // the key column only reads as a column if every key is in it, so the rows
    // are not centred one by one.
    let answers = answers.into_iter().fold(column![].spacing(4), |list, row| {
        list.push(key_row(row, ANSWER_KEY_WIDTH))
    });

    center(
        container(
            column![asked, answers]
                .spacing(18)
                .align_x(Horizontal::Center),
        )
        .padding(20)
        .style(overlay_box),
    )
    .into()
}
