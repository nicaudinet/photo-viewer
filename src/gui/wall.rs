//! The wall view: async thumbnails laid out shortest-column masonry, with a
//! favourites/to-delete visibility filter and click-to-open.

use std::collections::HashMap;
use std::path::PathBuf;

use iced::widget::{button, container, image, responsive, scrollable, text, Column, Row, Stack};
use iced::{Background, Border, Element, Length, Shadow, Size, Task, Theme};

use super::corner_icon;
use crate::library::Library;
use crate::Message;

/// Thumbnail column width and inter-item spacing (matches the Python wall).
const THUMB_WIDTH: u32 = 300;
const WALL_SPACING: f32 = 20.0;

/// Messages produced only while the wall view is on screen. Routed to
/// [`WallState::update`] by `App::update`.
#[derive(Debug, Clone)]
pub(crate) enum WallMsg {
    ThumbDecoded {
        path: PathBuf,
        result: Result<(image::Handle, u32), String>,
    },
    /// `f`: filter to favourites only (or back to all).
    FilterFavourites,
    /// `d`: filter to to-delete only (or back to all).
    FilterToDelete,
}

/// Wall-view visibility filter. Toggling one filter off returns to `All`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WallFilter {
    All,
    Favourites,
    ToDelete,
}

/// A decoded thumbnail: the RGBA handle plus its scaled pixel size (needed for
/// masonry column-height bookkeeping).
struct ThumbState {
    handle: image::Handle,
    height: u32,
}

/// Wall-view state: the library plus its thumbnail cache and visibility filter.
pub(crate) struct WallState {
    pub(crate) library: Library,
    /// Decoded thumbnails, keyed by path. Decoded once per wall session.
    thumbs: HashMap<PathBuf, ThumbState>,
    wall_filter: WallFilter,
}

impl WallState {
    pub(crate) fn new(library: Library) -> Self {
        Self {
            library,
            thumbs: HashMap::new(),
            wall_filter: WallFilter::All,
        }
    }

    pub(crate) fn update(&mut self, msg: WallMsg) -> Task<Message> {
        match msg {
            WallMsg::ThumbDecoded { path, result } => {
                match result {
                    Ok((handle, height)) => {
                        self.thumbs.insert(path, ThumbState { handle, height });
                    }
                    Err(e) => eprintln!("Thumbnail decode error: {e}"),
                }
                Task::none()
            }
            WallMsg::FilterFavourites => {
                self.wall_filter = toggle_filter(self.wall_filter, WallFilter::Favourites);
                Task::none()
            }
            WallMsg::FilterToDelete => {
                self.wall_filter = toggle_filter(self.wall_filter, WallFilter::ToDelete);
                Task::none()
            }
        }
    }

    /// Decode (once) every thumbnail not already cached.
    pub(crate) fn decode_thumbs(&self) -> Task<Message> {
        let tasks: Vec<Task<Message>> = self
            .library
            .paths
            .iter()
            .filter(|p| !self.thumbs.contains_key(*p))
            .map(|p| {
                let path = p.clone();
                let key = p.clone();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            crate::imaging::thumbnail(&path, THUMB_WIDTH)
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    move |result| {
                        Message::Wall(WallMsg::ThumbDecoded {
                            path: key.clone(),
                            result,
                        })
                    },
                )
            })
            .collect();
        Task::batch(tasks)
    }

    pub(crate) fn view<'a>(
        &'a self,
        star_icon: &'a image::Handle,
        delete_icon: &'a image::Handle,
    ) -> Element<'a, Message> {
        responsive(move |size| self.build_wall(size, star_icon, delete_icon)).into()
    }

    /// Lay the (filtered) thumbnails out shortest-column masonry for `size`.
    fn build_wall<'a>(
        &'a self,
        size: Size,
        star_icon: &'a image::Handle,
        delete_icon: &'a image::Handle,
    ) -> Element<'a, Message> {
        let current = self.library.paths.index();

        let items: Vec<(usize, &PathBuf)> = self
            .library
            .paths
            .iter()
            .enumerate()
            .filter(|(_, p)| match self.wall_filter {
                WallFilter::All => true,
                WallFilter::Favourites => self.library.favourites.contains(*p),
                WallFilter::ToDelete => self.library.to_delete.contains(*p),
            })
            .collect();

        let item_width = WALL_SPACING + THUMB_WIDTH as f32;
        let col_count = (((size.width - WALL_SPACING) / item_width).floor() as usize).max(1);

        let mut buckets: Vec<Vec<Element<'a, Message>>> =
            (0..col_count).map(|_| Vec::new()).collect();
        let mut heights = vec![0.0_f32; col_count];

        for (index, path) in items {
            // Shortest-column placement (matches the Python masonry).
            let col = heights
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .map(|(i, _)| i)
                .unwrap_or(0);
            let thumb_height = self.thumbs.get(path).map(|t| t.height as f32).unwrap_or(THUMB_WIDTH as f32);
            buckets[col].push(self.thumb_element(index, path, current, star_icon, delete_icon));
            heights[col] += thumb_height + WALL_SPACING;
        }

        let columns: Vec<Element<'a, Message>> = buckets
            .into_iter()
            .map(|items| {
                Column::with_children(items)
                    .spacing(WALL_SPACING)
                    .width(Length::Fixed(THUMB_WIDTH as f32))
                    .into()
            })
            .collect();

        let grid = Row::with_children(columns).spacing(WALL_SPACING);
        let centered = container(grid)
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding(WALL_SPACING);
        scrollable(centered).height(Length::Fill).into()
    }

    /// One thumbnail: image (or placeholder) + icon overlay, clickable, with a
    /// highlight border when it is the current image. Icons are hidden while a
    /// filter is active (the filter already conveys the status).
    fn thumb_element<'a>(
        &'a self,
        index: usize,
        path: &'a PathBuf,
        current: usize,
        star_icon: &'a image::Handle,
        delete_icon: &'a image::Handle,
    ) -> Element<'a, Message> {
        let inner: Element<'a, Message> = match self.thumbs.get(path) {
            Some(thumb) => image(thumb.handle.clone())
                .width(Length::Fixed(THUMB_WIDTH as f32))
                .height(Length::Fixed(thumb.height as f32))
                .into(),
            None => container(text("Loading\u{2026}").size(14))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .width(Length::Fixed(THUMB_WIDTH as f32))
                .height(Length::Fixed(THUMB_WIDTH as f32))
                .style(placeholder_style)
                .into(),
        };

        let show_icons = self.wall_filter == WallFilter::All;
        let icon = if show_icons && self.library.favourites.contains(path) {
            Some(star_icon.clone())
        } else if show_icons && self.library.to_delete.contains(path) {
            Some(delete_icon.clone())
        } else {
            None
        };

        let body: Element<'a, Message> = match icon {
            Some(handle) => Stack::with_children(vec![inner, corner_icon(handle)]).into(),
            None => inner,
        };

        let selected = index == current;
        button(body)
            .padding(0)
            .on_press(Message::ThumbClicked(index))
            .style(move |theme: &Theme, _status| thumb_button_style(theme, selected))
            .into()
    }
}

/// Toggle `filter` on: pressing its key again (when already active) returns to
/// `All`, otherwise it becomes the sole active filter.
fn toggle_filter(current: WallFilter, filter: WallFilter) -> WallFilter {
    if current == filter {
        WallFilter::All
    } else {
        filter
    }
}

fn thumb_button_style(theme: &Theme, selected: bool) -> button::Style {
    let palette = theme.extended_palette();
    button::Style {
        background: None,
        text_color: palette.background.base.text,
        border: if selected {
            Border {
                color: palette.primary.strong.color,
                width: 4.0,
                radius: 0.0.into(),
            }
        } else {
            Border::default()
        },
        shadow: Shadow::default(),
        // 0.14 added pixel-grid snapping; keep the non-crisp default.
        snap: false,
    }
}

fn placeholder_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(palette.background.weak.color)),
        ..container::Style::default()
    }
}
