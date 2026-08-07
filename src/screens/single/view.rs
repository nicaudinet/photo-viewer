//! The widget tree: one fit-to-window image, and the favourite marker over it.

use iced::widget::{image, Space, Stack};
use iced::{ContentFit, Element, Length};

use crate::core::tags;
use crate::Message;

use super::SingleState;

impl SingleState {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let photo: Element<'_, Message> = match &self.large {
            Some(handle) => image(handle.clone())
                .content_fit(ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            None => Space::new()
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        };

        // Same star, same corner as on the wall, so the two screens agree about
        // what a favourite looks like.
        if self.library.is_tagged(tags::FAVOURITE, self.library.current()) {
            Stack::with_children(vec![photo, crate::screens::wall::favourite_star()]).into()
        } else {
            photo
        }
    }

}
