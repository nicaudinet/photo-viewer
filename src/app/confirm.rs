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

use crate::keymap::Row;

use super::{App, Message};

/// A question waiting on the user, and what each answer sends.
pub(super) struct Confirm {
    pub(super) prompt: String,
    /// Anything the user has to know before answering, and would not otherwise
    /// see. Whoever asks the question decides what that is.
    pub(super) detail: Option<String>,
    /// The answers on offer, in order. `n` always says no, so no answer may
    /// claim that key; nor `q`, which asks about quitting.
    ///
    /// Every answer is a letter the user types. There is no default one: a
    /// question about files that cannot be got back should take a deliberate
    /// key, not whatever the hands were already resting on.
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
    /// The answers, as the same key rows the help overlay is made of. Every way
    /// out is named: an answer the overlay does not mention is an answer nobody
    /// can give.
    pub(super) fn answers(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .choices
            .iter()
            .map(|c| Row {
                keys: c.key.to_string(),
                desc: c.label,
            })
            .collect();
        rows.push(Row {
            keys: "n".to_string(),
            // A question with nothing on offer is a statement, and "no" is no
            // answer to one.
            desc: if self.choices.is_empty() { "ok" } else { "no" },
        });
        rows
    }
}

impl App {
    /// Carry out the answer keyed `key`. An unrecognised key leaves the
    /// question up: only the keys the overlay names do anything.
    pub(super) fn answer(&mut self, key: char) -> Task<Message> {
        let Some(confirm) = &self.confirm else {
            return Task::none();
        };
        let Some(index) = confirm.choices.iter().position(|c| c.key == key) else {
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
            beneath: None,
            modifiers: iced::keyboard::Modifiers::default(),
        }
    }

    #[test]
    fn a_named_key_sends_what_that_answer_carries() {
        // `t` is last, so picking it proves the key was matched rather than the
        // first choice taken.
        let mut app = asked(&['s', 'k', 't']);
        let _ = app.answer('t');
        assert!(app.confirm.is_none());
        assert!(app.help_open, "the keyed answer's message was not sent");
    }

    #[test]
    fn a_declined_answer_sends_nothing() {
        let mut app = asked(&['s', 't']);
        let _ = app.answer('s');
        assert!(!app.help_open, "the wrong answer's message was sent");
    }

    #[test]
    fn an_unrecognised_key_leaves_the_question_up() {
        let mut app = asked(&['s', 'k']);
        let _ = app.answer('z');
        assert!(app.confirm.is_some());
    }

    #[test]
    fn the_answers_name_every_way_out() {
        let rows = question(&['s', 'k', 'o']).answers();
        let keys: Vec<&str> = rows.iter().map(|row| row.keys.as_str()).collect();
        assert_eq!(
            keys,
            ["s", "k", "o", "n"],
            "the answers named the wrong keys"
        );
    }

    #[test]
    fn a_statement_is_acknowledged_rather_than_refused() {
        let rows = question(&[]).answers();
        assert_eq!(rows.len(), 1, "a statement offers only the way out");
        assert_eq!(rows[0].keys, "n");
        assert_eq!(rows[0].desc, "ok");
    }
}
