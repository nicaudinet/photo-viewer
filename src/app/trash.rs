//! Sending photos to the system trash.

use std::path::PathBuf;

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
