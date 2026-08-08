//! The help overlay: what can be done here, and nothing else.
//!
//! Every row comes from a keymap table rather than a list kept beside one, so
//! the overlay cannot claim a key the keyboard does not have — and because each
//! binding carries a guard, what it lists is what is live in this screen, in
//! this mode, right now. Pressing any of it acts and closes the overlay; see
//! `App::key`.
//!
//! The live screen goes above the global keys, as lazygit puts the focused
//! panel's bindings above the ones that work anywhere.

use iced::widget::{center, column, container, row, scrollable, text, Column};
use iced::{Element, Length};

use crate::keymap::Row;

use super::view::overlay_box;
use super::{App, Message, Screen};

/// How tall the overlay may grow before its list starts scrolling. Leaves room
/// for the title and the footer inside a small window.
const MAX_HEIGHT: f32 = 460.0;

/// Width of the key column. Wide enough for `⌘A` and `Space`, narrow enough
/// that the sentences beside them stay in one block.
const KEY_WIDTH: f32 = 96.0;

impl App {
    /// The overlay's sections: the live screen's keys, then the global ones.
    ///
    /// A section with no live bindings is left out entirely, which is what the
    /// empty screen gets — with no photos loaded there is nothing to say about
    /// selections or stacks, only `o`, `e`, `?` and `q`.
    fn help_sections(&self) -> Vec<(&'static str, Vec<Row>)> {
        let screen = match &self.screen {
            Screen::Empty => ("", Vec::new()),
            Screen::Single(s) => ("Photo", s.bindings()),
            // A stack is an ordinary wall over a narrowed library, so it has the
            // wall's keys; naming the section for it is what tells the user
            // which of the two they are standing in.
            Screen::Wall(w) if w.parent.is_some() => ("Stack", w.bindings()),
            Screen::Wall(w) => ("Wall", w.bindings()),
        };

        [screen, ("Everywhere", self.bindings())]
            .into_iter()
            .filter(|(_, rows)| !rows.is_empty())
            .collect()
    }
}

/// The overlay, over whatever the live screen has drawn.
pub(crate) fn help_overlay(app: &App) -> Element<'static, Message> {
    let sections = app
        .help_sections()
        .into_iter()
        .fold(column![].spacing(20), |body, (title, rows)| {
            body.push(section(title, rows))
        });

    let panel = column![
        text("What you can do here").size(22),
        container(scrollable(sections)).max_height(MAX_HEIGHT),
        // The one thing about the overlay that its own rows cannot say.
        text("Any of these acts and closes this list")
            .size(13)
            .style(text::secondary),
    ]
    .spacing(16);

    center(container(panel).padding(28).style(overlay_box)).into()
}

/// One section of the overlay: its name, then a line per binding.
fn section(title: &'static str, rows: Vec<Row>) -> Column<'static, Message> {
    rows.into_iter().fold(
        column![text(title).size(14).style(text::secondary)].spacing(8),
        |section, Row { keys, desc }| {
            section.push(
                row![
                    text(keys).size(16).width(Length::Fixed(KEY_WIDTH)),
                    text(desc).size(16),
                ]
                .spacing(16),
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The empty screen, with the help up. Built directly rather than through
    /// `App::new`, which reads `std::env::args` — under a test harness that is
    /// the harness's own arguments.
    fn empty_app() -> App {
        App {
            screen: Screen::Empty,
            help_open: true,
            fullscreen: false,
            revealed: false,
            confirm: None,
            beneath: None,
            modifiers: iced::keyboard::Modifiers::default(),
        }
    }

    #[test]
    fn a_screen_with_no_keys_of_its_own_gets_no_section() {
        let app = empty_app();
        let sections = app.help_sections();
        assert_eq!(sections.len(), 1, "the empty screen has only the globals");
        assert_eq!(sections[0].0, "Everywhere");

        let keys: Vec<&str> = sections[0].1.iter().map(|row| row.keys.as_str()).collect();
        // With nothing loaded there is no wall to toggle to, so `w` is not
        // offered — but opening something is the whole point of this screen.
        assert!(keys.contains(&"o"), "{keys:?}");
        assert!(!keys.contains(&"w"), "{keys:?}");
    }

    #[test]
    fn the_help_says_how_to_close_itself() {
        let app = empty_app();
        let closing = app
            .help_sections()
            .into_iter()
            .flat_map(|(_, rows)| rows)
            .find(|row| row.desc == "Close this help")
            .expect("the overlay must name its own way out");
        assert_eq!(closing.keys, "? / Esc");
    }
}
