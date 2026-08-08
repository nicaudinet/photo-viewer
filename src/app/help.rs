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

use iced::alignment::Horizontal;
use iced::font::Weight;
use iced::theme::palette::lighten;
use iced::widget::{center, column, container, responsive, row, scrollable, text, Column};
use iced::{Element, Font, Length, Theme};

use crate::keymap::Row;

use super::view::overlay_box;
use super::{App, Message, Screen};

/// How much of the window the overlay leaves above and below itself. The list
/// grows into whatever is left and scrolls once it runs out, so a tall window
/// shows the whole keymap and a short one still reads as a panel over a photo
/// rather than as a screen of its own.
const MARGIN: f32 = 40.0;

/// The overlay's own padding, inside the box.
const PADDING: f32 = 16.0;

/// Width of the key column. Wide enough for `⌘A` and `Space`, narrow enough
/// that the sentences beside them stay in one block. The keys are set flush
/// right inside it, so however long the spelling, it ends up against the
/// sentence it belongs to rather than adrift from it.
const KEY_WIDTH: f32 = 84.0;

/// How far the key colour is lifted off the palette's primary — the same lift
/// the wall gives the cursor ring, so the accent means one thing in the app.
const KEY_LIGHTEN: f32 = 0.25;

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
///
/// Laid out against the live window rather than a fixed height: the bindings
/// are read together, so the panel takes all the room the window can spare.
pub(crate) fn help_overlay(app: &App) -> Element<'static, Message> {
    let sections = app.help_sections();

    responsive(move |window| {
        let body = sections
            .iter()
            .cloned()
            .fold(column![].spacing(14), |body, (title, rows)| {
                body.push(section(title, rows))
            });

        let list = container(scrollable(body))
            .max_height((window.height - 2.0 * (MARGIN + PADDING)).max(0.0));

        center(container(list).padding(PADDING).style(overlay_box)).into()
    })
    .into()
}

/// One section of the overlay: its name, then a line per binding.
fn section(title: &'static str, rows: Vec<Row>) -> Column<'static, Message> {
    rows.into_iter().fold(
        column![text(title).size(13).style(text::secondary)].spacing(3),
        |section, Row { keys, desc }| {
            section.push(
                row![
                    text(keys)
                        .size(15)
                        .font(BOLD)
                        .style(key_colour)
                        .width(Length::Fixed(KEY_WIDTH))
                        .align_x(Horizontal::Right),
                    text(desc).size(15),
                ]
                .spacing(12),
            )
        },
    )
}

/// Bold, so a key is told from the sentence about it by weight as well as by
/// colour — the overlay is read by scanning the left edge for a key.
const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};

/// The accent the wall paints the cursor in, applied to the keys themselves.
fn key_colour(theme: &Theme) -> text::Style {
    let palette = theme.extended_palette();
    text::Style {
        color: Some(lighten(palette.primary.base.color, KEY_LIGHTEN)),
    }
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
