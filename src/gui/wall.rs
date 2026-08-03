//! The wall view: async thumbnails laid out shortest-column masonry, with a
//! favourites/to-delete visibility filter and click-to-open.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

use iced::widget::{button, container, image, responsive, scrollable, text, Column, Row, Stack};
use iced::{Background, Border, Element, Length, Shadow, Size, Task, Theme};

use super::corner_icon;
use crate::library::Library;
use crate::Message;

/// Thumbnail column width and inter-item spacing (matches the Python wall).
const THUMB_WIDTH: u32 = 300;
const WALL_SPACING: f32 = 20.0;

/// Max thumbnail decodes running at once. A wall of N images no longer fires N
/// tasks up front; the scheduler keeps at most this many in flight and refills
/// as each lands (see [`WallState::schedule`]). Sized to the CPU count, since
/// decode is CPU-bound; bounding it caps thrash and peak memory.
fn max_in_flight() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    })
}

/// Messages produced only while the wall view is on screen. Routed to
/// [`WallState::update`] by `App::update`.
#[derive(Debug, Clone)]
pub(crate) enum WallMsg {
    ThumbDecoded {
        path: PathBuf,
        result: Result<(image::Handle, u32), String>,
    },
    /// The wall scrolled; carries the vertical scroll fraction (0 = top, 1 =
    /// bottom) so the scheduler can prioritise thumbnails near the viewport.
    Scrolled(f32),
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
    /// Paths whose decode has been dispatched but not yet landed. Bounds
    /// concurrency (`len <= max_in_flight`) and stops double-dispatching.
    in_flight: HashSet<PathBuf>,
    /// Latest vertical scroll fraction (0 = top, 1 = bottom); steers which
    /// pending thumbnails the scheduler decodes next.
    scroll_fraction: f32,
    wall_filter: WallFilter,
}

impl WallState {
    pub(crate) fn new(library: Library) -> Self {
        Self {
            library,
            thumbs: HashMap::new(),
            in_flight: HashSet::new(),
            scroll_fraction: 0.0,
            wall_filter: WallFilter::All,
        }
    }

    pub(crate) fn update(&mut self, msg: WallMsg) -> Task<Message> {
        match msg {
            WallMsg::ThumbDecoded { path, result } => {
                self.in_flight.remove(&path);
                match result {
                    Ok((handle, height)) => {
                        self.thumbs.insert(path, ThumbState { handle, height });
                    }
                    Err(e) => eprintln!("Thumbnail decode error: {e}"),
                }
                // A slot freed: dispatch the next-nearest pending thumbnail(s).
                self.schedule()
            }
            WallMsg::Scrolled(fraction) => {
                self.scroll_fraction = if fraction.is_finite() {
                    fraction.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                // Reprioritise: the next freed slots go to the new viewport.
                self.schedule()
            }
            WallMsg::FilterFavourites => {
                self.wall_filter = toggle_filter(self.wall_filter, WallFilter::Favourites);
                self.schedule()
            }
            WallMsg::FilterToDelete => {
                self.wall_filter = toggle_filter(self.wall_filter, WallFilter::ToDelete);
                self.schedule()
            }
        }
    }

    /// Fill free decode slots with the highest-priority pending thumbnails.
    ///
    /// Priority: displayed (unfiltered-out) thumbnails first, ordered by
    /// distance from the current scroll viewport, so what you're looking at
    /// decodes soonest; then any filtered-out paths in library order, so every
    /// thumbnail is still eventually decoded (ready if the filter is cleared).
    /// Called on wall entry and whenever a slot frees or priorities shift.
    pub(crate) fn schedule(&mut self) -> Task<Message> {
        let free = max_in_flight().saturating_sub(self.in_flight.len());
        if free == 0 {
            return Task::none();
        }

        // Choose the paths first (borrows self immutably), then dispatch and
        // record them (borrows self mutably) — the scope ends the read borrow.
        let chosen: Vec<PathBuf> = {
            let paths: Vec<&PathBuf> = self.library.paths.iter().collect();
            prioritise(
                &paths,
                |p| self.is_displayed(p),
                |p| self.needs_decode(p),
                self.scroll_fraction,
                free,
            )
            .into_iter()
            .cloned()
            .collect()
        };

        let tasks: Vec<Task<Message>> = chosen
            .into_iter()
            .map(|path| {
                self.in_flight.insert(path.clone());
                let key = path.clone();
                Task::perform(
                    crate::imaging::thumbnail_async(path, THUMB_WIDTH),
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

    /// A path still needs decoding: not cached and not already dispatched.
    fn needs_decode(&self, path: &PathBuf) -> bool {
        !self.thumbs.contains_key(path) && !self.in_flight.contains(path)
    }

    /// Whether `path` is shown under the active filter.
    fn is_displayed(&self, path: &PathBuf) -> bool {
        match self.wall_filter {
            WallFilter::All => true,
            WallFilter::Favourites => self.library.favourites.contains(path),
            WallFilter::ToDelete => self.library.to_delete.contains(path),
        }
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
            .filter(|(_, p)| self.is_displayed(p))
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
        scrollable(centered)
            .on_scroll(|viewport| Message::Wall(WallMsg::Scrolled(viewport.relative_offset().y)))
            .height(Length::Fill)
            .into()
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

/// Order `paths` for decode and return the first `free`.
///
/// `is_displayed` marks which paths the active filter shows; `pending` marks
/// which still need decoding. Displayed + pending come first, nearest the
/// `scroll_fraction` focus (0 = top, 1 = bottom); then filtered-out + pending
/// in list order, so every thumbnail is still eventually decoded. Priority is
/// by list position, not pixel height, so a decode landing never reshuffles it.
fn prioritise<'a>(
    paths: &[&'a PathBuf],
    is_displayed: impl Fn(&PathBuf) -> bool,
    pending: impl Fn(&PathBuf) -> bool,
    scroll_fraction: f32,
    free: usize,
) -> Vec<&'a PathBuf> {
    let displayed: Vec<&'a PathBuf> = paths.iter().copied().filter(|p| is_displayed(p)).collect();
    let focus = scroll_fraction * displayed.len().saturating_sub(1) as f32;

    // (tier, key): tier 0 = displayed (key = distance from focus), tier 1 =
    // filtered-out (key = list index). Lower sorts first; sort is stable, so
    // equal-distance items keep list order.
    let mut candidates: Vec<(u8, f32, &'a PathBuf)> = Vec::new();
    for (pos, p) in displayed.iter().copied().enumerate() {
        if pending(p) {
            candidates.push((0, (pos as f32 - focus).abs(), p));
        }
    }
    for (i, p) in paths.iter().copied().enumerate() {
        if !is_displayed(p) && pending(p) {
            candidates.push((1, i as f32, p));
        }
    }
    candidates.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    candidates.into_iter().take(free).map(|(_, _, p)| p).collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn paths(n: usize) -> Vec<PathBuf> {
        (0..n).map(|i| PathBuf::from(format!("{i}.jpg"))).collect()
    }

    #[test]
    fn prioritises_thumbnails_nearest_the_viewport() {
        let paths = paths(10);
        let refs: Vec<&PathBuf> = paths.iter().collect();
        // Scrolled to the middle (focus = 0.5 * 9 = 4.5): 4 and 5 are closest
        // (0.5 each), then 3 (1.5, pushed before 6 so it wins the tie).
        let chosen = prioritise(&refs, |_| true, |_| true, 0.5, 3);
        assert_eq!(chosen, vec![&paths[4], &paths[5], &paths[3]]);
    }

    #[test]
    fn decodes_top_down_from_the_top() {
        let paths = paths(6);
        let refs: Vec<&PathBuf> = paths.iter().collect();
        let chosen = prioritise(&refs, |_| true, |_| true, 0.0, 3);
        assert_eq!(chosen, vec![&paths[0], &paths[1], &paths[2]]);
    }

    #[test]
    fn skips_already_decoded_or_in_flight() {
        let paths = paths(6);
        let refs: Vec<&PathBuf> = paths.iter().collect();
        // 0 already done: from the top, the next two pending are 1 and 2.
        let chosen = prioritise(&refs, |_| true, |p| p != &paths[0], 0.0, 2);
        assert_eq!(chosen, vec![&paths[1], &paths[2]]);
    }

    #[test]
    fn displayed_first_then_filtered_out() {
        let paths = paths(6);
        let refs: Vec<&PathBuf> = paths.iter().collect();
        let shown: HashSet<PathBuf> = [0, 2, 4].iter().map(|i| paths[*i].clone()).collect();
        // Displayed evens come first (by distance from the top), then the
        // filtered-out odds in list order — everything still gets decoded.
        let chosen = prioritise(&refs, |p| shown.contains(p), |_| true, 0.0, 6);
        assert_eq!(
            chosen,
            vec![
                &paths[0], &paths[2], &paths[4], &paths[1], &paths[3], &paths[5]
            ]
        );
    }

    #[test]
    fn free_zero_yields_nothing() {
        let paths = paths(4);
        let refs: Vec<&PathBuf> = paths.iter().collect();
        let chosen = prioritise(&refs, |_| true, |_| true, 0.0, 0);
        assert!(chosen.is_empty());
    }
}
