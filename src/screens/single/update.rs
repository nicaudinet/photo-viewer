//! Where a [`SingleMsg`] turns into state and tasks.

use std::path::PathBuf;

use iced::Task;

use crate::core::{imaging, tags};
use crate::Message;

use super::message::SingleMsg;
use super::SingleState;

impl SingleState {
    pub(crate) fn update(&mut self, msg: SingleMsg, generation: &mut u64) -> Task<Message> {
        match msg {
            SingleMsg::Next => {
                self.library.next();
                self.decode_current(generation)
            }
            SingleMsg::Prev => {
                self.library.prev();
                self.decode_current(generation)
            }
            SingleMsg::RotateAnticlockwise => self.rotate(false),
            SingleMsg::RotateClockwise => self.rotate(true),
            SingleMsg::ToggleFavourite => {
                let path = self.library.current().clone();
                self.library.toggle_tag(tags::FAVOURITE, &[path]);
                // No `refilter` here: the wall owns which images are on show,
                // and it re-derives that on entry. Dropping the current image
                // out from under the single view would be the one thing this
                // screen must never do.
                Task::perform(
                    tags::save_async(
                        self.library.image_dir.clone(),
                        self.library.tags.clone(),
                    ),
                    |result| Message::Single(SingleMsg::TagsSaved(result)),
                )
            }
            SingleMsg::TagsSaved(Ok(())) => Task::none(),
            SingleMsg::TagsSaved(Err(e)) => {
                eprintln!("Could not save tags: {e}");
                Task::none()
            }
            SingleMsg::Rotated { path, result } => {
                self.rotating.remove(&path);
                match result {
                    // The file changed on disk: re-decode it, unless the view
                    // has since moved to a different image. (Wall thumbnails
                    // are rebuilt on next entry, so none is stale here.)
                    Ok(()) if self.library.current() == &path => {
                        self.decode_current(generation)
                    }
                    Ok(()) => Task::none(),
                    Err(e) => {
                        eprintln!("Rotate failed: {e}");
                        Task::none()
                    }
                }
            }
            SingleMsg::LargeDecoded {
                generation: tagged,
                result,
            } => {
                // Generation gate: drop a decode superseded by a newer request.
                if tagged == *generation {
                    match result {
                        Ok(handle) => self.large = Some(handle),
                        Err(e) => eprintln!("Decode error: {e}"),
                    }
                }
                Task::none()
            }
        }
    }


    /// Rotate the current image 90° (clockwise if `clockwise`, else anti-),
    /// writing the result back to its file off-thread. Ignored while a rotate
    /// of the same file is already running.
    fn rotate(&mut self, clockwise: bool) -> Task<Message> {
        let Some(path) = self.claim_rotate() else {
            return Task::none();
        };
        let key = path.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    imaging::rotate_in_place(&path, clockwise)
                })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| {
                Message::Single(SingleMsg::Rotated {
                    path: key.clone(),
                    result,
                })
            },
        )
    }


    /// Claim the current path for a rotate, or `None` if one is already
    /// writing it — two concurrent read-modify-writes of the same file both
    /// read the pre-rotation pixels, so one of the two turns is lost.
    fn claim_rotate(&mut self) -> Option<PathBuf> {
        let path = self.library.current().clone();
        self.rotating.insert(path.clone()).then_some(path)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::library::Library;
    use crate::screens::single::SingleState;
    use std::collections::HashSet;

    /// A single view over `n` images, pointed at the first.
    fn single(n: usize) -> SingleState {
        let files: Vec<PathBuf> = (0..n).map(|i| PathBuf::from(format!("{i}.jpg"))).collect();
        SingleState::new(Library {
            all: files.clone(),
            paths: crate::core::pointed_list::PointedList::new(files).unwrap(),
            image_dir: PathBuf::from("/imgs"),
            selection: HashSet::new(),
            tags: crate::core::tags::Tags::new(),
            filter: None,
        })
    }

    #[test]
    fn a_second_rotate_is_ignored_while_one_is_writing() {
        let mut state = single(2);
        let path = state.library.current().clone();

        assert_eq!(state.claim_rotate(), Some(path.clone()));
        // Holding `r` down must not race two writes against the same file:
        // both would read the pre-rotation pixels and one turn would be lost.
        assert_eq!(state.claim_rotate(), None);

        // The claim is released once the write lands.
        let mut generation = 0;
        let _ = state.update(
            SingleMsg::Rotated {
                path: path.clone(),
                result: Ok(()),
            },
            &mut generation,
        );
        assert_eq!(state.claim_rotate(), Some(path));
    }

    #[test]
    fn rotating_a_different_image_is_not_blocked() {
        let mut state = single(2);
        assert!(state.claim_rotate().is_some());
        // A rotate of the *next* image is unrelated work; only same-file
        // writes race.
        state.library.next();
        assert!(state.claim_rotate().is_some());
    }

    #[test]
    fn a_rotate_re_decodes_the_image_it_rotated() {
        let mut state = single(2);
        let path = state.library.current().clone();
        let mut generation = 0;
        let _ = state.update(SingleMsg::Rotated { path, result: Ok(()) }, &mut generation);
        // `decode_current` bumps the generation; nothing else here does.
        assert_eq!(generation, 1);
    }

    #[test]
    fn a_rotate_landing_after_navigating_away_does_not_re_decode() {
        let mut state = single(2);
        let path = state.library.current().clone();
        state.library.next();

        let mut generation = 0;
        let _ = state.update(SingleMsg::Rotated { path, result: Ok(()) }, &mut generation);
        // Re-decoding here would replace the image on screen with the one the
        // user just navigated off.
        assert_eq!(generation, 0);
    }

    #[test]
    fn a_failed_rotate_releases_the_claim_without_decoding() {
        let mut state = single(2);
        let path = state.library.current().clone();
        assert_eq!(state.claim_rotate(), Some(path.clone()));

        let mut generation = 0;
        let _ = state.update(
            SingleMsg::Rotated {
                path: path.clone(),
                result: Err("nope".into()),
            },
            &mut generation,
        );
        assert_eq!(generation, 0);
        assert_eq!(state.claim_rotate(), Some(path));
    }
}
