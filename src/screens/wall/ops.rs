//! What an operation actually does to one file, off-thread.
//!
//! Kept apart from the queue that schedules them: a single rotate runs exactly
//! the same write with no queue in front of it, and should not have to reach
//! into [`super::batch`] to find it.

use std::path::PathBuf;

use crate::core::transfer::{self, Transferred};

use super::queue::{BatchKind, FileDone};

/// Run one file of a batch, off-thread.
pub(super) async fn run_one(path: PathBuf, kind: BatchKind) -> Result<FileDone, String> {
    match kind {
        BatchKind::Rotate { clockwise } => rotate_async(path, clockwise)
            .await
            .map(|()| FileDone::Reshaped),
        BatchKind::Transfer {
            kind,
            dest,
            collision,
        } => tokio::task::spawn_blocking(move || transfer::transfer(&path, &dest, kind, collision))
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
            .map(|done| match done {
                // A move that went through: the image is not in this folder any
                // more, so the wall has to stop showing it.
                Transferred::SourceGone => FileDone::Gone,
                // A copy, or a move skipped over a name clash. Either way the
                // wall is unchanged — and a skipped file stays selected, so a
                // second attempt with a different answer hits exactly those.
                Transferred::SourceKept => FileDone::Unchanged,
            }),
    }
}

/// Rotate one image 90° on disk. Shared with the single-image rotate, which is
/// the same write without a queue in front of it.
pub(super) async fn rotate_async(path: PathBuf, clockwise: bool) -> Result<(), String> {
    tokio::task::spawn_blocking(move || crate::core::imaging::rotate_in_place(&path, clockwise))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}
