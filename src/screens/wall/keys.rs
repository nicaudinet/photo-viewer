//! The wall's keyboard.
//!
//! Every key that means something on the wall is named here, and nowhere else,
//! as one table: the chords, the sentence the help shows, the guard saying when
//! the key is live, and what it does. The app hands over the raw event and takes
//! back either a [`WallKey`] — the wall's own business — or one of the few
//! things only the app can do.
//!
//! The guards are the whole of the wall's context-awareness. The wall has a
//! mode, a selection, a filter, stacks and a wall underneath, and most keys mean
//! something in some of those and nothing in the rest: `p` outside a stack, `+`
//! with nothing stacked, `x` with nothing selected. A guarded-off binding is
//! neither dispatched nor shown, so the help lists what can be done here and the
//! keyboard does exactly that much.

use iced::keyboard::{self, key::Named};

use crate::core::library::RangeOp;
use crate::core::transfer::TransferKind;
use crate::keymap::{lookup, rows, Binding, Chord, Row};

use super::message::{Dir, WallMsg};
use super::select::WallMode;
use super::WallState;

/// What a key press on the wall asks for.
///
/// Most of them the wall answers itself. The rest need the app: opening an
/// image means leaving the wall, and trashing or transferring a selection puts
/// a question on screen first. Naming them here keeps them out of the app's
/// vocabulary, where they would read as things any screen might ask for.
pub(crate) enum WallKey {
    /// The wall's own business.
    Msg(WallMsg),
    /// Leave the wall and open this index in the single view.
    Open(usize),
    /// Ask before trashing the selection.
    Trash,
    /// Ask before keeping the photograph under the cursor and trashing the
    /// rest of the stack it is in.
    Pick,
    /// Aim a move or copy at a folder the user picks.
    Transfer(TransferKind),
}

impl WallState {
    /// What `event` means here, or `None` if it means nothing.
    pub(crate) fn key(&self, event: &keyboard::Event) -> Option<WallKey> {
        lookup(WALL, self, event)
    }

    /// Everything the wall can be told right now, for the help overlay.
    pub(crate) fn bindings(&self) -> Vec<Row> {
        rows(WALL, self)
    }

    /// Whether there is a committed selection to operate on.
    ///
    /// The cheap half of [`WallState::operable_selection`]: same answer, no walk
    /// over the library, because a guard is asked on every key press. The mode
    /// invariant is what makes it the same answer — outside `Visual` the mode is
    /// `Select` exactly when the selection is non-empty.
    fn has_selection(&self) -> bool {
        self.mode == WallMode::Select && self.batch.is_none()
    }

    /// Whether the wall is showing stacks rather than plain photographs.
    fn stacked(&self) -> bool {
        self.library.grouping.is_some()
    }

    /// Whether this wall is a stack, with the wall it came from underneath.
    fn in_stack(&self) -> bool {
        self.parent.is_some()
    }

    /// Whether the tile under the cursor stands for more than one photograph.
    fn on_stack(&self) -> bool {
        self.stack_at(self.library.paths.index())
    }

    /// Whether anything is running that Esc — or a second `g` — would stop.
    fn working(&self) -> bool {
        self.batch.is_some() || self.hashing.is_some()
    }
}

/// One entry of the wall's keyboard.
type WallBinding = Binding<WallState, WallKey>;

/// Whether the filter can be turned either way right now.
///
/// Refused while anything is selected — a filter that hid a selected photo
/// would put images the user cannot see into the next batch — and while a batch
/// is running over the list it would change.
fn filterable(w: &WallState) -> bool {
    !w.is_visual() && !w.has_selection() && w.batch.is_none() && w.library.selection.is_empty()
}

/// The wall's keyboard, in press-order: move, open, select, act.
///
/// Entries that share a key have disjoint guards, so which one answers is a
/// question about the wall's state and never about their order here.
const WALL: &[WallBinding] = &[
    // Movement. Live in every mode: in `Visual` the cursor is the moving end of
    // the range, which is why painting needs no keys of its own.
    Binding::always(
        &[Chord::named(Named::ArrowLeft), Chord::key('h')],
        "Move left",
        |_| WallKey::Msg(WallMsg::Nav(Dir::Left)),
    ),
    Binding::always(
        &[Chord::named(Named::ArrowRight), Chord::key('l')],
        "Move right",
        |_| WallKey::Msg(WallMsg::Nav(Dir::Right)),
    ),
    Binding::always(
        &[Chord::named(Named::ArrowUp), Chord::key('k')],
        "Move up a row",
        |_| WallKey::Msg(WallMsg::Nav(Dir::Up)),
    ),
    Binding::always(
        &[Chord::named(Named::ArrowDown), Chord::key('j')],
        "Move down a row",
        |_| WallKey::Msg(WallMsg::Nav(Dir::Down)),
    ),
    // Enter is the key whose meaning depends most on where the wall is: a range
    // being painted is waiting to be committed, and a stack has no single
    // photograph to open, so Enter goes into it instead.
    Binding::when(
        &[Chord::named(Named::Enter)],
        "Commit the painted range",
        WallState::is_visual,
        |_| WallKey::Msg(WallMsg::CommitVisual),
    ),
    Binding::when(
        &[Chord::named(Named::Enter)],
        "Open this stack",
        |w| !w.is_visual() && w.on_stack(),
        |w| {
            WallKey::Msg(WallMsg::Descend {
                index: w.library.paths.index(),
            })
        },
    ),
    Binding::when(
        &[Chord::named(Named::Enter)],
        "Open this photo",
        |w| !w.is_visual() && !w.on_stack(),
        |w| WallKey::Open(w.library.paths.index()),
    ),
    // Selection. The three set edits are refused mid-paint — they would end the
    // range behind the user's back — so mid-paint they are not offered either.
    Binding::when(
        &[Chord::named(Named::Space)],
        "Select or deselect this photo",
        |w| !w.is_visual(),
        |_| WallKey::Msg(WallMsg::ToggleCursor),
    ),
    Binding::when(
        &[Chord::key('v')],
        "Paint a range to select",
        |w| !w.is_visual(),
        |_| WallKey::Msg(WallMsg::EnterVisual { op: RangeOp::Add }),
    ),
    // `x` means "remove a run", which is meaningless with nothing selected.
    Binding::when(
        &[Chord::key('x')],
        "Paint a range to deselect",
        |w| !w.is_visual() && w.has_selection(),
        |_| {
            WallKey::Msg(WallMsg::EnterVisual {
                op: RangeOp::Remove,
            })
        },
    ),
    // Painting, either key cancels — as `v` does in vim.
    Binding::when(
        &[Chord::key('v'), Chord::key('x')],
        "Cancel the painted range",
        WallState::is_visual,
        |_| WallKey::Msg(WallMsg::EnterVisual { op: RangeOp::Add }),
    ),
    Binding::when(
        &[Chord::cmd('A')],
        "Select all",
        |w| !w.is_visual(),
        |_| WallKey::Msg(WallMsg::SelectAll),
    ),
    Binding::when(
        &[Chord::key('i')],
        "Invert the selection",
        |w| !w.is_visual(),
        |_| WallKey::Msg(WallMsg::InvertSelection),
    ),
    // Everything below acts on the selection where there is one and on the
    // photograph under the cursor where there is not, so each one is written
    // twice: the key is the same, the sentence is not.
    Binding::when(
        &[Chord::key('f')],
        "Favourite the selection",
        |w| !w.is_visual() && w.has_selection(),
        |_| WallKey::Msg(WallMsg::ToggleFavourite),
    ),
    Binding::when(
        &[Chord::key('f')],
        "Favourite this photo",
        |w| !w.is_visual() && !w.has_selection(),
        |_| WallKey::Msg(WallMsg::ToggleFavourite),
    ),
    // The filter is refused while anything is selected: it could hide a selected
    // photo, and the next batch would touch images the user cannot see.
    Binding::when(
        &[Chord::shift('F')],
        "Show every photo again",
        |w| filterable(w) && w.library.filter.is_some(),
        |_| WallKey::Msg(WallMsg::ToggleFilter),
    ),
    Binding::when(
        &[Chord::shift('F')],
        "Show only the favourites",
        |w| filterable(w) && w.library.filter.is_none(),
        |_| WallKey::Msg(WallMsg::ToggleFilter),
    ),
    Binding::when(
        &[Chord::key('r')],
        "Rotate the selection anticlockwise",
        |w| !w.is_visual() && !w.working() && w.has_selection(),
        |_| WallKey::Msg(WallMsg::Rotate { clockwise: false }),
    ),
    Binding::when(
        &[Chord::shift('R'), Chord::key('R').alias()],
        "Rotate the selection clockwise",
        |w| !w.is_visual() && !w.working() && w.has_selection(),
        |_| WallKey::Msg(WallMsg::Rotate { clockwise: true }),
    ),
    Binding::when(
        &[Chord::key('r')],
        "Rotate this photo anticlockwise",
        |w| !w.is_visual() && !w.working() && !w.has_selection(),
        |_| WallKey::Msg(WallMsg::Rotate { clockwise: false }),
    ),
    Binding::when(
        &[Chord::shift('R'), Chord::key('R').alias()],
        "Rotate this photo clockwise",
        |w| !w.is_visual() && !w.working() && !w.has_selection(),
        |_| WallKey::Msg(WallMsg::Rotate { clockwise: true }),
    ),
    // Move, copy and trash have no cursor reading: they ask a question naming a
    // count, and "1 photo" is not what the user meant to be asked about.
    Binding::when(
        &[Chord::key('m')],
        "Move the selection to another folder",
        |w| !w.is_visual() && w.has_selection(),
        |_| WallKey::Transfer(TransferKind::Move),
    ),
    Binding::when(
        &[Chord::key('c')],
        "Copy the selection to another folder",
        |w| !w.is_visual() && w.has_selection(),
        |_| WallKey::Transfer(TransferKind::Copy),
    ),
    Binding::when(
        &[Chord::key('d')],
        "Move the selection to the trash",
        |w| !w.is_visual() && w.has_selection(),
        |_| WallKey::Trash,
    ),
    // Stacking. Inside a stack `g` backs out and ungroups, which is the only
    // reading that survives a re-chain — see `GROUP_MODE_PLAN.md`.
    Binding::when(
        &[Chord::key('g')],
        "Stop stacking",
        |w| !w.is_visual() && w.batch.is_none() && w.hashing.is_some(),
        |_| WallKey::Msg(WallMsg::ToggleGrouping),
    ),
    Binding::when(
        &[Chord::key('g')],
        "Leave the stack and unstack the wall",
        |w| !w.is_visual() && !w.working() && w.in_stack(),
        |_| WallKey::Msg(WallMsg::ToggleGrouping),
    ),
    Binding::when(
        &[Chord::key('g')],
        "Unstack the wall",
        |w| !w.is_visual() && !w.working() && !w.in_stack() && w.stacked(),
        |_| WallKey::Msg(WallMsg::ToggleGrouping),
    ),
    Binding::when(
        &[Chord::key('g')],
        "Stack similar photos",
        |w| !w.is_visual() && !w.working() && !w.in_stack() && !w.stacked(),
        |_| WallKey::Msg(WallMsg::ToggleGrouping),
    ),
    // The dial only exists while there are stacks to tune. `=` as well as `+`,
    // so loosening does not need the shift key held.
    Binding::when(
        &[Chord::sym('+'), Chord::sym('=').alias()],
        "Looser stacks",
        WallState::stacked,
        |_| WallKey::Msg(WallMsg::Retune { looser: true }),
    ),
    Binding::when(
        &[Chord::sym('-')],
        "Tighter stacks",
        WallState::stacked,
        |_| WallKey::Msg(WallMsg::Retune { looser: false }),
    ),
    // Only inside a stack: out on the folder a plain thumbnail has no rest to
    // trash, and on a stack the user cannot see which member they would keep.
    Binding::when(
        &[Chord::key('p')],
        "Keep this photo, trash the rest of the stack",
        |w| !w.is_visual() && w.batch.is_none() && w.in_stack(),
        |_| WallKey::Pick,
    ),
    // Esc is a ladder, and the help shows the rung the user is standing on. The
    // app takes the rungs above these — a question, then the help itself.
    Binding::when(
        &[Chord::named(Named::Escape)],
        "Stop the operation",
        |w| w.batch.is_some(),
        |_| WallKey::Msg(WallMsg::Escape),
    ),
    Binding::when(
        &[Chord::named(Named::Escape)],
        "Stop stacking",
        |w| w.batch.is_none() && w.hashing.is_some(),
        |_| WallKey::Msg(WallMsg::Escape),
    ),
    Binding::when(
        &[Chord::named(Named::Escape)],
        "Cancel the painted range",
        |w| !w.working() && w.is_visual(),
        |_| WallKey::Msg(WallMsg::Escape),
    ),
    Binding::when(
        &[Chord::named(Named::Escape)],
        "Clear the selection",
        |w| !w.working() && w.mode == WallMode::Select,
        |_| WallKey::Msg(WallMsg::Escape),
    ),
    // The last rung, and only inside a stack: elsewhere Esc has nothing left to
    // drop, so it is not offered at all.
    Binding::when(
        &[Chord::named(Named::Escape)],
        "Leave the stack",
        |w| !w.working() && w.mode == WallMode::Normal && w.in_stack(),
        |_| WallKey::Msg(WallMsg::Escape),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::wall::fixture::*;

    /// A key press with no modifiers held.
    fn press_key(state: &WallState, key: keyboard::Key) -> Option<WallKey> {
        state.key(&keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            repeat: false,
            text: None,
        })
    }

    fn press(state: &WallState, c: &str) -> Option<WallKey> {
        press_key(state, keyboard::Key::Character(c.into()))
    }

    fn enter(state: &WallState) -> Option<WallKey> {
        press_key(state, keyboard::Key::Named(Named::Enter))
    }

    /// What the help would say about `key` right now, if anything.
    fn help_for(state: &WallState, key: &str) -> Option<String> {
        state
            .bindings()
            .into_iter()
            .find(|row| row.keys.split(" / ").any(|k| k == key))
            .map(|row| row.desc.to_string())
    }

    #[test]
    fn enter_opens_the_image_under_the_cursor() {
        let mut state = wall(&[200.0; 6], 1);
        state.library.goto(3);
        assert!(matches!(enter(&state), Some(WallKey::Open(3))));
    }

    #[test]
    fn enter_commits_the_range_instead_while_painting() {
        let mut state = wall(&[200.0; 6], 1);
        enter_visual(&mut state, RangeOp::Add);
        assert!(matches!(
            enter(&state),
            Some(WallKey::Msg(WallMsg::CommitVisual))
        ));
    }

    #[test]
    fn the_keys_that_need_the_app_say_so() {
        let mut state = wall(&[200.0; 6], 1);
        // They act on the selection, so they exist once there is one.
        let _ = state.update(WallMsg::SelectAll);
        assert!(matches!(press(&state, "d"), Some(WallKey::Trash)));
        assert!(matches!(
            press(&state, "m"),
            Some(WallKey::Transfer(TransferKind::Move))
        ));
        assert!(matches!(
            press(&state, "c"),
            Some(WallKey::Transfer(TransferKind::Copy))
        ));
    }

    #[test]
    fn g_toggles_grouping_and_the_dial_turns_both_ways() {
        let mut state = wall(&[200.0; 6], 1);
        assert!(matches!(
            press(&state, "g"),
            Some(WallKey::Msg(WallMsg::ToggleGrouping))
        ));
        // The dial only exists once there are stacks to turn it on.
        assert!(press(&state, "+").is_none());

        group(&mut state, &[0, 1, 2, 40, 41, 42]);
        for key in ["+", "="] {
            assert!(
                matches!(
                    press(&state, key),
                    Some(WallKey::Msg(WallMsg::Retune { looser: true }))
                ),
                "{key} should loosen"
            );
        }
        assert!(matches!(
            press(&state, "-"),
            Some(WallKey::Msg(WallMsg::Retune { looser: false }))
        ));
    }

    #[test]
    fn enter_goes_into_a_stack_instead_of_opening_it() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 1, 2, 40, 41, 42]);
        // A stack has no single photograph to open.
        assert!(matches!(
            enter(&state),
            Some(WallKey::Msg(WallMsg::Descend { index: 0 }))
        ));

        state.library.goto(1);
        descend(&mut state, 1);
        // And inside it, Enter opens photographs again.
        assert!(matches!(enter(&state), Some(WallKey::Open(0))));
    }

    #[test]
    fn p_asks_the_app_because_it_needs_a_question_first() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 1, 2, 40, 41, 42]);
        descend(&mut state, 0);
        assert!(matches!(press(&state, "p"), Some(WallKey::Pick)));
    }

    #[test]
    fn p_does_not_exist_outside_a_stack() {
        let state = wall(&[200.0; 6], 1);
        // Nothing to pick between, so the key is inert and unlisted.
        assert!(press(&state, "p").is_none());
        assert_eq!(help_for(&state, "p"), None);
    }

    #[test]
    fn an_unbound_key_means_nothing_here() {
        let state = wall(&[200.0; 6], 1);
        assert!(press(&state, "z").is_none());
    }

    #[test]
    fn the_help_describes_the_wall_the_user_is_looking_at() {
        let mut state = wall(&[200.0; 6], 1);
        assert_eq!(
            help_for(&state, "\u{21b5}"),
            Some("Open this photo".to_string())
        );
        assert_eq!(help_for(&state, "f"), Some("Favourite this photo".into()));
        assert_eq!(help_for(&state, "g"), Some("Stack similar photos".into()));
        // Nothing is selected and nothing is running, so Esc has no rung.
        assert_eq!(help_for(&state, "Esc"), None);
        assert_eq!(help_for(&state, "x"), None);

        let _ = state.update(WallMsg::SelectAll);
        assert_eq!(
            help_for(&state, "f"),
            Some("Favourite the selection".into())
        );
        assert_eq!(help_for(&state, "Esc"), Some("Clear the selection".into()));
        assert_eq!(
            help_for(&state, "x"),
            Some("Paint a range to deselect".into())
        );

        enter_visual(&mut state, RangeOp::Add);
        assert_eq!(
            help_for(&state, "\u{21b5}"),
            Some("Commit the painted range".to_string())
        );
        assert_eq!(
            help_for(&state, "Esc"),
            Some("Cancel the painted range".into())
        );
        // A set edit mid-paint would end the range behind the user's back.
        assert_eq!(help_for(&state, "Space"), None);
    }

    #[test]
    fn a_stack_offers_the_way_back_out() {
        let mut state = wall(&[200.0; 6], 1);
        group(&mut state, &[0, 1, 2, 40, 41, 42]);
        assert_eq!(help_for(&state, "g"), Some("Unstack the wall".into()));

        descend(&mut state, 0);
        assert_eq!(help_for(&state, "Esc"), Some("Leave the stack".into()));
        assert_eq!(
            help_for(&state, "p"),
            Some("Keep this photo, trash the rest of the stack".into())
        );
    }
}
