//! The queue behind an operation over the selection.
//!
//! A [`Batch`] hands out the next few paths, takes their results back, and says
//! when it is done. It runs no tasks, holds no handles, and knows nothing about
//! iced or the wall — [`super::batch`] is what drives it.
//!
//! Work is released a few files at a time for the same reason thumbnail decodes
//! are: a selection can be thousands of images, and handing the runtime all of
//! them would thrash the disk and the CPU to no end. [`Batch::claim`] is the
//! only place that rule lives.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::transfer::{Collision, TransferKind};

/// The operations a selection can be put through.
#[derive(Debug, Clone)]
pub(crate) enum BatchKind {
    Rotate {
        clockwise: bool,
    },
    Transfer {
        kind: TransferKind,
        dest: PathBuf,
        collision: Collision,
    },
}

impl BatchKind {
    pub(super) fn verb(&self) -> &'static str {
        match self {
            BatchKind::Rotate { .. } => "Rotating",
            BatchKind::Transfer { kind, .. } => kind.verb(),
        }
    }
}

/// What one finished file means for the view — the only part of an operation's
/// result the caller has to care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileDone {
    /// It changed shape on disk, so its thumbnail and height are stale.
    Reshaped,
    /// It is no longer in the library's folder.
    Gone,
    /// Nothing the view shows has changed.
    Unchanged,
}

/// The queue behind a running operation: what is left, what is out, and what
/// came back.
pub(crate) struct Batch {
    kind: BatchKind,
    pending: VecDeque<PathBuf>,
    in_flight: usize,
    done: usize,
    total: usize,
    /// What went wrong, per file. Reported once at the end rather than one
    /// message per failure, which for a whole failed batch would be a wall of
    /// identical lines.
    failed: Vec<(PathBuf, String)>,
    /// Files that have left the library's folder. Collected rather than handed
    /// back one by one: the caller re-lays its view around a removal, and doing
    /// that per file would shuffle the view under whoever is watching it.
    gone: Vec<PathBuf>,
    /// Set by [`Batch::cancel`]: nothing further is handed out, but work
    /// already dispatched is left to land rather than abandoned mid-write.
    cancelled: bool,
}

impl Batch {
    pub(crate) fn new(kind: BatchKind, paths: Vec<PathBuf>) -> Self {
        let pending: VecDeque<PathBuf> = paths.into_iter().collect();
        Self {
            kind,
            total: pending.len(),
            pending,
            in_flight: 0,
            done: 0,
            failed: Vec::new(),
            gone: Vec::new(),
            cancelled: false,
        }
    }

    pub(crate) fn kind(&self) -> &BatchKind {
        &self.kind
    }

    /// Take the next files to dispatch, up to a total of `cap` in flight at
    /// once. Empty once cancelled, or once nothing is left.
    ///
    /// The paths are counted as in flight the moment they are handed out, so
    /// every one must be reported back through [`Batch::record`].
    pub(crate) fn claim(&mut self, cap: usize) -> Vec<PathBuf> {
        if self.cancelled {
            return Vec::new();
        }
        let free = cap.saturating_sub(self.in_flight);
        let mut chosen = Vec::new();
        while chosen.len() < free {
            let Some(path) = self.pending.pop_front() else {
                break;
            };
            chosen.push(path);
        }
        self.in_flight += chosen.len();
        chosen
    }

    /// Fold one finished file in, and say what it means for the view.
    ///
    /// A failure is recorded and then reported as [`FileDone::Unchanged`]: the
    /// file did not move and did not change shape, so there is nothing to
    /// redraw for it.
    pub(crate) fn record(&mut self, path: &Path, result: Result<FileDone, String>) -> FileDone {
        self.in_flight = self.in_flight.saturating_sub(1);
        self.done += 1;
        match result {
            Ok(FileDone::Gone) => {
                self.gone.push(path.to_path_buf());
                FileDone::Gone
            }
            Ok(done) => done,
            Err(e) => {
                self.failed.push((path.to_path_buf(), e));
                FileDone::Unchanged
            }
        }
    }

    /// Whether every dispatched file has landed and no more will be handed out.
    pub(crate) fn is_finished(&self) -> bool {
        self.in_flight == 0 && (self.pending.is_empty() || self.cancelled)
    }

    /// Stop handing out work. Files already dispatched still have to land —
    /// abandoning a write half-done is how a photo gets corrupted.
    pub(crate) fn cancel(&mut self) {
        self.cancelled = true;
        self.pending.clear();
    }

    /// Consume a finished batch: report what failed, and hand back the files
    /// that have left the folder for the caller to drop from its view.
    pub(crate) fn finish(self) -> Vec<PathBuf> {
        if !self.failed.is_empty() {
            eprintln!(
                "{} failed for {} of {} images; first: {}",
                self.kind.verb(),
                self.failed.len(),
                self.total,
                self.failed[0].1
            );
        }
        self.gone
    }

    /// What to show while this is running: the operation and how far along it
    /// is, or — once cancelled — how many files are still landing.
    pub(crate) fn label(&self) -> String {
        if self.cancelled {
            format!("{} \u{2014} finishing {}", self.kind.verb(), self.in_flight)
        } else {
            format!("{} {}/{}", self.kind.verb(), self.done, self.total)
        }
    }
}

/// Counters the wall's tests assert on. Not part of the running API: the queue
/// is driven entirely through `claim`, `record`, `cancel` and `finish`.
#[cfg(test)]
impl Batch {
    pub(super) fn remaining(&self) -> Vec<PathBuf> {
        self.pending.iter().cloned().collect()
    }

    pub(super) fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub(super) fn in_flight(&self) -> usize {
        self.in_flight
    }

    pub(super) fn done(&self) -> usize {
        self.done
    }

    pub(super) fn total(&self) -> usize {
        self.total
    }

    pub(super) fn failed_len(&self) -> usize {
        self.failed.len()
    }

    pub(super) fn gone(&self) -> &[PathBuf] {
        &self.gone
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}
