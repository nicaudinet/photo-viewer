//! A question waiting on the user.
//!
//! Modal: while one is up the keyboard is swapped for one that speaks only its
//! answers (see [`super::keys`]), and `update` swallows whatever else still
//! arrives.
//!
//! Mechanism only. A [`Choice`] carries the [`Message`] to send if it is
//! picked, so this module never learns what any answer does — whoever asks the
//! question decides that, and adding a new one costs nothing here.

use iced::Task;

use super::{App, Message};

/// A question waiting on the user, and what each answer sends.
pub(super) struct Confirm {
    pub(super) prompt: String,
    /// Anything the user has to know before answering, and would not otherwise
    /// see. Whoever asks the question decides what that is.
    pub(super) detail: Option<String>,
    /// The answers on offer, in order. Enter picks the first, so that one must
    /// always be the safe reading of the question. `n` and Esc always cancel,
    /// so no answer may claim either key; nor `q`, which still quits.
    pub(super) choices: Vec<Choice>,
}

pub(super) struct Choice {
    pub(super) key: char,
    pub(super) label: &'static str,
    /// What picking this answer sends. Built when the question is asked, so
    /// what happens is settled by what the user was shown, not by whatever the
    /// app happens to hold by the time they answer.
    pub(super) message: Message,
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
        // Taken before the message is sent, so `update` sees no question up and
        // does not swallow the very thing the user just asked for.
        let mut confirm = self.confirm.take().expect("checked just above");
        let message = confirm.choices.swap_remove(index).message;
        self.update(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Screen;

    /// A question whose answers are told apart by what they send: the key `t`
    /// toggles the help overlay, every other key does nothing observable.
    fn question(keys: &[char]) -> Confirm {
        Confirm {
            prompt: "Do the thing?".to_string(),
            detail: None,
            choices: keys
                .iter()
                .map(|&key| Choice {
                    key,
                    label: "do it",
                    message: if key == 't' {
                        Message::ToggleHelp
                    } else {
                        Message::ConfirmNo
                    },
                })
                .collect(),
        }
    }

    /// The app, with `question` already on screen. Built directly rather than
    /// through `App::new`, which reads `std::env::args` — under a test harness
    /// that is the harness's own arguments.
    fn asked(keys: &[char]) -> App {
        App {
            screen: Screen::Empty,
            help_open: false,
            fullscreen: false,
            revealed: false,
            confirm: Some(question(keys)),
            modifiers: iced::keyboard::Modifiers::default(),
        }
    }

    #[test]
    fn enter_sends_what_the_first_answer_carries() {
        let mut app = asked(&['t', 'k', 'o']);
        let _ = app.answer(None);
        assert!(app.confirm.is_none());
        assert!(app.help_open, "the first answer's message was not sent");
    }

    #[test]
    fn a_named_key_sends_what_that_answer_carries() {
        // `t` is last, so picking it proves the key was matched rather than the
        // first choice taken.
        let mut app = asked(&['s', 'k', 't']);
        let _ = app.answer(Some('t'));
        assert!(app.confirm.is_none());
        assert!(app.help_open, "the keyed answer's message was not sent");
    }

    #[test]
    fn a_declined_answer_sends_nothing() {
        let mut app = asked(&['s', 't']);
        let _ = app.answer(Some('s'));
        assert!(!app.help_open, "the wrong answer's message was sent");
    }

    #[test]
    fn an_unrecognised_key_leaves_the_question_up() {
        let mut app = asked(&['s', 'k']);
        let _ = app.answer(Some('z'));
        assert!(app.confirm.is_some());
    }

    #[test]
    fn a_statement_is_dismissed_by_enter_alone() {
        let mut app = asked(&[]);
        let _ = app.answer(Some('z'));
        assert!(app.confirm.is_some(), "a stray key must not dismiss it");
        let _ = app.answer(None);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn the_hint_names_every_way_out() {
        let hint = question(&['s', 'k', 'o']).hint();
        for key in ['s', 'k', 'o'] {
            assert!(hint.contains(key), "hint did not name {key}: {hint}");
        }
        assert!(hint.contains("Esc"), "hint did not name the way to cancel");
    }
}
