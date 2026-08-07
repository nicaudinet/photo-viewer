//! The single view: one fit-to-window image, with navigation and rotate.
//!
//! Every action here applies to the current image and nothing else — see
//! `SELECT_MODE_PLAN.md`. That includes `f`: whatever the wall has selected,
//! favouriting from this screen touches the photo on it and no other.
//!
//! | module | what it owns |
//! |---|---|
//! | [`message`] | [`SingleMsg`], everything this screen can be told |
//! | [`keys`] | its keyboard |
//! | [`update`] | where a message turns into state and tasks |
//! | [`view`] | the widget tree |

mod keys;
mod message;
mod update;
mod view;

use std::collections::HashSet;
use std::path::PathBuf;

use iced::widget::image;
use iced::Task;

use crate::core::library::Library;
use crate::Message;

pub(crate) use message::SingleMsg;

/// Single-view state: the library plus the current fit-to-window decode.
pub(crate) struct SingleState {
    pub(crate) library: Library,
    /// Latest fit-to-window decode for the current path. `None` until the first
    /// decode lands (the previous image stays on screen meanwhile).
    large: Option<image::Handle>,
    /// Paths with a rotate write in flight. Holding the key down would
    /// otherwise race two read-modify-writes against the same file.
    rotating: HashSet<PathBuf>,
}


impl SingleState {
    pub(crate) fn new(library: Library) -> Self {
        Self {
            library,
            large: None,
            rotating: HashSet::new(),
        }
    }


    /// Kick off an off-thread decode of the current image, tagged with a fresh
    /// generation so an earlier in-flight decode can't overwrite it. Bumps the
    /// caller's global generation counter.
    pub(crate) fn decode_current(&self, generation: &mut u64) -> Task<Message> {
        *generation += 1;
        let generation = *generation;
        let path = self.library.current().clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || crate::core::imaging::full(&path))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::Single(SingleMsg::LargeDecoded { generation, result }),
        )
    }

}
