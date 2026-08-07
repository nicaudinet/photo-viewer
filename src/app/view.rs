//! What is on screen: the live screen's own view, plus the overlays that can
//! sit over any of them.

use iced::alignment::Horizontal;
use iced::widget::{center, column, container, row, text, Stack};
use iced::{Background, Border, Color, Element, Length, Theme};

use crate::screens::empty::empty_view;

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
            layers.push(help_overlay());
        }
        if let Some(confirm) = &self.confirm {
            layers.push(confirm_overlay(
                &confirm.prompt,
                confirm.detail.as_deref(),
                &confirm.hint(),
            ));
        }

        if layers.len() == 1 {
            layers.pop().unwrap()
        } else {
            Stack::with_children(layers).into()
        }
    }
}

/// Shown in the help overlay, in press-order.
const SHORTCUTS: &[(&str, &str)] = &[
    ("\u{2190} / h", "Previous image / left"),
    ("\u{2192} / l", "Next image / right"),
    ("\u{2191} / k", "Wall: up a row"),
    ("\u{2193} / j", "Wall: down a row"),
    ("\u{21b5}", "Wall: commit range, else open"),
    ("w", "Wall / single (toggle)"),
    ("r / \u{21e7}R", "Rotate anticlockwise / clockwise"),
    ("o", "Open file"),
    ("v", "Wall: paint a range to select"),
    ("x", "Wall: paint a range to deselect"),
    ("Space", "Wall: select / deselect one"),
    ("\u{2318}A", "Wall: select all"),
    ("i", "Wall: invert selection"),
    ("f", "Favourite (selection, or one)"),
    ("\u{21e7}F", "Wall: show only favourites"),
    ("m", "Wall: move the selection"),
    ("c", "Wall: copy the selection"),
    ("d", "Wall: trash the selection"),
    ("e", "Fullscreen (toggle)"),
    ("?", "Show help (toggle)"),
    ("Esc", "Help, then range, then selection"),
    ("q", "Quit"),
];

/// Shared translucent-panel styling for the overlays.
fn overlay_box(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(Color {
            a: 0.96,
            ..palette.background.weak.color
        })),
        border: Border {
            color: palette.background.strong.color,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    }
}

pub(crate) fn help_overlay() -> Element<'static, Message> {
    let title = text("Keyboard Shortcuts").size(24);
    let rows = SHORTCUTS
        .iter()
        .fold(column![].spacing(10), |col, (keys, desc)| {
            col.push(
                row![
                    text(*keys).size(16).width(Length::Fixed(90.0)),
                    text(*desc).size(16),
                ]
                .spacing(20),
            )
        });

    center(
        container(column![title, rows].spacing(18))
            .padding(28)
            .style(overlay_box),
    )
    .into()
}

/// A modal question. Everything else is swallowed while it is up (see
/// `App::update` and `App::subscription`), so the only ways out are the keys
/// `hint` names — which is why every answer has to appear there.
///
/// `detail` is for what the user needs in order to answer: how many files
/// already exist at the destination, or that moved images will leave the wall.
pub(crate) fn confirm_overlay(
    prompt: &str,
    detail: Option<&str>,
    hint: &str,
) -> Element<'static, Message> {
    let mut body = column![text(prompt.to_string()).size(22)].spacing(12);
    if let Some(detail) = detail {
        body = body.push(text(detail.to_string()).size(15).style(text::secondary));
    }

    center(
        container(
            column![body, text(hint.to_string()).size(14)]
                .spacing(18)
                .align_x(Horizontal::Center),
        )
        .padding(28)
        .style(overlay_box),
    )
    .into()
}
