//! What is on screen: the live screen's own view, plus the overlays that can
//! sit over any of them.

use iced::alignment::Horizontal;
use iced::widget::{center, column, container, text, Stack};
use iced::{Background, Border, Element, Theme};

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
