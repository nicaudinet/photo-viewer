//! Sending photos to the system trash, and asking first.

use std::path::PathBuf;

use iced::Task;

use super::confirm::{Choice, Confirm};
use super::{App, Message, Screen};

impl App {
    /// `d`: ask before trashing the selection.
    ///
    /// The files are named in the answer rather than re-read from the selection
    /// when it is given: the question the user answered stated a count, and
    /// that count has to be what actually happens.
    pub(super) fn ask_about_trash(&mut self) {
        let Screen::Wall(w) = &self.screen else {
            return;
        };
        let Some(selected) = w.operable_selection() else {
            return;
        };
        let prompt = match selected.len() {
            1 => "Move 1 photo to the trash?".to_string(),
            n => format!("Move {n} photos to the trash?"),
        };
        self.confirm = Some(Confirm {
            prompt,
            detail: None,
            choices: vec![Choice {
                key: 'y',
                label: "yes",
                message: Message::Trash(selected),
            }],
        });
    }

    /// `p`: ask before keeping the photograph under the cursor and trashing
    /// the rest of the stack it is in.
    ///
    /// The whole point of a stack is that its photographs are near-identical,
    /// so the question names the one being kept as well as the count. "Trash 7
    /// photos?" is answerable; "trash 7 of these 8 near-identical photos?" is
    /// not, unless it says which one survives.
    pub(super) fn ask_about_pick(&mut self) {
        let Screen::Wall(w) = &self.screen else {
            return;
        };
        let Some(rest) = w.rest_of_the_stack() else {
            return;
        };
        let prompt = match rest.len() {
            1 => "Keep this photo and move the other one to the trash?".to_string(),
            n => format!("Keep this photo and move the other {n} to the trash?"),
        };
        let keeping = w.library.current().file_name().map(|name| {
            format!(
                "Keeping {} \u{2014} the rest of the stack goes",
                name.to_string_lossy()
            )
        });
        self.confirm = Some(Confirm {
            prompt,
            detail: keeping,
            choices: vec![Choice {
                key: 'y',
                label: "yes",
                // The same message `d` sends: what leaves is a list of files,
                // and going back out of the emptied stack falls out of the wall
                // having nothing left to show.
                message: Message::Trash(rest),
            }],
        });
    }

    /// Trash `paths`, off the GUI thread. What actually left the disk comes
    /// back as [`Message::Removed`], which only the app can act on: if nothing
    /// is left there is no library to show.
    pub(super) fn trash(&self, paths: Vec<PathBuf>) -> Task<Message> {
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || trash_all(paths))
                    .await
                    .unwrap_or_else(|e| (Vec::new(), vec![(PathBuf::new(), e.to_string())]))
            },
            |(gone, failed)| Message::Removed { gone, failed },
        )
    }
}

/// Move each path to the system trash, one at a time so a single stubborn file
/// doesn't take the rest of the batch down with it. Returns what actually left
/// the disk and what didn't.
///
/// The trash, not `unlink`: there is no undo in this app, and a mistaken
/// selection of a hundred photos should be a nuisance rather than a
/// catastrophe.
pub(super) fn trash_all(paths: Vec<PathBuf>) -> (Vec<PathBuf>, Vec<(PathBuf, String)>) {
    let mut gone = Vec::new();
    let mut failed = Vec::new();
    for path in paths {
        match trash::delete(&path) {
            Ok(()) => gone.push(path),
            // Already missing: the file is gone either way, which is all the
            // library cares about.
            Err(_) if !path.exists() => gone.push(path),
            Err(e) => failed.push((path, e.to_string())),
        }
    }
    (gone, failed)
}
