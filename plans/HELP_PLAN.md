# Context-aware help — plan

`?` opens a list of what can be done *here*, and nothing else: the keys the live
screen accepts in the mode it is in, plus the handful the app answers anywhere.
Pressing any of those keys runs it and closes the list.

The problem it solves: the help overlay is a hand-written table of 25 rows shown
identically on every screen. Half of them are prefixed `Wall:` because they are
lies elsewhere, three keys mean different things depending on the mode and can
only describe one of them, and nothing ties any row to the keymap it claims to
document — a binding can be added, moved or deleted without the help noticing.

Modelled on lazygit, where `?` lists the focused panel's bindings above the
global ones, and the list closes as soon as you act.

---

## Design decisions

### One table per keymap, driving both dispatch and help

Each screen's keymap becomes a `&'static [Binding<S, O>]`, where every entry
carries its chords, its description, a guard saying when it is live, and what it
does. `key()` walks the table and returns the first live match; the help walks
the same table and renders every live entry.

The alternative — a `bindings()` function beside `key()`, listing the same keys
a second time — was rejected. Two lists in one file still drift, and the drift
is silent in the direction that matters: help that lies about a key is worse
than no help. Here a binding cannot exist without a description, because they
are the same value.

The guard is what makes the help context-aware, and it pays for itself twice: a
key that does nothing right now is not offered to the user *and* not dispatched,
so `p` outside a stack stops being a keypress that asks a question about nothing.

### A meaning that changes is two bindings, not one description

`Enter` opens a photo, commits a painted range, or descends into a stack; `g`
stacks or unstacks; `⇧F` narrows to favourites or widens back. Each is written
as several entries with disjoint guards and one description each, rather than
one entry whose description is computed. Descriptions stay `&'static str`, the
table stays readable as a list of sentences, and the guards that decide which
one is shown are the same guards that decide what the key does.

### Aliases are hidden, not spelled out

`h` and `←` both mean left and both belong in the help. `r` with shift held and
a bare `R` are the same binding written twice for platforms that disagree about
which one arrives, and only `⇧R` belongs in the help. So a chord carries whether
it is shown, and a binding's key column is its shown chords joined by ` / `.

### The overlay is a reference, not a mode

Any bound key runs and closes the list; an unbound key leaves it up. The list is
therefore never something to get out of before working — which is what it was,
in the worst way: keys already fell through to the screen underneath, so `d`
raised a trash question while the help sat over it.

`?` and `Esc` only close, having nothing else to do. `Esc` in particular keeps
its ladder: with the help up it takes that rung and stops there, so one press
never both closes the help and clears a selection.

### A question is its own help

While a confirmation is up, `?` does nothing — as now. The question names its
answers in its own hint text, which is the only help that could be true while
the ordinary keymap is swapped out from under it.

---

## The shape

```rust
struct Binding<S, O: 'static> {
    chords: &'static [Chord],   // ⏎ / ⌘A / ⇧R, aliases included
    desc: &'static str,
    when: fn(&S) -> bool,       // is this live right now?
    act: fn(&S) -> O,           // reads state: Enter opens the cursor's index
}

fn lookup<S, O>(table, state, event) -> Option<O>   // first live match
fn rows<S, O>(table, state) -> Vec<Row>            // every live entry
```

`S` is the screen's state and `O` what its `key()` already returns — `WallKey`,
`SingleMsg`, `Message`. Nothing else about the screens changes.

Chords match against the raw key, the modified key, and the modifiers:

| Chord | Matches | Shown as |
|---|---|---|
| `key('v')` | `v` with no shift or command | `v` |
| `shift('R')` | modified `R`, or `r` with shift | `⇧R` |
| `cmd('a')` | `a` with command | `⌘A` |
| `named(Enter)` | the named key | `⏎` |

`?` is `shift('?')`: on a US layout the raw key is `/`, and the `?` only exists
in the modified key — which is why the app already special-cases it today.

## Sections

Screen first, global last, as lazygit does it:

| Section | When |
|---|---|
| `Photo` | single view |
| `Wall` | wall view |
| `Stack` | wall view, inside a stack |
| `Global` | always — `?`, `q`, `e`, `w`, `o` |

The empty screen has no screen section: `o`, `q`, `e` and `?` are all it has.

---

## Phase 1 — The keymap machinery

New top-level `src/keymap.rs`: `Chord`, `Binding`, `lookup`, `rows`, and the
labels. Pure, unit-tested against synthetic key events, wired to nothing.

## Phase 2 — The wall's table

`screens/wall/keys.rs` becomes a table. Guards go on `p` (inside a stack),
`+`/`-` (grouping on), the three `Enter`s, and the `Esc` rungs. The existing
tests keep passing unchanged: they press keys and assert on what comes back.

## Phase 3 — The single view's table

The same, smaller. `screens/single/keys.rs`.

## Phase 4 — The app's table

`app/keys.rs`'s `app_key` becomes a table over `App`. `Esc` reaches the wall's
own rungs by falling through the table instead of being forwarded by hand, so
the ladder in `app/update.rs` keeps only the rungs the app owns — a question,
then the help. `App::key` closes the help after dispatching any key that matched.

## Phase 5 — The overlay

`app/view.rs` grows a `help_rows()` that asks the live screen and the app for
their live entries; `SHORTCUTS` is deleted. Sections, two columns, capped height
with a scrollbar, and a footer saying that any key here acts and closes.

---

## Commit order

Phase order. Each phase compiles, passes `cargo test`, and leaves the app
usable: phases 1 to 4 change no behaviour the user can see except which keys are
inert, and phase 5 is the only one that draws anything.
