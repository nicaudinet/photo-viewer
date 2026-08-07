//! Choosing where a move or a copy is aimed, and looking at what is there.
//!
//! The whole question is assembled before it is put to the user, so it can name
//! the real number of files that would land on a namesake. Asking once, up
//! front, beats interrupting a running batch per clash.

use std::path::{Path, PathBuf};

use iced::Task;

use crate::core::transfer::{self, Collision, TransferKind};
use crate::screens::wall::{BatchKind, WallMsg};

use super::confirm::{Choice, Confirm};
use super::{App, Message, Screen};

/// A move or copy that has been aimed at a folder but not yet agreed to.
///
/// Everything the question needs is gathered before it is asked — including a
/// look at the destination — so the user is told the real number of name
/// clashes once, up front, rather than being interrupted per file.
#[derive(Debug, Clone)]
pub(crate) struct TransferPlan {
    pub(super) kind: TransferKind,
    pub(super) dest: PathBuf,
    pub(super) paths: Vec<PathBuf>,
    /// How many of `paths` already have a namesake in `dest`.
    pub(super) collisions: usize,
    /// The destination is the folder the images are already in.
    pub(super) same_dir: bool,
    /// The destination sits inside the library's folder. The scan is not
    /// recursive, so moved images vanish from the wall — correct, but worth
    /// saying before it happens rather than after.
    pub(super) inside_library: bool,
}

/// Ask for a destination folder, then look at what is already in it.
///
/// The whole question is assembled before it is put to the user: how many files
/// would land on a namesake, and whether the folder is one the wall is showing.
/// Asking once, up front, beats interrupting a running batch per clash.
pub(super) async fn pick_destination(
    kind: TransferKind,
    paths: Vec<PathBuf>,
    image_dir: PathBuf,
) -> Option<TransferPlan> {
    let title = match kind {
        TransferKind::Move => "Move photos to\u{2026}",
        TransferKind::Copy => "Copy photos to\u{2026}",
    };
    let dest = rfd::AsyncFileDialog::new()
        .set_title(title)
        .set_directory(&image_dir)
        .pick_folder()
        .await?
        .path()
        .to_path_buf();

    // One `stat` per file: off the GUI thread, since a big selection over a
    // network share is not instant.
    tokio::task::spawn_blocking(move || {
        let collisions = transfer::collisions(&paths, &dest);
        let same_dir = dest == image_dir;
        TransferPlan {
            kind,
            collisions,
            same_dir,
            inside_library: !same_dir && dest.starts_with(&image_dir),
            dest,
            paths,
        }
    })
    .await
    .ok()
}

/// A folder's own name, for the question — the full path is usually far too
/// long to read at a glance. Falls back to the path for a root directory.
pub(super) fn folder_name(dir: &Path) -> String {
    match dir.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => dir.display().to_string(),
    }
}

impl App {
    /// `m` / `c`: aim an operation at a folder the user picks. Nothing is
    /// written until the question that follows is answered.
    pub(super) fn start_transfer(&self, kind: TransferKind) -> Task<Message> {
        let Screen::Wall(w) = &self.screen else {
            return Task::none();
        };
        let Some(selected) = w.operable_selection() else {
            return Task::none();
        };
        let image_dir = w.library.image_dir.clone();
        Task::perform(
            pick_destination(kind, selected, image_dir),
            Message::TransferTarget,
        )
    }

    /// Turn a destination the user picked into the question to ask about it.
    pub(super) fn ask_about(&mut self, plan: TransferPlan) {
        let folder = folder_name(&plan.dest);
        if plan.same_dir {
            // Nothing to do, and every file would "clash" with itself — so this
            // is a statement rather than a question.
            self.confirm = Some(Confirm {
                prompt: format!("Those photos are already in {folder}."),
                detail: None,
                choices: Vec::new(),
            });
            return;
        }

        let count = plan.paths.len();
        let noun = if count == 1 { "photo" } else { "photos" };
        let prompt = format!("{} {count} {noun} to {folder}?", plan.kind.word());

        let mut detail = Vec::new();
        if plan.collisions > 0 {
            let (n, verb) = (
                plan.collisions,
                if plan.collisions == 1 { "is" } else { "are" },
            );
            detail.push(format!("{n} of them {verb} already there."));
        }
        if plan.inside_library && plan.kind == TransferKind::Move {
            detail.push("They will leave the wall: the folder scan is not recursive.".to_string());
        }

        // With clashes there is no honest yes/no: the answer *is* the policy,
        // chosen once and applied to every file, so nothing is written before
        // the user has said what should happen to the ones already there.
        let choices = if plan.collisions > 0 {
            vec![
                Choice {
                    key: 's',
                    label: "skip those",
                    message: transfer_message(&plan, Collision::Skip),
                },
                Choice {
                    key: 'k',
                    label: "keep both",
                    message: transfer_message(&plan, Collision::KeepBoth),
                },
                Choice {
                    key: 'o',
                    label: "overwrite",
                    message: transfer_message(&plan, Collision::Overwrite),
                },
            ]
        } else {
            vec![Choice {
                key: 'y',
                label: "yes",
                message: transfer_message(&plan, Collision::Skip),
            }]
        };

        self.confirm = Some(Confirm {
            prompt,
            detail: (!detail.is_empty()).then(|| detail.join("\n")),
            choices,
        });
    }
}

/// What picking `collision` on `plan` sends: the wall's own batch runner, over
/// exactly the files the question named.
fn transfer_message(plan: &TransferPlan, collision: Collision) -> Message {
    Message::Wall(WallMsg::StartBatch {
        kind: BatchKind::Transfer {
            kind: plan.kind,
            dest: plan.dest.clone(),
            collision,
        },
        paths: plan.paths.clone(),
    })
}
