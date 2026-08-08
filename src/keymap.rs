//! Keymaps as tables, so a key and its description are one value.
//!
//! A screen's keyboard is a `&'static [Binding<S, O>]`: each entry names the
//! chords that trigger it, what it does, a sentence saying so, and a guard
//! saying when it is live at all. [`lookup`] walks the table to dispatch a key
//! press; [`rows`] walks the same table to render the help. Neither can go
//! stale against the other, because there is only one list.
//!
//! The guard is what makes the help context-aware: `p` — keep this photograph,
//! trash the rest of its stack — means nothing outside a stack, so it is
//! neither offered to the user nor dispatched there. See `HELP_PLAN.md`.

use iced::keyboard::{self, key::Named};

/// One key press, as a table can name it.
///
/// Matching reads three things off the event: the raw key (the layout's key,
/// with no modifiers applied), the modified key (what the layout produces with
/// them applied — `?` for shift and `/`), and the modifiers themselves. Which
/// of the first two carries the character depends on the platform and the
/// layout, so [`Chord::shift`] accepts either.
#[derive(Clone, Copy)]
pub(crate) struct Chord {
    key: Press,
    mods: Mods,
    /// Whether the help shows this chord. `false` for a second spelling of the
    /// same press — a bare `R` beside `⇧R` — which is a platform detail and not
    /// something to teach.
    shown: bool,
}

#[derive(Clone, Copy)]
enum Press {
    /// The character as the user thinks of it: `v`, `?`, `+`, `A` for `⌘A`.
    Char(char),
    Named(Named),
}

#[derive(Clone, Copy, PartialEq)]
enum Mods {
    None,
    Shift,
    Command,
    /// Whatever it took to type the character. `+` is shift and `=` on one
    /// layout and a key of its own on another, and neither spelling is worth
    /// teaching as a chord.
    Any,
}

impl Chord {
    /// A bare character: no shift, no command.
    pub(crate) const fn key(c: char) -> Self {
        Self {
            key: Press::Char(c),
            mods: Mods::None,
            shown: true,
        }
    }

    /// A shifted character, named by what it produces: `⇧R` is `shift('R')`,
    /// and `?` — shift and `/` on a US layout — is `shift('?')`.
    pub(crate) const fn shift(c: char) -> Self {
        Self {
            key: Press::Char(c),
            mods: Mods::Shift,
            shown: true,
        }
    }

    /// A symbol, however the layout produces it: `?`, `+`, `-`. Shown as
    /// itself, because "shift and slash" is not how anyone reads `?`.
    pub(crate) const fn sym(c: char) -> Self {
        Self {
            key: Press::Char(c),
            mods: Mods::Any,
            shown: true,
        }
    }

    /// A character with command held. Named uppercase, as `⌘A` is written.
    pub(crate) const fn cmd(c: char) -> Self {
        Self {
            key: Press::Char(c),
            mods: Mods::Command,
            shown: true,
        }
    }

    /// A named key — Enter, Escape, Space, an arrow.
    pub(crate) const fn named(named: Named) -> Self {
        Self {
            key: Press::Named(named),
            mods: Mods::None,
            shown: true,
        }
    }

    /// The same press written a second way, for the help to leave out.
    pub(crate) const fn alias(self) -> Self {
        Self {
            shown: false,
            ..self
        }
    }

    /// Whether `event` is this chord.
    fn matches(&self, event: &keyboard::Event) -> bool {
        let keyboard::Event::KeyPressed {
            key,
            modified_key,
            modifiers,
            ..
        } = event
        else {
            return false;
        };

        match (self.key, self.mods) {
            (Press::Named(named), _) => {
                key.as_ref() == keyboard::Key::Named(named) && !modifiers.command()
            }
            // A bare key must not answer for a shifted one: `f` favourites and
            // `⇧F` filters, so the guard has to be on both sides.
            (Press::Char(c), Mods::None) => {
                !modifiers.shift()
                    && !modifiers.command()
                    && (is_char(key, c) || is_char(modified_key, c))
            }
            // Either the layout produced the shifted character, or it reported
            // the unshifted one with shift held. Both spellings arrive in the
            // wild, and `?` only ever arrives as the first.
            (Press::Char(c), Mods::Shift) => {
                !modifiers.command()
                    && (is_char(modified_key, c)
                        || is_char(key, c)
                        || (modifiers.shift() && is_char_lowered(key, c)))
            }
            (Press::Char(c), Mods::Any) => {
                !modifiers.command() && (is_char(key, c) || is_char(modified_key, c))
            }
            (Press::Char(c), Mods::Command) => {
                modifiers.command() && (is_char_lowered(key, c) || is_char_lowered(modified_key, c))
            }
        }
    }

    /// How the help writes this chord. The only place a key's spelling is
    /// decided — anything wanting to name a key asks here rather than writing
    /// the string out again.
    pub(crate) fn label(&self) -> String {
        let key = match self.key {
            Press::Char(c) => c.to_string(),
            Press::Named(named) => named_label(named).to_string(),
        };
        match self.mods {
            Mods::None | Mods::Any => key,
            Mods::Shift => format!("\u{21e7}{key}"),
            Mods::Command => format!("\u{2318}{key}"),
        }
    }
}

/// Whether `key` is exactly the character `c`.
fn is_char(key: &keyboard::Key, c: char) -> bool {
    matches!(key.as_ref(), keyboard::Key::Character(s) if s.chars().eq([c]))
}

/// Whether `key` is `c` ignoring case, for the chords named by their uppercase
/// form: `⌘A` arrives as `a`, and `⇧R` as `r` with shift held.
fn is_char_lowered(key: &keyboard::Key, c: char) -> bool {
    matches!(
        key.as_ref(),
        keyboard::Key::Character(s)
            if s.chars().flat_map(char::to_lowercase).eq(c.to_lowercase())
    )
}

fn named_label(named: Named) -> &'static str {
    match named {
        Named::Enter => "\u{21b5}",
        Named::Escape => "Esc",
        // Lower case, like the letter keys beside them: a capital in this
        // column means shift is part of the chord. `Esc` keeps its capital as
        // an abbreviation rather than a word.
        Named::Space => "space",
        Named::ArrowLeft => "\u{2190}",
        Named::ArrowRight => "\u{2192}",
        Named::ArrowUp => "\u{2191}",
        Named::ArrowDown => "\u{2193}",
        Named::Backspace => "\u{232b}",
        // Nothing else is bound; a label is still better than a panic.
        _ => "?",
    }
}

/// One row of a keymap: what to press, what it does, and when it applies.
///
/// `S` is the state the guard and the action read — a screen's own state, or
/// the app's. `O` is whatever that keymap already answers with.
pub(crate) struct Binding<S, O: 'static> {
    /// Every spelling of this press, the shown ones first.
    pub(crate) chords: &'static [Chord],
    /// One sentence, in the imperative: "Trash the selection".
    pub(crate) desc: &'static str,
    /// Whether this binding does anything in the state it is handed. A binding
    /// that is not live is invisible to both the keyboard and the help.
    pub(crate) when: fn(&S) -> bool,
    /// What the press means. Takes the state because some keys read it: Enter
    /// opens whatever the cursor is on.
    pub(crate) act: fn(&S) -> O,
}

impl<S, O> Binding<S, O> {
    /// A binding that is live whenever its screen is.
    pub(crate) const fn always(
        chords: &'static [Chord],
        desc: &'static str,
        act: fn(&S) -> O,
    ) -> Self {
        Self {
            chords,
            desc,
            when: |_| true,
            act,
        }
    }

    /// A binding that is live only in the states `when` accepts.
    pub(crate) const fn when(
        chords: &'static [Chord],
        desc: &'static str,
        when: fn(&S) -> bool,
        act: fn(&S) -> O,
    ) -> Self {
        Self {
            chords,
            desc,
            when,
            act,
        }
    }
}

/// What `event` means in `state`, or `None` if it means nothing here.
///
/// The first live binding that matches wins, so entries that share a key are
/// told apart by their guards rather than by their order — but order still
/// decides if two guards overlap, which is a bug in the table.
pub(crate) fn lookup<S, O>(
    table: &'static [Binding<S, O>],
    state: &S,
    event: &keyboard::Event,
) -> Option<O> {
    table
        .iter()
        .find(|binding| (binding.when)(state) && binding.chords.iter().any(|c| c.matches(event)))
        .map(|binding| (binding.act)(state))
}

/// One line of help: the keys, and what they do.
#[derive(Clone)]
pub(crate) struct Row {
    pub(crate) keys: String,
    pub(crate) desc: &'static str,
}

/// Everything `state` can be told right now, in table order.
pub(crate) fn rows<S, O>(table: &'static [Binding<S, O>], state: &S) -> Vec<Row> {
    table
        .iter()
        .filter(|binding| (binding.when)(state))
        .filter_map(|binding| {
            let keys: Vec<String> = binding
                .chords
                .iter()
                .filter(|c| c.shown)
                .map(Chord::label)
                .collect();
            // Every chord hidden means a binding with nothing to show: a
            // deliberate way to keep a key working without teaching it.
            (!keys.is_empty()).then(|| Row {
                keys: keys.join(" / "),
                desc: binding.desc,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::Modifiers;

    /// A key press, with `modified` for what the layout produces once the
    /// modifiers are applied.
    fn press(key: keyboard::Key, modified: keyboard::Key, modifiers: Modifiers) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key,
            modified_key: modified,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            repeat: false,
            text: None,
        }
    }

    fn ch(c: &str) -> keyboard::Key {
        keyboard::Key::Character(c.into())
    }

    /// The plain press of a character, as an unmodified keyboard sends it.
    fn plain(c: &str) -> keyboard::Event {
        press(ch(c), ch(c), Modifiers::empty())
    }

    #[test]
    fn a_bare_chord_takes_only_the_bare_press() {
        let f = Chord::key('f');
        assert!(f.matches(&plain("f")));
        // Shift or command held is a different chord, and must stay one.
        assert!(!f.matches(&press(ch("f"), ch("F"), Modifiers::SHIFT)));
        assert!(!f.matches(&press(ch("f"), ch("f"), Modifiers::COMMAND)));
        assert!(!f.matches(&plain("g")));
    }

    #[test]
    fn a_shifted_chord_takes_both_spellings() {
        let shift_f = Chord::shift('F');
        // The layout applied the shift...
        assert!(shift_f.matches(&press(ch("f"), ch("F"), Modifiers::SHIFT)));
        // ...or it reported the bare key and the modifier...
        assert!(shift_f.matches(&press(ch("f"), ch("f"), Modifiers::SHIFT)));
        // ...or the uppercase arrived on its own.
        assert!(shift_f.matches(&plain("F")));
        assert!(!shift_f.matches(&plain("f")));
    }

    /// `?` is shift and `/` on a US layout, and a key of its own elsewhere:
    /// a symbol chord takes it either way, and is written as itself.
    #[test]
    fn a_symbol_is_taken_however_it_was_typed() {
        let question = Chord::sym('?');
        assert!(question.matches(&press(ch("/"), ch("?"), Modifiers::SHIFT)));
        assert!(question.matches(&plain("?")));
        assert_eq!(question.label(), "?");
        assert!(!question.matches(&plain("/")));
    }

    #[test]
    fn a_command_chord_needs_command() {
        let all = Chord::cmd('A');
        assert!(all.matches(&press(ch("a"), ch("a"), Modifiers::COMMAND)));
        assert!(!all.matches(&plain("a")));
    }

    #[test]
    fn a_named_chord_matches_the_named_key() {
        let enter = Chord::named(Named::Enter);
        let event = press(
            keyboard::Key::Named(Named::Enter),
            keyboard::Key::Named(Named::Enter),
            Modifiers::empty(),
        );
        assert!(enter.matches(&event));
        assert!(!enter.matches(&plain("a")));
    }

    #[test]
    fn a_release_is_not_a_press() {
        let released = keyboard::Event::KeyReleased {
            key: ch("f"),
            modified_key: ch("f"),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: Modifiers::empty(),
        };
        assert!(!Chord::key('f').matches(&released));
    }

    #[test]
    fn labels_read_as_the_keys_are_written() {
        assert_eq!(Chord::key('v').label(), "v");
        assert_eq!(Chord::shift('R').label(), "\u{21e7}R");
        assert_eq!(Chord::cmd('A').label(), "\u{2318}A");
        assert_eq!(Chord::named(Named::Enter).label(), "\u{21b5}");
        assert_eq!(Chord::named(Named::Escape).label(), "Esc");
    }

    /// A state with a mode, standing in for a screen with one.
    struct Fake {
        painting: bool,
    }

    const TABLE: &[Binding<Fake, &'static str>] = &[
        Binding::when(
            &[Chord::named(Named::Enter)],
            "Commit the range",
            |s| s.painting,
            |_| "commit",
        ),
        // Disjoint with the one above, as entries sharing a key have to be:
        // overlapping guards would show two answers for one press.
        Binding::when(
            &[Chord::named(Named::Enter)],
            "Open",
            |s| !s.painting,
            |_| "open",
        ),
        Binding::always(
            &[
                Chord::named(Named::ArrowLeft),
                Chord::key('h'),
                Chord::shift('H').alias(),
            ],
            "Left",
            |_| "left",
        ),
    ];

    fn enter() -> keyboard::Event {
        press(
            keyboard::Key::Named(Named::Enter),
            keyboard::Key::Named(Named::Enter),
            Modifiers::empty(),
        )
    }

    #[test]
    fn the_live_binding_is_the_one_that_answers() {
        assert_eq!(
            lookup(TABLE, &Fake { painting: true }, &enter()),
            Some("commit")
        );
        assert_eq!(
            lookup(TABLE, &Fake { painting: false }, &enter()),
            Some("open")
        );
        assert_eq!(lookup(TABLE, &Fake { painting: false }, &plain("z")), None);
    }

    #[test]
    fn the_help_shows_the_live_bindings_and_hides_aliases() {
        let painting: Vec<(String, &str)> = rows(TABLE, &Fake { painting: true })
            .into_iter()
            .map(|r| (r.keys, r.desc))
            .collect();
        assert_eq!(
            painting,
            vec![
                ("\u{21b5}".to_string(), "Commit the range"),
                ("\u{2190} / h".to_string(), "Left"),
            ]
        );

        // The guard is the whole of the context-awareness: the same key, the
        // other mode, the other sentence.
        let normal = rows(TABLE, &Fake { painting: false });
        assert_eq!(normal[0].desc, "Open");
        assert_eq!(normal.len(), 2);
    }
}
