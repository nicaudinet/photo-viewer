//! Where a [`WallMsg`] turns into state and tasks. Every field of
//! [`WallState`] is written from here or from something it calls.

use iced::Task;

use crate::core::tags;
use crate::Message;

use super::queue::BatchKind;
use super::message::WallMsg;
use super::thumbs::ThumbState;
use super::WallState;

impl WallState {
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
                // Rotated mid-decode: these are the old pixels. Dropping them
                // leaves the path un-cached, so `schedule` re-dispatches it.
                if self.stale.remove(&path) {
                    return self.schedule();
                }
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
            WallMsg::Nav(dir) => self.navigate(dir),
            WallMsg::Rotate { clockwise } => self.rotate(clockwise),
            WallMsg::Rotated { path, result } => self.rotated(path, result),
            WallMsg::EnterVisual { op } => self.enter_visual(op),
            WallMsg::CommitVisual => self.commit_visual(),
            // These three edit the committed set, and `settle` would drop the
            // mode back out of `Visual` — so mid-paint they are ignored rather
            // than allowed to end the range behind the user's back. Finish or
            // cancel the range first.
            WallMsg::ToggleCursor if self.is_visual() => Task::none(),
            WallMsg::SelectAll if self.is_visual() => Task::none(),
            WallMsg::InvertSelection if self.is_visual() => Task::none(),
            WallMsg::ToggleCursor => {
                self.library.toggle_selected(self.library.paths.index());
                self.settle()
            }
            WallMsg::SelectAll => {
                self.library.select_all();
                self.settle()
            }
            WallMsg::InvertSelection => {
                self.library.invert_selection();
                self.settle()
            }
            WallMsg::ToggleFavourite => self.favourite(),
            WallMsg::ToggleFilter => self.toggle_filter(),
            WallMsg::Escape => self.escape(),
            WallMsg::StartBatch { kind, paths } => self.start_batch(kind, paths),
            WallMsg::BatchProgress { path, result } => self.batch_progress(path, result),
        }
    }

    /// Rotate 90° on disk: the whole selection in `Select`, otherwise just the
    /// image under the cursor.
    ///
    /// Ignored while painting a range, for the same reason `Space` is: the
    /// range has no committed meaning yet, so there is no honest answer to
    /// "which images?". Finish or cancel it first. Also ignored while a batch
    /// is already running, so two batches can't fight over the same files.
    pub(super) fn rotate(&mut self, clockwise: bool) -> Task<Message> {
        if self.is_visual() || self.batch.is_some() {
            return Task::none();
        }
        if let Some(selected) = self.operable_selection() {
            return self.start_batch(BatchKind::Rotate { clockwise }, selected);
        }
        self.rotate_one(clockwise)
    }

    /// Favourite the whole selection, or — with nothing selected — the image
    /// under the cursor.
    ///
    /// The same shape as `r`: an operation over the selection where there is
    /// one, over the cursor where there is not. Unlike `r` it needs no batch —
    /// a tag is a line in a small text file, not a write per photo.
    ///
    /// Ignored while painting, for the reason everything is: an uncommitted
    /// range has no settled meaning.
    pub(super) fn favourite(&mut self) -> Task<Message> {
        if self.is_visual() {
            return Task::none();
        }
        let paths = match self.operable_selection() {
            Some(selected) => selected,
            None => vec![self.library.current().clone()],
        };
        self.library.toggle_tag(tags::FAVOURITE, &paths);
        // Only bites while the favourites are what is on screen, where
        // un-favouriting takes photos off the wall as it goes.
        self.library.refilter();
        self.relaid()
    }

    /// Show only the favourites, or show everything again.
    ///
    /// Refused while anything is selected. A filter that hid a selected photo
    /// would put images the user cannot see into the next batch; forbidding the
    /// combination removes that whole class of bug rather than papering over it
    /// (see `SELECT_MODE_PLAN.md`). Clearing the selection with Esc is one key.
    pub(super) fn toggle_filter(&mut self) -> Task<Message> {
        if self.is_visual() || self.batch.is_some() || !self.library.selection.is_empty() {
            return Task::none();
        }
        let tag = match self.library.filter {
            Some(_) => None,
            None => Some(tags::FAVOURITE.to_string()),
        };
        if !self.library.set_filter(tag) {
            // Nothing carries the tag: the wall would go blank and claim the
            // folder was empty.
            return Task::none();
        }
        // Only the view changed, so there is nothing to write to disk.
        Task::batch([self.resettled(), self.remeasure()])
    }

    /// Everything the wall has to redo after the visible list changed, plus the
    /// write to disk that a tag change earns.
    pub(super) fn relaid(&mut self) -> Task<Message> {
        let save = Task::perform(
            tags::save_async(self.library.image_dir.clone(), self.library.tags.clone()),
            Message::TagsSaved,
        );
        Task::batch([self.resettled(), save, self.settle()])
    }

    /// Re-aim the wall at wherever the cursor ended up. The tiles themselves
    /// have not changed shape, so no thumbnail is thrown away — the masonry
    /// simply has fewer or more of them to place.
    pub(super) fn resettled(&mut self) -> Task<Message> {
        self.desired_y = None;
        self.refocus();
        let reveal = match self.viewport {
            Some(viewport) => self.reveal(&self.layout(viewport.width)),
            None => Task::none(),
        };
        Task::batch([reveal, self.schedule()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::wall::fixture::*;
    use crate::screens::wall::select::WallMode;
    use crate::core::library::RangeOp;
    use crate::screens::wall::message::Dir;

    #[test]
    fn f_in_normal_favourites_only_the_cursor_image() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(3);
        fav(&mut state);
        assert_eq!(starred(&state), vec![3]);
        // And again takes it back off.
        fav(&mut state);
        assert!(starred(&state).is_empty());
    }

    #[test]
    fn f_in_select_favourites_the_whole_selection() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);
        let _ = state.update(WallMsg::CommitVisual);

        fav(&mut state);
        assert_eq!(starred(&state), vec![0, 1]);
        // The selection is untouched, so it can be acted on again.
        assert_eq!(selected(&state), vec![0, 1]);
    }

    #[test]
    fn f_is_ignored_while_painting() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        nav(&mut state, Dir::Down);
        fav(&mut state);
        // An uncommitted range has no settled meaning, so there is no honest
        // answer to "which images?".
        assert!(starred(&state).is_empty());
    }

    #[test]
    fn the_filter_narrows_the_wall_to_the_favourites() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(4);
        fav(&mut state);
        state.library.goto(1);
        fav(&mut state);

        filter(&mut state);
        assert_eq!(state.library.paths.len(), 2);
        // The cursor was on a favourite, so it stays on that photo.
        assert_eq!(state.library.current(), &state.library.all[1]);

        filter(&mut state);
        assert_eq!(state.library.paths.len(), 6);
    }

    #[test]
    fn the_filter_is_refused_while_anything_is_selected() {
        let mut state = wall(&[200.0; 6], 1);
        fav(&mut state);
        let _ = state.update(WallMsg::SelectAll);

        filter(&mut state);
        // A filter that hid a selected photo would put images the user cannot
        // see into the next batch. Esc clears the selection in one key.
        assert_eq!(state.library.filter, None);
        assert_eq!(state.library.paths.len(), 6);
    }

    #[test]
    fn the_filter_is_refused_when_nothing_is_favourited() {
        let mut state = wall(&[200.0; 6], 1);
        filter(&mut state);
        // A blank wall would claim the folder was empty.
        assert_eq!(state.library.filter, None);
        assert_eq!(state.library.paths.len(), 6);
    }

    #[test]
    fn unfavouriting_the_last_one_puts_the_whole_folder_back() {
        let mut state = wall(&[200.0; 6], 1);
        fav(&mut state);
        filter(&mut state);
        assert_eq!(state.library.paths.len(), 1);

        fav(&mut state);
        assert_eq!(state.library.filter, None);
        assert_eq!(state.library.paths.len(), 6);
    }

    #[test]
    fn unfavouriting_a_selected_photo_while_filtered_deselects_it() {
        let mut state = wall(&[200.0; 6], 1);
        let _ = state.update(WallMsg::SelectAll);
        fav(&mut state);
        let _ = state.update(WallMsg::Escape); // clear, so the filter is allowed
        filter(&mut state);

        state.library.goto(2);
        let _ = state.update(WallMsg::ToggleCursor);
        fav(&mut state);

        // It left the wall, so it must leave the selection with it — otherwise
        // the next batch would touch a photo nobody can see.
        assert_eq!(state.library.paths.len(), 5);
        assert!(state.library.selection.is_empty());
        assert_eq!(state.mode, WallMode::Normal);
    }

    #[test]
    fn the_star_is_hidden_while_the_wall_is_already_all_favourites() {
        let mut state = wall(&[200.0; 6], 1);
        let path = state.library.current().clone();
        fav(&mut state);
        assert!(state.tile_look(0, &path, 0).star);

        filter(&mut state);
        // Every tile would carry one, which says nothing.
        assert!(!state.tile_look(0, &path, 0).star);
    }

    #[test]
    fn a_favourite_that_is_also_selected_shows_both_marks() {
        let mut state = wall(&[200.0; 6], 1);
        let path = state.library.current().clone();
        fav(&mut state);
        let _ = state.update(WallMsg::ToggleCursor);

        let look = state.tile_look(0, &path, 0);
        assert!(look.star);
        assert!(look.badge);
    }
}
