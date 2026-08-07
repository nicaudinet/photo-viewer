//! Rotating one image, and what the wall has to forget afterwards.
//!
//! A rotate is the only thing that changes a tile's shape while the wall is up,
//! so it is also the only thing that can invalidate a decoded thumbnail.

use std::path::PathBuf;

use iced::Task;

use crate::Message;

use super::ops::rotate_async;
use super::layout::THUMB_WIDTH;
use super::message::WallMsg;
use super::WallState;

impl WallState {
    /// Rotate the image under the cursor 90° on disk, off-thread. Ignored while
    /// a rotate of the same file is already in flight.
    pub(super) fn rotate_one(&mut self, clockwise: bool) -> Task<Message> {
        let Some(path) = self.claim_rotate() else {
            return Task::none();
        };
        let key = path.clone();
        Task::perform(rotate_async(path, clockwise), move |result| {
            Message::Wall(WallMsg::Rotated {
                path: key.clone(),
                result,
            })
        })
    }

    /// Claim the selected path for a rotate, or `None` if one is already
    /// writing it — two concurrent read-modify-writes of the same file would
    /// race, and one 90° turn would be lost.
    pub(super) fn claim_rotate(&mut self) -> Option<PathBuf> {
        let path = self.library.current().clone();
        self.rotating.insert(path.clone()).then_some(path)
    }

    /// The file changed shape on disk: invalidate its thumbnail, re-read its
    /// dimensions, and let the masonry settle around the new one.
    ///
    /// Only slots at or after the rotated image can move — the shortest-column
    /// pass places index `i` from the column heights left by indices before it,
    /// which this doesn't touch. So nothing above needs re-decoding, and the
    /// thumbnails below simply get placed at new offsets with the handles they
    /// already have.
    pub(super) fn rotated(&mut self, path: PathBuf, result: Result<(), String>) -> Task<Message> {
        self.rotating.remove(&path);
        if let Err(e) = result {
            eprintln!("Rotate failed: {e}");
            return Task::none();
        }

        let invalidate = self.invalidate_rotated(path);
        // Rotating changes the tile's height, so a sticky centre taken before
        // it no longer describes where the selection sits.
        self.desired_y = None;
        self.refocus();

        let reveal = match self.viewport {
            Some(viewport) => self.reveal(&self.layout(viewport.width)),
            None => Task::none(),
        };
        Task::batch([invalidate, reveal, self.schedule()])
    }

    /// Drop what the wall knows about a file that has just changed shape on
    /// disk, and start re-reading its dimensions.
    ///
    /// Split out from [`WallState::rotated`] because a batch runs this once per
    /// image and must *not* scroll or re-aim the view while doing so — see
    /// [`WallState::batch_progress`].
    pub(super) fn invalidate_rotated(&mut self, path: PathBuf) -> Task<Message> {
        self.thumbs.remove(&path);
        // An in-flight decode of this path predates the write; discard it when
        // it lands (see `WallMsg::ThumbDecoded`).
        if self.in_flight.contains(&path) {
            self.stale.insert(path.clone());
        }
        // Provisional height so the placeholder is the right shape immediately:
        // a 90° turn swaps the aspect ratio, so `h' = WIDTH^2 / h`. The header
        // read below replaces it with the exact value a moment later — that one
        // has to match `imaging::thumbnail` bit for bit or the wall shifts when
        // the decode lands.
        if let Some(height) = self.ratios.get(&path).copied() {
            let width = THUMB_WIDTH as f32;
            self.ratios
                .insert(path.clone(), (width * width / height).round().max(1.0));
        }
        Task::perform(
            crate::core::imaging::thumb_heights_async(vec![path], THUMB_WIDTH),
            |heights| Message::Wall(WallMsg::RatiosLoaded(heights)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::wall::fixture::*;

    #[test]
    fn rotating_swaps_the_aspect_and_drops_the_thumbnail() {
        let mut state = wall(&SPREAD, 3);
        let path = state.library.current().clone();
        state.thumbs.insert(path.clone(), fake_thumb(200));

        let _ = state.update(WallMsg::Rotated {
            path: path.clone(),
            result: Ok(()),
        });

        // The cached decode is pre-rotation pixels, so it must go.
        assert!(!state.thumbs.contains_key(&path));
        // 300 wide over a 200-tall thumb becomes 300 * 300/200 = 450 tall.
        assert_eq!(state.ratios.get(&path), Some(&450.0));
        assert!(state.rotating.is_empty());
    }

    #[test]
    fn a_failed_rotate_leaves_the_thumbnail_alone() {
        let mut state = wall(&SPREAD, 3);
        let path = state.library.current().clone();
        state.thumbs.insert(path.clone(), fake_thumb(200));

        let _ = state.update(WallMsg::Rotated {
            path: path.clone(),
            result: Err("nope".into()),
        });

        assert!(state.thumbs.contains_key(&path));
        assert_eq!(state.ratios.get(&path), Some(&200.0));
        assert!(state.rotating.is_empty());
    }

    #[test]
    fn rotation_only_moves_slots_at_or_after_it() {
        let mut state = wall(&SPREAD, 3);
        let before = placements(&state.layout(width_for(3)));

        // Rotate index 2 — a 200-tall tile, so the swap to 450 actually
        // changes something (a 300-tall one is square and would not).
        let path = state.library.paths.iter().nth(2).unwrap().clone();
        state.library.goto(2);
        let _ = state.update(WallMsg::Rotated {
            path,
            result: Ok(()),
        });
        let after = placements(&state.layout(width_for(3)));

        // Everything the masonry placed before it is untouched: index `i` is
        // positioned from the column heights left by indices < `i`.
        assert_eq!(before[..2], after[..2]);
        // And it did actually reflow below, or this would prove nothing.
        assert_ne!(before[2..], after[2..]);
    }

    #[test]
    fn a_decode_landing_after_a_rotate_is_discarded() {
        let mut state = wall(&SPREAD, 3);
        let path = state.library.current().clone();
        state.in_flight.insert(path.clone());

        let _ = state.update(WallMsg::Rotated {
            path: path.clone(),
            result: Ok(()),
        });
        assert!(state.stale.contains(&path));

        // That decode began before the write, so its pixels are the old ones.
        let _ = state.update(WallMsg::ThumbDecoded {
            path: path.clone(),
            result: Ok((fake_thumb(200).handle, 200)),
        });
        assert!(!state.thumbs.contains_key(&path));
        assert!(state.stale.is_empty());
        // Left un-cached, so the same `schedule` that frees the slot puts it
        // straight back in flight — now reading the rotated file.
        assert!(state.in_flight.contains(&path));
    }

    #[test]
    fn a_decode_landing_without_a_rotate_is_cached() {
        let mut state = wall(&SPREAD, 3);
        let path = state.library.current().clone();
        state.in_flight.insert(path.clone());

        let _ = state.update(WallMsg::ThumbDecoded {
            path: path.clone(),
            result: Ok((fake_thumb(200).handle, 200)),
        });
        assert!(state.thumbs.contains_key(&path));
    }

    #[test]
    fn a_second_rotate_is_ignored_while_one_is_writing() {
        let mut state = wall(&SPREAD, 3);
        let path = state.library.current().clone();

        assert_eq!(state.claim_rotate(), Some(path.clone()));
        // Holding `r` down must not race two writes against the same file.
        assert_eq!(state.claim_rotate(), None);

        // Once the write lands the claim is released.
        let _ = state.update(WallMsg::Rotated {
            path: path.clone(),
            result: Ok(()),
        });
        assert_eq!(state.claim_rotate(), Some(path));
    }
}
