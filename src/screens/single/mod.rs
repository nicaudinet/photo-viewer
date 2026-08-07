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
use std::sync::atomic::{AtomicU64, Ordering};

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
    /// The generation of the latest decode this screen asked for. A decode
    /// landing with an older number has been superseded and is dropped.
    generation: u64,
}

/// Source of decode generations, monotonic for the life of the process.
///
/// Deliberately not a counter inside [`SingleState`]: leaving the single view
/// and coming back builds a fresh one, and a per-screen counter would restart
/// at zero while a decode from the previous visit was still in flight. That
/// decode would then carry a number the new screen is about to reuse, and the
/// pixels of the image you left would land on the image you are looking at.
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn next_generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::Relaxed) + 1
}


impl SingleState {
    pub(crate) fn new(library: Library) -> Self {
        Self {
            library,
            large: None,
            rotating: HashSet::new(),
            generation: 0,
        }
    }


    /// Kick off an off-thread decode of the current image, tagged with a fresh
    /// generation so an earlier in-flight decode can't overwrite it.
    pub(crate) fn decode_current(&mut self) -> Task<Message> {
        self.generation = next_generation();
        let generation = self.generation;
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
