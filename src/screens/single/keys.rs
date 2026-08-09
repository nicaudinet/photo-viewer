//! The single view's keyboard.
//!
//! Short, because this screen is a flat sequence of images: there is no
//! selection to paint and nothing to move between but previous and next. Every
//! binding it shares with the wall is spelled out again rather than shared,
//! because the two screens are free to disagree about what a key does — `f` here
//! favourites one photo, not a selection.

use iced::keyboard::{self, key::Named};

use crate::keymap::{lookup, rows, Binding, Chord, Row};

use super::message::SingleMsg;
use super::SingleState;

/// What a key press on this screen asks for.
///
/// Nearly all of it the screen answers itself. Two keys do not: leaving for the
/// wall replaces the screen, and trashing puts a question up first and may leave
/// no library to show — both of which only the app can do. Naming them here
/// rather than in the app's own table is what puts them under "Photo" in the
/// help — they are keys of this screen, not ones that work everywhere.
pub(crate) enum SingleKey {
    /// This screen's own business.
    Msg(SingleMsg),
    /// Ask before trashing the photograph on screen.
    Trash,
    /// Leave the photograph and show the wall it came from.
    ShowWall,
}

impl SingleState {
    /// What `event` means here, or `None` if it means nothing.
    pub(crate) fn key(&self, event: &keyboard::Event) -> Option<SingleKey> {
        lookup(SINGLE, self, event)
    }

    /// Everything this screen can be told right now, for the help overlay.
    pub(crate) fn bindings(&self) -> Vec<Row> {
        rows(SINGLE, self)
    }
}

/// The sentence the two movement rows share. There is no up or down here — the
/// screen is a flat sequence — so the words are the sequence's own.
const MOVE: &str = "previous / next";

/// The sentence the two rotate keys share, as on the wall.
const ROTATE: &str = "Rotate left / right";

/// The single view's keyboard. Nothing here is guarded: a photograph on screen
/// can always be moved away from, rotated and favourited, and this screen has
/// no mode in which that stops being true.
const SINGLE: &[Binding<SingleState, SingleKey>] = &[
    // Two rows in the help, as on the wall: the arrows, then their vim twins,
    // in the order `MOVE` names them.
    Binding::always(
        &[Chord::named(Named::ArrowLeft), Chord::key('h')],
        MOVE,
        |_| SingleKey::Msg(SingleMsg::Prev),
    )
    .merged(),
    Binding::always(
        &[Chord::named(Named::ArrowRight), Chord::key('l')],
        MOVE,
        |_| SingleKey::Msg(SingleMsg::Next),
    )
    .merged(),
    Binding::always(&[Chord::key('f')], "Favourite this photo", |_| {
        SingleKey::Msg(SingleMsg::ToggleFavourite)
    }),
    // One row, as on the wall: two keys for the two ways round.
    Binding::always(&[Chord::key('r')], ROTATE, |_| {
        SingleKey::Msg(SingleMsg::RotateAnticlockwise)
    })
    .merged(),
    Binding::always(
        &[Chord::shift('R'), Chord::key('R').alias()],
        ROTATE,
        |_| SingleKey::Msg(SingleMsg::RotateClockwise),
    )
    .merged(),
    // The same key as on the wall, and the same question — only there is no
    // selection here to ask about, so it names the one photograph on screen.
    Binding::always(&[Chord::key('d')], "Move this photo to the trash", |_| {
        SingleKey::Trash
    }),
    // Last, because it is the way out rather than something done to the photo —
    // and one way only. The wall has no `w` back: there a photograph is opened
    // by pointing at it, so the key would have to mean "whichever one is under
    // the cursor", which is not what it says here.
    Binding::always(&[Chord::key('w')], "Show the wall of photos", |_| {
        SingleKey::ShowWall
    }),
];
