//! The empty screen: shown when no image is loaded. Carries no state.

use iced::widget::{center, container, text};
use iced::{Border, Element, Theme};

use crate::Message;

pub(crate) fn empty_view() -> Element<'static, Message> {
    let label = text("No image loaded\nPress ? for help!").size(18).center();
    center(
        container(label)
            .padding(60)
            .style(|theme: &Theme| container::Style {
                border: Border {
                    color: theme.extended_palette().background.strong.color,
                    width: 2.0,
                    radius: 4.0.into(),
                },
                ..container::Style::default()
            }),
    )
    .padding(40)
    .into()
}
