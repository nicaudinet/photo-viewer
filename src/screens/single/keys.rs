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

impl SingleState {
    /// What `event` means here, or `None` if it means nothing.
    pub(crate) fn key(&self, event: &keyboard::Event) -> Option<SingleMsg> {
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

/// The single view's keyboard. Nothing here is guarded: a photograph on screen
/// can always be moved away from, rotated and favourited, and this screen has
/// no mode in which that stops being true.
const SINGLE: &[Binding<SingleState, SingleMsg>] = &[
    // Two rows in the help, as on the wall: the arrows, then their vim twins,
    // in the order `MOVE` names them.
    Binding::always(
        &[Chord::named(Named::ArrowLeft), Chord::key('h')],
        MOVE,
        |_| SingleMsg::Prev,
    )
    .merged(),
    Binding::always(
        &[Chord::named(Named::ArrowRight), Chord::key('l')],
        MOVE,
        |_| SingleMsg::Next,
    )
    .merged(),
    Binding::always(&[Chord::key('f')], "Favourite this photo", |_| {
        SingleMsg::ToggleFavourite
    }),
    Binding::always(&[Chord::key('r')], "Rotate anticlockwise", |_| {
        SingleMsg::RotateAnticlockwise
    }),
    Binding::always(
        &[Chord::shift('R'), Chord::key('R').alias()],
        "Rotate clockwise",
        |_| SingleMsg::RotateClockwise,
    ),
];
