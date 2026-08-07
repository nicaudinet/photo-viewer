//! A question waiting on the user.
//!
//! Modal: while one is up the keyboard is swapped for one that speaks only its
//! answers (see [`super::keys`]), and `update` swallows whatever else still
//! arrives. Every way out is named in the hint line — an answer the overlay
//! does not mention is an answer nobody can give.

use std::path::PathBuf;

use iced::Task;

use crate::core::transfer::{Collision, TransferKind};
use crate::screens::wall::{BatchKind, WallMsg};

use super::destination::{folder_name, TransferPlan};
use super::trash::trash_all;
use super::{App, Message, Screen};

/// A question waiting on the user, and what each answer does.
pub(super) struct Confirm {
    pub(super) prompt: String,
    /// Anything that has to be known before answering — a name clash, or the
    /// fact that moved files will drop off the wall.
    pub(super) detail: Option<String>,
    /// The answers on offer, in order. Enter picks the first, so that one must
    /// always be the safe reading of the question. `n` and Esc always cancel,
    /// so no answer may claim either key; nor `q`, which still quits.
    choices: Vec<Choice>,
}

struct Choice {
    key: char,
    label: &'static str,
    action: ConfirmAction,
}

impl Confirm {
    /// The keys line under the question. Every way out is named: an answer the
    /// overlay does not mention is an answer nobody can give.
    pub(super) fn hint(&self) -> String {
        let mut parts: Vec<String> = self
            .choices
            .iter()
            .enumerate()
            .map(|(i, c)| match i {
                0 => format!("{} / \u{21b5} \u{2014} {}", c.key, c.label),
                _ => format!("{} \u{2014} {}", c.key, c.label),
            })
            .collect();
        parts.push("n / Esc \u{2014} cancel".to_string());
        parts.join("      ")
    }
}

enum ConfirmAction {
    /// Move every selected image to the system trash.
    DeleteSelected(Vec<PathBuf>),
    /// Move or copy the selection into the folder the user picked.
    Transfer {
        plan: TransferPlan,
        collision: Collision,
    },
}

impl App {
    /// Carry out the answer keyed `key`, or the first one on offer if `key` is
    /// `None` (what Enter means). An unrecognised key leaves the question up.
    pub(super) fn answer(&mut self, key: Option<char>) -> Task<Message> {
        let Some(confirm) = &self.confirm else {
            return Task::none();
        };
        let found = match key {
            Some(key) => confirm.choices.iter().position(|c| c.key == key),
            // Enter takes the first answer, which is always the safe one.
            None => (!confirm.choices.is_empty()).then_some(0),
        };
        let Some(index) = found else {
            // A question with no answers on offer is a statement; Enter
            // dismisses it, an unrecognised key leaves it up.
            if key.is_none() {
                self.confirm = None;
            }
            return Task::none();
        };
        let mut confirm = self.confirm.take().expect("checked just above");
        self.act(confirm.choices.swap_remove(index).action)
    }

    fn act(&mut self, action: ConfirmAction) -> Task<Message> {
        match action {
            ConfirmAction::DeleteSelected(paths) => Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || trash_all(paths))
                        .await
                        .unwrap_or_else(|e| (Vec::new(), vec![(PathBuf::new(), e.to_string())]))
                },
                |(gone, failed)| Message::Removed { gone, failed },
            ),
            ConfirmAction::Transfer { plan, collision } => self.wall_msg(WallMsg::StartBatch {
                kind: BatchKind::Transfer {
                    kind: plan.kind,
                    dest: plan.dest,
                    collision,
                },
                paths: plan.paths,
            }),
        }
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
                    action: ConfirmAction::Transfer {
                        plan: plan.clone(),
                        collision: Collision::Skip,
                    },
                },
                Choice {
                    key: 'k',
                    label: "keep both",
                    action: ConfirmAction::Transfer {
                        plan: plan.clone(),
                        collision: Collision::KeepBoth,
                    },
                },
                Choice {
                    key: 'o',
                    label: "overwrite",
                    action: ConfirmAction::Transfer {
                        plan,
                        collision: Collision::Overwrite,
                    },
                },
            ]
        } else {
            vec![Choice {
                key: 'y',
                label: "yes",
                action: ConfirmAction::Transfer {
                    plan,
                    collision: Collision::Skip,
                },
            }]
        };

        self.confirm = Some(Confirm {
            prompt,
            detail: (!detail.is_empty()).then(|| detail.join("\n")),
            choices,
        });
    }

    /// `d`: ask before trashing the selection.
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
                action: ConfirmAction::DeleteSelected(selected),
            }],
        });
    }
}
