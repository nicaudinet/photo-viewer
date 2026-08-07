//! The widget tree. Reads state, writes none of it.

use std::path::PathBuf;

use iced::alignment::Horizontal;
use iced::widget::{button, container, image, scrollable, text, Column, Row, Space, Stack};
use iced::{Background, Color, Element, Length, Theme};

use crate::Message;

use super::layout::{WallLayout, SEL_BORDER, THUMB_WIDTH, TILE_WIDTH, WALL_SPACING};
use super::message::WallMsg;
use super::navigate::WALL_ID;
use super::tile::{accent_color, corner_badge, favourite_star, placeholder_style, thumb_button_style, Accent};
use super::WallState;

impl WallState {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        // The scrollable is rendered even before the first measurement, empty:
        // `measure` has to find it in the tree to report its size at all.
        let grid: Element<'_, Message> = match self.viewport {
            Some(viewport) => self.build_grid(&self.layout(viewport.width)),
            None => Column::new().into(),
        };

        let centered = container(grid)
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding(WALL_SPACING);
        let wall = scrollable(centered)
            .id(WALL_ID)
            .on_scroll(|viewport| Message::Wall(WallMsg::Scrolled(viewport.absolute_offset().y)))
            .height(Length::Fill);

        match self.mode_bar() {
            // Below the wall rather than over it: with no cursor ring to look
            // for in `Visual`, the bar is the only thing saying which mode is
            // live, so it must not be able to hide behind a thumbnail.
            Some(bar) => Column::with_children(vec![wall.into(), bar]).into(),
            None => wall.into(),
        }
    }

    /// Turn a computed [`WallLayout`] into the column-of-thumbnails widget tree.
    pub(super) fn build_grid<'a>(&'a self, layout: &WallLayout) -> Element<'a, Message> {
        let current = self.library.paths.index();
        let paths: Vec<&'a PathBuf> = self.library.paths.iter().collect();

        let columns: Vec<Element<'a, Message>> = layout
            .columns
            .iter()
            .map(|indices| {
                let items: Vec<Element<'a, Message>> = indices
                    .iter()
                    .filter_map(|&index| {
                        let path = *paths.get(index)?;
                        Some(self.thumb_element(index, path, current))
                    })
                    .collect();
                Column::with_children(items)
                    .spacing(WALL_SPACING)
                    .width(Length::Fixed(TILE_WIDTH))
                    .into()
            })
            .collect();

        Row::with_children(columns).spacing(WALL_SPACING).into()
    }

    /// One thumbnail: image (or placeholder), clickable, decorated according to
    /// [`WallState::tile_look`].
    pub(super) fn thumb_element<'a>(
        &'a self,
        index: usize,
        path: &'a PathBuf,
        current: usize,
    ) -> Element<'a, Message> {
        let height = self.tile_height(path);
        let inner: Element<'a, Message> = match self.thumbs.get(path) {
            Some(thumb) => image(thumb.handle.clone())
                .width(Length::Fixed(THUMB_WIDTH as f32))
                .height(Length::Fixed(thumb.height as f32))
                .into(),
            None => container(text("Loading\u{2026}").size(14))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .width(Length::Fixed(THUMB_WIDTH as f32))
                .height(Length::Fixed(height))
                .style(placeholder_style)
                .into(),
        };

        let look = self.tile_look(index, path, current);

        // Layers over the thumbnail, sized to it explicitly rather than left to
        // fill: a `Stack` takes its size from the first child, and the decoded
        // height and the header-derived one can differ by a pixel.
        let mut layers: Vec<Element<'a, Message>> = vec![inner];
        if let Some(accent) = look.tint {
            let alpha = look.tint_alpha;
            layers.push(
                container(Space::new())
                    .width(Length::Fixed(THUMB_WIDTH as f32))
                    .height(Length::Fixed(height))
                    .style(move |theme: &Theme| container::Style {
                        background: Some(Background::Color(Color {
                            a: alpha,
                            ..accent_color(theme, accent)
                        })),
                        ..container::Style::default()
                    })
                    .into(),
            );
        }
        if look.badge {
            layers.push(corner_badge("\u{2713}", Accent::Select, Horizontal::Right));
        }
        if look.star {
            // Opposite corner from the selection tick, so a favourite that is
            // also selected shows both rather than one hiding the other.
            layers.push(favourite_star());
        }

        let body: Element<'a, Message> = if layers.len() == 1 {
            layers.pop().unwrap()
        } else {
            Stack::with_children(layers).into()
        };

        let ring = look.ring;
        button(body)
            // Inset the content so the ring isn't drawn over it.
            .padding(SEL_BORDER)
            .on_press(Message::ThumbClicked(index))
            .style(move |theme: &Theme, _status| thumb_button_style(theme, ring))
            .into()
    }
}
