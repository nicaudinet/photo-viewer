//! Which screen is live, and every way of changing that.
//!
//! The library lives inside `Single`/`Wall`, so those screens are
//! unrepresentable without one. Moving between them hands the library across
//! and drops the losing screen's decode caches — a deliberate, revisitable
//! trade-off. Only `App` can do this, which is why it is here and not on a
//! screen.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use iced::Task;

use crate::core::library::{load_library, Library};
use crate::screens::single::SingleState;
use crate::screens::wall::WallState;

use super::{App, Message};

/// Which view is on screen, together with that view's own state.
///
/// The library lives inside `Single`/`Wall`, so those views are unrepresentable
/// without one — `Empty` genuinely carries nothing. Toggling between views moves
/// the library across; the losing view's decode caches are dropped (and re-built
/// on return — a deliberate, revisitable trade-off).
// The wall's state is the larger by some way, but exactly one `Screen` exists
// for the life of the process — boxing it would buy an indirection on every
// access and save nothing.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Screen {
    Empty,
    Single(SingleState),
    Wall(WallState),
}

impl App {
    /// Load the directory for `path`. A file opens in single view pointed at
    /// that file; a directory opens in the wall view (matching the Python app).
    pub(super) fn open(&mut self, path: PathBuf) -> Task<Message> {
        let (dir, target) = if path.is_file() {
            let parent = path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.clone());
            (parent, Some(path))
        } else {
            (path, None)
        };

        match load_library(&dir) {
            Ok(Some(mut lib)) => match target {
                // A file: open it directly in single view.
                Some(file) => {
                    lib.paths.goto_value(&file);
                    let single = SingleState::new(lib);
                    let task = single.decode_current(&mut self.generation);
                    self.screen = Screen::Single(single);
                    task
                }
                // A directory: show the wall of all its images.
                None => {
                    let mut wall = WallState::new(lib);
                    let task = wall.enter();
                    self.screen = Screen::Wall(wall);
                    task
                }
            },
            Ok(None) => {
                self.screen = Screen::Empty;
                Task::none()
            }
            Err(e) => {
                eprintln!("Failed to load {}: {e}", dir.display());
                self.screen = Screen::Empty;
                Task::none()
            }
        }
    }

    /// Leave the wall and open `index` in the single view. Shared by clicking a
    /// thumbnail and by pressing Enter on the selection.
    pub(super) fn open_index(&mut self, index: usize) -> Task<Message> {
        match std::mem::replace(&mut self.screen, Screen::Empty) {
            Screen::Wall(w) => {
                let mut library = w.library;
                library.goto(index);
                let single = SingleState::new(library);
                let task = single.decode_current(&mut self.generation);
                self.screen = Screen::Single(single);
                task
            }
            // Only the wall opens by index; restore anything else.
            other => {
                self.screen = other;
                Task::none()
            }
        }
    }

    /// Move the library across to the other view. The losing view's decode
    /// cache is dropped, so the new view always decodes fresh.
    pub(super) fn toggle_wall(&mut self) -> Task<Message> {
        match std::mem::replace(&mut self.screen, Screen::Empty) {
            Screen::Single(s) => {
                let mut wall = WallState::new(s.library);
                let task = wall.enter();
                self.screen = Screen::Wall(wall);
                task
            }
            Screen::Wall(w) => {
                let single = SingleState::new(w.library);
                let task = single.decode_current(&mut self.generation);
                self.screen = Screen::Single(single);
                task
            }
            Screen::Empty => Task::none(),
        }
    }

    /// Images have left the library's folder — trashed, or moved elsewhere.
    /// Falls back to the empty screen if none are left, which is why this sits
    /// here rather than on the wall: only `App` can change screens.
    pub(super) fn removed(&mut self, gone: Vec<PathBuf>, failed: Vec<(PathBuf, String)>) -> Task<Message> {
        for (path, error) in &failed {
            eprintln!("Could not trash {}: {error}", path.display());
        }
        let gone: HashSet<PathBuf> = gone.into_iter().collect();
        if gone.is_empty() {
            return Task::none();
        }
        match &mut self.screen {
            Screen::Wall(w) => match w.removed(&gone) {
                (true, task) => task,
                (false, _) => {
                    self.screen = Screen::Empty;
                    Task::none()
                }
            },
            _ => Task::none(),
        }
    }

    /// The current library, whichever loaded view holds it.
    pub(super) fn library(&self) -> Option<&Library> {
        match &self.screen {
            Screen::Single(s) => Some(&s.library),
            Screen::Wall(w) => Some(&w.library),
            Screen::Empty => None,
        }
    }

}
