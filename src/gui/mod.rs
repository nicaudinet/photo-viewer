//! GUI layer: one module per screen, plus the shared widgets, styling, and
//! constants they have in common. `App` (in `main.rs`) owns the screen-
//! independent state and orchestrates transitions between these screens.

pub mod empty;
pub mod single;
pub mod wall;

use iced::alignment::{Horizontal, Vertical};
use iced::widget::{center, column, container, image, row, text};
use iced::{Background, Border, Color, Element, Length, Theme};

use crate::Message;

const ICON_SIZE: f32 = 40.0;
const ICON_MARGIN: f32 = 10.0;

/// Shown in the help overlay, in press-order.
const SHORTCUTS: &[(&str, &str)] = &[
    ("\u{2190} / h", "Previous image"),
    ("\u{2192} / l", "Next image"),
    ("w", "Wall / single (toggle)"),
    ("r / \u{21e7}R", "Rotate anticlockwise / clockwise"),
    ("o", "Open file"),
    ("f", "Favourite / favourites filter"),
    ("\u{2318}F", "Save favourites"),
    ("d", "Mark to delete / to-delete filter"),
    ("\u{2318}D", "Delete all marked"),
    ("e", "Fullscreen (toggle)"),
    ("?", "Show help (toggle)"),
    ("Esc", "Close help"),
    ("q", "Quit"),
];

/// An icon pinned to the top-right corner with a fixed margin. Shared by the
/// single and wall views.
pub(crate) fn corner_icon(handle: image::Handle) -> Element<'static, Message> {
    container(
        image(handle)
            .width(Length::Fixed(ICON_SIZE))
            .height(Length::Fixed(ICON_SIZE)),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Horizontal::Right)
    .align_y(Vertical::Top)
    .padding(ICON_MARGIN)
    .into()
}

/// Shared translucent-panel styling for the help and confirm overlays.
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
    let rows = SHORTCUTS.iter().fold(column![].spacing(10), |col, (keys, desc)| {
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

pub(crate) fn confirm_overlay(count: usize) -> Element<'static, Message> {
    let message = if count == 1 {
        "Delete 1 photo?".to_string()
    } else {
        format!("Delete {count} photos?")
    };
    center(
        container(
            column![
                text(message).size(22),
                text("y / Enter — yes      n / Esc — no").size(14),
            ]
            .spacing(18)
            .align_x(Horizontal::Center),
        )
        .padding(28)
        .style(overlay_box),
    )
    .into()
}
