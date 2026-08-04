//! The wall view: async thumbnails laid out shortest-column masonry, with a
//! favourites/to-delete visibility filter, keyboard navigation, and
//! click-to-open.
//!
//! Layout runs in two phases. [`WallState::layout`] is a pure function from
//! (viewport width, thumbnail heights, filter) to a [`WallLayout`] — *where*
//! each thumbnail sits, as plain data. The view turns that into widgets;
//! `update` reads it to answer "what is to the left of the current image?".
//! Both derive from the same function over the same state, so keyboard
//! navigation always agrees with what is on screen.
//!
//! The viewport width phase one needs only exists once iced has laid the tree
//! out, so it is read back with a widget [`Operation`] (see [`measure`]) and
//! stored by `update`. The view never writes state.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

use iced::advanced::widget::operation::scrollable::Scrollable;
use iced::advanced::widget::operation::Outcome;
use iced::advanced::widget::{operate, Id, Operation};
use iced::theme::palette::lighten;
use iced::widget::operation::{scroll_to, AbsoluteOffset};
use iced::widget::{button, container, image, scrollable, text, Column, Row, Stack};
use iced::{
    Background, Border, Color, Element, Length, Rectangle, Shadow, Size, Task, Theme, Vector,
};

use super::corner_icon;
use crate::library::Library;
use crate::Message;

/// Thumbnail column width and inter-item spacing (matches the Python wall).
const THUMB_WIDTH: u32 = 300;
/// Inter-tile spacing, and the outer padding of the wall (the layout maths in
/// [`WallState::layout`] assumes the two are equal).
const WALL_SPACING: f32 = 12.0;
/// Width of the selection ring drawn around the current thumbnail.
///
/// `button` fills its border quad at its full bounds and *then* draws the
/// content on top, so a zero-padding button hides its own border behind an
/// opaque child. Every thumbnail therefore carries this much padding — the ring
/// is only made visible (rather than added) when the thumbnail is selected, so
/// selection never reflows the wall.
const SEL_BORDER: f32 = 4.0;
/// A thumbnail plus its selection ring: what the masonry actually places.
const TILE_WIDTH: f32 = THUMB_WIDTH as f32 + 2.0 * SEL_BORDER;
/// How far the selection ring is lifted off the theme's primary colour (OKLCH
/// lightness). On the dark theme this takes `#5865F2` to `#8B8BFF`.
const SEL_LIGHTEN: f32 = 0.25;

/// Identifies the wall's scrollable so [`measure`] and [`WallState::reveal`]
/// can target it.
const WALL_ID: Id = Id::new("wall");

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

/// A keyboard navigation direction, shared with the single view (where left and
/// right mean previous and next).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// Messages produced only while the wall view is on screen. Routed to
/// [`WallState::update`] by `App::update`.
#[derive(Debug, Clone)]
pub(crate) enum WallMsg {
    /// The size of the wall's scroll viewport, read back from the laid-out
    /// widget tree by [`measure`].
    Measured(Size),
    /// Header-derived thumbnail heights for the whole library.
    RatiosLoaded(Vec<(PathBuf, f32)>),
    ThumbDecoded {
        path: PathBuf,
        result: Result<(image::Handle, u32), String>,
    },
    /// The wall scrolled; carries the absolute vertical offset in pixels so the
    /// scheduler can prioritise thumbnails near the viewport.
    Scrolled(f32),
    /// `f`: filter to favourites only (or back to all).
    FilterFavourites,
    /// `d`: filter to to-delete only (or back to all).
    FilterToDelete,
    /// Move the selection one thumbnail in `Dir`.
    Nav(Dir),
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

/// Where the masonry put one thumbnail. `y` is absolute within the scrollable's
/// content, so it can be compared against the scroll offset directly.
struct Slot {
    col: usize,
    row: usize,
    y: f32,
    height: f32,
}

/// The result of one masonry pass: pure data, no widgets.
struct WallLayout {
    /// Per column, the library indices it holds, top to bottom.
    columns: Vec<Vec<usize>>,
    /// Library index -> where it sits. Only displayed images appear.
    slots: HashMap<usize, Slot>,
    /// Displayed library indices in library order — the space decode priority
    /// is expressed in.
    order: Vec<usize>,
    /// Total scrollable content height, including the outer padding.
    content_height: f32,
}

/// Wall-view state: the library plus its thumbnail cache, measurements and
/// visibility filter. Every field is written only by [`WallState::update`].
pub(crate) struct WallState {
    pub(crate) library: Library,
    /// Decoded thumbnails, keyed by path. Decoded once per wall session.
    thumbs: HashMap<PathBuf, ThumbState>,
    /// Header-derived thumbnail heights, known before any decode lands so the
    /// masonry settles once instead of reshuffling under the user.
    ratios: HashMap<PathBuf, f32>,
    /// Paths whose decode has been dispatched but not yet landed. Bounds
    /// concurrency (`len <= max_in_flight`) and stops double-dispatching.
    in_flight: HashSet<PathBuf>,
    /// Size of the scroll viewport. `None` until the first [`measure`] lands —
    /// the wall renders an empty scrollable until then.
    viewport: Option<Size>,
    /// Absolute vertical scroll offset in pixels.
    scroll_y: f32,
    /// Position (in [`WallLayout::order`]) of the thumbnail nearest the middle
    /// of the viewport. Cached rather than recomputed per decode, so filling a
    /// large wall stays linear overall.
    focus: f32,
    /// Sticky vertical centre for runs of left/right moves, so `h` then `l`
    /// returns to where it started instead of drifting.
    desired_y: Option<f32>,
    wall_filter: WallFilter,
}

impl WallState {
    pub(crate) fn new(library: Library) -> Self {
        Self {
            library,
            thumbs: HashMap::new(),
            ratios: HashMap::new(),
            in_flight: HashSet::new(),
            viewport: None,
            scroll_y: 0.0,
            focus: 0.0,
            desired_y: None,
            wall_filter: WallFilter::All,
        }
    }

    /// Everything the wall needs on entry: measure the viewport, read the
    /// thumbnail dimensions, and start decoding.
    pub(crate) fn enter(&mut self) -> Task<Message> {
        Task::batch([measure(), self.load_ratios(), self.schedule()])
    }

    pub(crate) fn update(&mut self, msg: WallMsg) -> Task<Message> {
        match msg {
            WallMsg::Measured(size) => {
                // Never re-measure from here: that would loop.
                if self.viewport == Some(size) {
                    return Task::none();
                }
                let first = self.viewport.is_none();
                self.viewport = Some(size);
                self.refocus();
                // On the first measurement the wall has just appeared, so bring
                // the image carried in from the single view into view.
                let reveal = if first {
                    self.reveal(&self.layout(size.width))
                } else {
                    Task::none()
                };
                Task::batch([reveal, self.schedule()])
            }
            WallMsg::RatiosLoaded(heights) => {
                self.ratios.extend(heights);
                self.refocus();
                self.schedule()
            }
            WallMsg::ThumbDecoded { path, result } => {
                self.in_flight.remove(&path);
                match result {
                    Ok((handle, height)) => {
                        self.thumbs.insert(path, ThumbState { handle, height });
                    }
                    Err(e) => eprintln!("Thumbnail decode error: {e}"),
                }
                // A slot freed: dispatch the next-nearest pending thumbnail(s).
                // Deliberately no `refocus` — with heights known up front the
                // layout barely moves, and this runs once per image.
                self.schedule()
            }
            WallMsg::Scrolled(offset) => {
                self.scroll_y = if offset.is_finite() { offset.max(0.0) } else { 0.0 };
                // Reprioritise: the next freed slots go to the new viewport.
                self.refocus();
                self.schedule()
            }
            WallMsg::FilterFavourites => {
                self.wall_filter = toggle_filter(self.wall_filter, WallFilter::Favourites);
                self.after_filter_change()
            }
            WallMsg::FilterToDelete => {
                self.wall_filter = toggle_filter(self.wall_filter, WallFilter::ToDelete);
                self.after_filter_change()
            }
            WallMsg::Nav(dir) => self.navigate(dir),
        }
    }

    /// Move the selection one thumbnail in `dir`, scroll it into view, and
    /// re-aim the decode scheduler at it. A no-op at the edges of the wall.
    fn navigate(&mut self, dir: Dir) -> Task<Message> {
        let Some(viewport) = self.viewport else {
            return Task::none();
        };
        let layout = self.layout(viewport.width);
        let current = self.library.paths.index();
        let Some(next) = neighbour(&layout, current, dir, self.desired_y) else {
            return Task::none();
        };
        self.library.goto(next);
        // Left/right keeps the vertical centre it started from; up/down resets
        // it, since those moves redefine where "beside" is.
        self.desired_y = match dir {
            Dir::Left | Dir::Right => Some(
                self.desired_y
                    .unwrap_or_else(|| centre_of(&layout, current).unwrap_or(0.0)),
            ),
            Dir::Up | Dir::Down => None,
        };
        let reveal = self.reveal(&layout);
        self.refocus();
        Task::batch([reveal, self.schedule()])
    }

    /// Scroll the selected thumbnail into view, but only if it isn't already —
    /// and only far enough to reach the nearer edge.
    ///
    /// Takes `&mut self` because a programmatic scroll doesn't fire `on_scroll`,
    /// so the new offset has to be recorded here or the next call would decide
    /// from a stale one.
    fn reveal(&mut self, layout: &WallLayout) -> Task<Message> {
        let Some(viewport) = self.viewport else {
            return Task::none();
        };
        let Some(slot) = layout.slots.get(&self.library.paths.index()) else {
            return Task::none();
        };
        // Include the selection ring, so it never sits half off-screen.
        let top = slot.y - SEL_BORDER;
        let bottom = slot.y + slot.height + SEL_BORDER;

        let target = if top < self.scroll_y {
            top - WALL_SPACING
        } else if bottom > self.scroll_y + viewport.height {
            bottom + WALL_SPACING - viewport.height
        } else {
            return Task::none();
        };

        let max = (layout.content_height - viewport.height).max(0.0);
        let y = target.clamp(0.0, max);
        self.scroll_y = y;
        scroll_to(WALL_ID, AbsoluteOffset { x: 0.0, y })
    }

    /// A filter toggle can hide the selected image, which would leave
    /// navigation with no anchor at all. Re-anchor, then re-prioritise.
    fn after_filter_change(&mut self) -> Task<Message> {
        self.ensure_current_displayed();
        self.desired_y = None;
        self.refocus();
        let reveal = match self.viewport {
            Some(viewport) => self.reveal(&self.layout(viewport.width)),
            None => Task::none(),
        };
        Task::batch([reveal, self.schedule()])
    }

    /// Move the pointer to the nearest displayed image if the filter just hid
    /// the one it was on.
    fn ensure_current_displayed(&mut self) {
        let current = self.library.paths.index();
        let nearest = {
            let paths: Vec<&PathBuf> = self.library.paths.iter().collect();
            if paths.get(current).is_some_and(|p| self.is_displayed(p)) {
                return;
            }
            paths
                .iter()
                .enumerate()
                .filter(|(_, p)| self.is_displayed(p))
                .min_by_key(|(i, _)| i.abs_diff(current))
                .map(|(i, _)| i)
        };
        if let Some(index) = nearest {
            self.library.goto(index);
        }
    }

    /// Re-aim decode priority at whatever is now in the middle of the viewport.
    fn refocus(&mut self) {
        let Some(viewport) = self.viewport else {
            self.focus = 0.0;
            return;
        };
        let layout = self.layout(viewport.width);
        self.focus = nearest_position(&layout, self.scroll_y + viewport.height / 2.0);
    }

    /// Read the library's thumbnail dimensions from their headers, off-thread.
    /// One task for the whole library: the cost is per-file IO, and a single
    /// task keeps it from competing with the decode scheduler.
    fn load_ratios(&self) -> Task<Message> {
        let paths: Vec<PathBuf> = self.library.paths.iter().cloned().collect();
        Task::perform(
            crate::imaging::thumb_heights_async(paths, THUMB_WIDTH),
            |heights| Message::Wall(WallMsg::RatiosLoaded(heights)),
        )
    }

    /// Fill free decode slots with the highest-priority pending thumbnails.
    ///
    /// Priority: displayed (unfiltered-out) thumbnails first, ordered by
    /// distance from [`WallState::focus`], so what you're looking at decodes
    /// soonest; then any filtered-out paths in library order, so every
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
                self.focus,
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

    /// The height a thumbnail will occupy: its decoded height, else the
    /// header-derived one, else a square guess until either arrives.
    fn tile_height(&self, path: &PathBuf) -> f32 {
        self.thumbs
            .get(path)
            .map(|t| t.height as f32)
            .or_else(|| self.ratios.get(path).copied())
            .unwrap_or(THUMB_WIDTH as f32)
    }

    /// Place every displayed thumbnail into the shortest column, as data.
    ///
    /// Deterministic in (`width`, `thumbs`, `ratios`, `wall_filter`, library
    /// order) — which is exactly why the view and `update` can call it
    /// separately and still agree.
    fn layout(&self, width: f32) -> WallLayout {
        // `n` tiles need `n * TILE_WIDTH + (n + 1) * WALL_SPACING` (spacing
        // between them, plus the equal outer padding) — hence the single
        // `WALL_SPACING` subtracted before the division.
        let item_width = WALL_SPACING + TILE_WIDTH;
        let col_count = (((width - WALL_SPACING) / item_width).floor() as usize).max(1);

        let mut columns: Vec<Vec<usize>> = vec![Vec::new(); col_count];
        let mut slots = HashMap::new();
        let mut order = Vec::new();
        // Seeded with the container's top padding, so every `y` is absolute
        // within the scrollable's content rather than relative to the grid.
        let mut heights = vec![WALL_SPACING; col_count];

        let displayed = self
            .library
            .paths
            .iter()
            .enumerate()
            .filter(|(_, p)| self.is_displayed(p));

        for (index, path) in displayed {
            let col = heights
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap_or(0);
            let height = self.tile_height(path);
            slots.insert(
                index,
                Slot {
                    col,
                    row: columns[col].len(),
                    y: heights[col],
                    height,
                },
            );
            columns[col].push(index);
            order.push(index);
            heights[col] += height + 2.0 * SEL_BORDER + WALL_SPACING;
        }

        // The tallest column's running total already includes the bottom
        // padding, since each placement adds a trailing `WALL_SPACING`.
        let content_height = heights.into_iter().fold(0.0_f32, f32::max);
        WallLayout {
            columns,
            slots,
            order,
            content_height,
        }
    }

    pub(crate) fn view<'a>(
        &'a self,
        star_icon: &'a image::Handle,
        delete_icon: &'a image::Handle,
    ) -> Element<'a, Message> {
        // The scrollable is rendered even before the first measurement, empty:
        // `measure` has to find it in the tree to report its size at all.
        let grid: Element<'a, Message> = match self.viewport {
            Some(viewport) => {
                self.build_grid(&self.layout(viewport.width), star_icon, delete_icon)
            }
            None => Column::new().into(),
        };

        let centered = container(grid)
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding(WALL_SPACING);
        scrollable(centered)
            .id(WALL_ID)
            .on_scroll(|viewport| Message::Wall(WallMsg::Scrolled(viewport.absolute_offset().y)))
            .height(Length::Fill)
            .into()
    }

    /// Turn a computed [`WallLayout`] into the column-of-thumbnails widget tree.
    fn build_grid<'a>(
        &'a self,
        layout: &WallLayout,
        star_icon: &'a image::Handle,
        delete_icon: &'a image::Handle,
    ) -> Element<'a, Message> {
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
                        Some(self.thumb_element(index, path, current, star_icon, delete_icon))
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
                .height(Length::Fixed(self.tile_height(path)))
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
            // Inset the content so the selection ring isn't drawn over it.
            .padding(SEL_BORDER)
            .on_press(Message::ThumbClicked(index))
            .style(move |theme: &Theme, _status| thumb_button_style(theme, selected))
            .into()
    }
}

/// Read the wall scrollable's viewport size back out of the laid-out widget
/// tree, as a message for `update` to store.
///
/// Yields nothing if the wall isn't on screen — the operation simply finds no
/// scrollable with [`WALL_ID`] and no message is sent.
pub(crate) fn measure() -> Task<Message> {
    operate(measure_wall(WALL_ID)).map(|size| Message::Wall(WallMsg::Measured(size)))
}

fn measure_wall(target: Id) -> impl Operation<Size> {
    struct Measure {
        target: Id,
        found: Option<Size>,
    }

    impl Operation<Size> for Measure {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Size>)) {
            operate(self);
        }

        fn scrollable(
            &mut self,
            id: Option<&Id>,
            bounds: Rectangle,
            _content_bounds: Rectangle,
            _translation: Vector,
            _state: &mut dyn Scrollable,
        ) {
            if Some(&self.target) == id {
                self.found = Some(bounds.size());
            }
        }

        fn finish(&self) -> Outcome<Size> {
            self.found.map_or(Outcome::None, Outcome::Some)
        }
    }

    Measure {
        target,
        found: None,
    }
}

/// The library index one step from `current` in `dir`, or `None` at an edge.
///
/// Up and down are simply the neighbouring row of the same column. Left and
/// right cross to the adjacent column and take whichever thumbnail's vertical
/// centre is nearest — masonry rows are not aligned, so "the same row index" in
/// the next column can be at an entirely different height. `desired_y` overrides
/// the reference centre so a run of sideways moves doesn't drift.
fn neighbour(
    layout: &WallLayout,
    current: usize,
    dir: Dir,
    desired_y: Option<f32>,
) -> Option<usize> {
    let slot = layout.slots.get(&current)?;
    match dir {
        Dir::Up => layout.columns[slot.col]
            .get(slot.row.checked_sub(1)?)
            .copied(),
        Dir::Down => layout.columns[slot.col].get(slot.row + 1).copied(),
        Dir::Left | Dir::Right => {
            let col = if dir == Dir::Left {
                slot.col.checked_sub(1)?
            } else {
                slot.col + 1
            };
            let reference = desired_y.unwrap_or(slot.y + slot.height / 2.0);
            layout.columns.get(col)?.iter().copied().min_by(|a, b| {
                distance_to(layout, *a, reference)
                    .partial_cmp(&distance_to(layout, *b, reference))
                    .unwrap_or(Ordering::Equal)
            })
        }
    }
}

/// Vertical centre of `index`'s slot.
fn centre_of(layout: &WallLayout, index: usize) -> Option<f32> {
    layout.slots.get(&index).map(|s| s.y + s.height / 2.0)
}

fn distance_to(layout: &WallLayout, index: usize, y: f32) -> f32 {
    centre_of(layout, index).map_or(f32::MAX, |c| (c - y).abs())
}

/// Position in [`WallLayout::order`] of the thumbnail whose centre is nearest
/// `y` — decode priority is expressed in that space, not in pixels, so a decode
/// landing can't reshuffle it.
fn nearest_position(layout: &WallLayout, y: f32) -> f32 {
    layout
        .order
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            distance_to(layout, **a, y)
                .partial_cmp(&distance_to(layout, **b, y))
                .unwrap_or(Ordering::Equal)
        })
        .map(|(pos, _)| pos as f32)
        .unwrap_or(0.0)
}

/// Order `paths` for decode and return the first `free`.
///
/// `is_displayed` marks which paths the active filter shows; `pending` marks
/// which still need decoding. Displayed + pending come first, nearest `focus`
/// (a position in the displayed list); then filtered-out + pending in list
/// order, so every thumbnail is still eventually decoded. Priority is by list
/// position, not pixel height, so a decode landing never reshuffles it.
fn prioritise<'a>(
    paths: &[&'a PathBuf],
    is_displayed: impl Fn(&PathBuf) -> bool,
    pending: impl Fn(&PathBuf) -> bool,
    focus: f32,
    free: usize,
) -> Vec<&'a PathBuf> {
    let displayed: Vec<&'a PathBuf> = paths.iter().copied().filter(|p| is_displayed(p)).collect();

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
            .then(a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
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
        // Always the same width — only the colour changes — so selecting a
        // thumbnail never shifts the masonry.
        border: Border {
            color: if selected {
                // `primary.strong` is only a 0.10 lift off the base; take a
                // bigger one so the ring reads clearly against dark thumbnails.
                lighten(palette.primary.base.color, SEL_LIGHTEN)
            } else {
                Color::TRANSPARENT
            },
            width: SEL_BORDER,
            radius: 0.0.into(),
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

    /// A wall of thumbnails `THUMB_WIDTH` wide and `heights[i]` tall, sized to
    /// fit exactly `col_count` columns. Heights are seeded as if the
    /// header-read prepass had already landed.
    fn wall(heights: &[f32], col_count: usize) -> WallState {
        let files = paths(heights.len());
        let library = Library {
            paths: crate::pointed_list::PointedList::new(files.clone()).unwrap(),
            favourites: HashSet::new(),
            to_delete: HashSet::new(),
            image_dir: PathBuf::from("/wall"),
            cache_dir: PathBuf::from("/wall/.cache"),
            favourites_file: PathBuf::from("/wall/.cache/favourites"),
            to_delete_file: PathBuf::from("/wall/.cache/to_delete"),
        };
        let mut state = WallState::new(library);
        state.ratios = files.into_iter().zip(heights.iter().copied()).collect();
        state.viewport = Some(Size::new(width_for(col_count), 1000.0));
        state
    }

    /// The narrowest width that fits exactly `n` columns.
    fn width_for(n: usize) -> f32 {
        n as f32 * (WALL_SPACING + TILE_WIDTH) + WALL_SPACING
    }

    #[test]
    fn masonry_fills_the_shortest_column() {
        // Heights chosen so placement order is 0->c0, 1->c1, 2->c2, 3->c0
        // (shortest at 220), 4->c2, 5->c1.
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        assert_eq!(layout.columns, vec![vec![0, 3], vec![1, 5], vec![2, 4]]);
    }

    #[test]
    fn left_and_right_pick_the_nearest_vertical_centre() {
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        // 3 sits in column 0 at y=232, centre 382. Column 1 holds 1 (centre
        // 212) and 5 (centre 552) — 5 is nearer. List order would have said 4,
        // which is two columns away.
        assert_eq!(neighbour(&layout, 3, Dir::Right, None), Some(5));
        assert_eq!(neighbour(&layout, 5, Dir::Left, None), Some(3));
    }

    #[test]
    fn up_and_down_stay_in_the_column() {
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        assert_eq!(neighbour(&layout, 0, Dir::Down, None), Some(3));
        assert_eq!(neighbour(&layout, 3, Dir::Up, None), Some(0));
    }

    #[test]
    fn navigation_stops_at_the_edges() {
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        assert_eq!(neighbour(&layout, 0, Dir::Up, None), None);
        assert_eq!(neighbour(&layout, 0, Dir::Left, None), None);
        assert_eq!(neighbour(&layout, 3, Dir::Down, None), None);
        assert_eq!(neighbour(&layout, 4, Dir::Right, None), None);
    }

    #[test]
    fn desired_y_overrides_the_current_centre() {
        let state = wall(&[200.0, 400.0, 200.0, 300.0, 200.0, 200.0], 3);
        let layout = state.layout(width_for(3));
        // From 3 (centre 382) the nearest in column 1 is 5, but anchored near
        // the top the answer is 1 instead — this is what stops h/l drifting.
        assert_eq!(neighbour(&layout, 3, Dir::Right, Some(0.0)), Some(1));
    }

    #[test]
    fn single_column_layout_is_the_library_order() {
        let state = wall(&[200.0; 4], 1);
        let layout = state.layout(width_for(1));
        assert_eq!(layout.columns, vec![vec![0, 1, 2, 3]]);
        assert_eq!(neighbour(&layout, 1, Dir::Down, None), Some(2));
        assert_eq!(neighbour(&layout, 1, Dir::Right, None), None);
    }

    #[test]
    fn slot_y_is_absolute_within_the_scroll_content() {
        let state = wall(&[200.0, 200.0], 1);
        let layout = state.layout(width_for(1));
        // First tile starts below the container's top padding, not at 0.
        assert_eq!(layout.slots[&0].y, WALL_SPACING);
        // Second is a tile (200 + 2 * ring) and a gap further down.
        assert_eq!(layout.slots[&1].y, WALL_SPACING + 200.0 + 2.0 * SEL_BORDER + WALL_SPACING);
        // Content height includes the bottom padding.
        assert_eq!(layout.content_height, layout.slots[&1].y + 200.0 + 2.0 * SEL_BORDER + WALL_SPACING);
    }

    /// Heights that lay out as columns `[[0, 3], [1, 5], [2, 4]]` at 3 columns.
    const SPREAD: [f32; 6] = [200.0, 400.0, 200.0, 300.0, 200.0, 200.0];

    #[test]
    fn measuring_unblocks_navigation() {
        let mut state = wall(&SPREAD, 3);
        // Before the first measurement there is no layout, so nav is inert.
        state.viewport = None;
        let _ = state.update(WallMsg::Nav(Dir::Right));
        assert_eq!(state.library.paths.index(), 0);

        let size = Size::new(width_for(3), 1000.0);
        let _ = state.update(WallMsg::Measured(size));
        assert_eq!(state.viewport, Some(size));

        let _ = state.update(WallMsg::Nav(Dir::Right));
        assert_eq!(state.library.paths.index(), 1);
        let _ = state.update(WallMsg::Nav(Dir::Down));
        assert_eq!(state.library.paths.index(), 5);
        let _ = state.update(WallMsg::Nav(Dir::Left));
        assert_eq!(state.library.paths.index(), 3);
    }

    #[test]
    fn sideways_moves_round_trip() {
        let mut state = wall(&SPREAD, 3);
        let _ = state.update(WallMsg::Nav(Dir::Down)); // 0 -> 3
        assert_eq!(state.library.paths.index(), 3);
        // 3's nearest neighbour rightwards is 5, but 5's is 3 only because the
        // sticky centre survives the second press.
        let _ = state.update(WallMsg::Nav(Dir::Right));
        assert_eq!(state.library.paths.index(), 5);
        let _ = state.update(WallMsg::Nav(Dir::Left));
        assert_eq!(state.library.paths.index(), 3);
    }

    #[test]
    fn up_or_down_clears_the_sticky_centre() {
        let mut state = wall(&SPREAD, 3);
        let _ = state.update(WallMsg::Nav(Dir::Down)); // 0 -> 3, centre 382
        let _ = state.update(WallMsg::Nav(Dir::Right)); // -> 5, sticky 382
        let _ = state.update(WallMsg::Nav(Dir::Up)); // -> 1, sticky cleared
        assert_eq!(state.library.paths.index(), 1);
        assert_eq!(state.desired_y, None);
        // Now anchored on 1 (centre 212), not the stale 382, so left gives 0.
        let _ = state.update(WallMsg::Nav(Dir::Left));
        assert_eq!(state.library.paths.index(), 0);
    }

    #[test]
    fn filtering_re_anchors_a_hidden_selection() {
        let mut state = wall(&SPREAD, 3);
        let files: Vec<PathBuf> = state.library.paths.iter().cloned().collect();
        state.library.favourites = [files[4].clone()].into_iter().collect();
        let _ = state.update(WallMsg::Nav(Dir::Down)); // selection on 3
        assert_eq!(state.library.paths.index(), 3);

        // 3 isn't a favourite, so filtering would leave nav with no anchor.
        let _ = state.update(WallMsg::FilterFavourites);
        assert_eq!(state.library.paths.index(), 4);

        // Clearing the filter leaves the pointer where it was re-anchored.
        let _ = state.update(WallMsg::FilterFavourites);
        assert_eq!(state.library.paths.index(), 4);
    }

    #[test]
    fn scrolling_moves_the_decode_focus() {
        // One column, so list order and visual order coincide.
        let mut state = wall(&[200.0; 20], 1);
        assert_eq!(state.focus, 0.0);
        let _ = state.update(WallMsg::Scrolled(2000.0));
        // 2000 + 500 (half the 1000px viewport) lands well down the column.
        assert!(state.focus > 5.0, "focus was {}", state.focus);
    }

    #[test]
    fn prioritises_thumbnails_nearest_the_focus() {
        let paths = paths(10);
        let refs: Vec<&PathBuf> = paths.iter().collect();
        // Focused midway (4.5): 4 and 5 are closest (0.5 each), then 3 (1.5,
        // pushed before 6 so it wins the tie).
        let chosen = prioritise(&refs, |_| true, |_| true, 4.5, 3);
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
