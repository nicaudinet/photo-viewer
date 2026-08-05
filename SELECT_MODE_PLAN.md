# Select mode — plan

A modal selection system for the wall view, so groups of images can be acted on
at once: rotate them all, delete them all, move them to a folder.

The interaction model is vim's: a `NORMAL` mode where a cursor moves over the
wall, a `VISUAL` mode that paints a range as you move, and — unlike vim — a
persistent `SELECT` mode that holds the committed selection while you operate on
it.

Favourites and mark-to-delete are removed first (Phase 0) and reintroduced later
(Phase 6) on top of this machinery, where they fit more naturally.

---

## Design decisions

Recorded here because the reasoning is not recoverable from the code.

### Selection is a set of paths, not indices

`HashSet<PathBuf>`, matching how the old favourites set worked. Deleting or
moving files renumbers `PointedList`, so an index-keyed selection would silently
come to mean different images. Paths survive it.

### The range is a linear run, not a rectangle

The wall is shortest-column masonry, so `j` from a tile can jump a variable
number of library indices. Two readings of "anchor to cursor" follow:

- **linear run** — every library index between the two. Looks jagged on screen
  when columns are ragged.
- **geometric rectangle** — only tiles inside the bounding box. Looks tidy, but
  the set then depends on the window width, so a resize silently changes what
  you selected.

Linear wins. It is width-independent, it matches vim, and it matches how photos
are actually grouped (chronological filenames). The jaggedness is honest: it
shows you exactly what you got.

There is no row-wise (`V`) variant. Rows in ragged masonry are not a real thing
— `slot.row == 3` sits at a different `y` in every column — so "select these
rows" would mean something different per column. `Cmd+A` and `i` (invert) cover
the same ground without lying.

### There is exactly one cursor, and it is the library cursor

Motions move `library.paths.index()` in every mode. `VISUAL` stores only its
anchor; the moving end of the range *is* the cursor. This means `navigate`,
`neighbour`, `reveal` and `desired_y` are untouched by this whole feature.

The cursor's **ring is not drawn in `VISUAL`** — the leading edge of the pending
range already shows where you are, and a second highlight competing with the
range tint reads as noise. It is drawn normally in `NORMAL` and `SELECT`.

### Escape restores the cursor from VISUAL, but not from SELECT

Leaving `VISUAL` returns the cursor to the anchor: you pressed `v` at a place,
changed your mind, and should end up back where you started.

Leaving `SELECT` leaves the cursor where it currently is. In `SELECT` the user
moves the cursor deliberately (to pick the next run to add or remove), so
teleporting it back to a position from several operations ago would be
surprising rather than helpful.

### Editing a selection is re-entering VISUAL

Rather than a second grammar for "modify the selection", `v` from `SELECT`
starts an additive range and `x` starts a subtractive one. Same motions, same
preview, one extra key. `Space` toggles the single tile under the cursor, for
scattered picks that aren't runs.

### Escape is a one-step ladder

`VISUAL` over `SELECT` takes two presses to fully exit. One keypress must never
silently discard a forty-image selection.

### Filters are gone, and stay gone while a selection is active

The favourites/to-delete filters disappear in Phase 0. When they return in Phase
6 they must be **disabled whenever a selection is active**. A filter that hides
selected images means a delete-selected can destroy files the user cannot see —
disabling the combination removes the entire bug class rather than papering over
it with warnings.

### Single view never touches more than one image

Every action in the single view applies to the current image and nothing else,
regardless of what is selected. There is no visible selection on that screen, so
acting on invisible files from it is the worst available surprise. The selection
survives the trip and is shown as a badge, but only the wall operates on it.

### Rotation must be lossless

See Phase 3. Batch-rotating 200 JPEGs through a decode/re-encode cycle would
degrade every one of them, and slowly.

---

## Modes

```rust
enum WallMode {
    Normal,
    Visual { anchor: usize, op: RangeOp },
    Select,
}

enum RangeOp { Add, Remove }
```

The pending range is `min(anchor, cursor)..=max(anchor, cursor)`.

| From | Key | To | Effect |
|---|---|---|---|
| NORMAL | `v` | VISUAL(Add) | anchor = cursor |
| NORMAL | `Space` | SELECT | select the cursor tile |
| NORMAL | `Cmd`+click | SELECT | select the clicked tile |
| VISUAL | motions | VISUAL | move cursor, live-preview the range |
| VISUAL | `Enter` | SELECT | union (Add) or subtract (Remove) the range |
| VISUAL | `v` / `Esc` | previous mode | drop pending, cursor back to anchor |
| SELECT | motions | SELECT | move cursor, selection unchanged |
| SELECT | `v` | VISUAL(Add) | anchor = cursor |
| SELECT | `x` | VISUAL(Remove) | anchor = cursor |
| SELECT | `Space` | SELECT | toggle the cursor tile |
| SELECT | `Cmd+A` / `i` | SELECT | select all / invert |
| SELECT | `Esc` | NORMAL | clear the set, cursor stays put |
| SELECT | (set emptied) | NORMAL | automatic |

`WallMode` does not need to survive a `w` toggle: `enter()` reconstructs it as
`Select` when `library.selection` is non-empty, `Normal` otherwise. `VISUAL` is
inherently transient, so `w` while in it drops the pending range and keeps the
committed set.

## Tile rendering

Four states, computed in `build_grid`:

1. **cursor** (`NORMAL`/`SELECT` only) — the existing lightened-primary ring
2. **selected** — tint fill, accent border, corner checkmark badge (reuses
   `corner_icon`, so the state is not conveyed by hue alone)
3. **pending add** — the same tint at lower alpha
4. **pending remove** — desaturated tint over a still-selected tile

Committed membership is `selection.contains(path)`, O(1); pending membership is
a bounds check against the range. No per-frame allocation. Every tile already
carries `SEL_BORDER` padding, so none of this reflows the masonry.

## Status bar

Shown at the bottom whenever the mode is not `NORMAL`:

```
-- VISUAL -- 12 (+5)
SELECT 12 · r rotate · d delete · m move · v add · x remove · Esc clear
```

This is the entire discoverability story for the feature, and in `VISUAL` it is
also what tells the user the missing cursor ring is intentional.

## Key routing

`subscription` stays screen-blind and emits neutral messages (`Message::KeyV`,
`KeyX`, `KeySpace`, `Activate`, `KeyD`); `App::update` dispatches by screen and
`WallState::update` dispatches by mode. This is the existing pattern for `f`/`d`
and needs no new machinery.

---

## Phase 0 — Remove favourites and to-delete

Its own commit, before anything else. Everything downstream is smaller for it.

**`library.rs`** — drop `favourites`, `to_delete`, `favourite`, `unfavourite`,
`delete`, `undelete`, `delete_all`, `save`, `save_favourites`, `join_paths`,
`load_cache_set`, `cache_dir`, `favourites_file`, `to_delete_file`,
`LibraryError::NotInLibrary`. `Library` becomes `paths` + `image_dir`. Around 14
of the 30 tests go with them.

**`wall.rs`** — drop `WallFilter`, `FilterFavourites`, `FilterToDelete`,
`after_filter_change`, `ensure_current_displayed`, and the icon handles threaded
through `view` / `build_grid` / `thumb_element`. `is_displayed` collapses to
`true`; inline it and drop the first priority tier in `prioritise`.

**`single.rs`** — drop `ToggleFavourite`, `ToggleDelete`, `SaveFavourites`,
`SaveFavDirPicked`, `toggle_favourite`, `toggle_delete`, `confirm_delete`, and
the icon overlay in `view`.

**`main.rs`** — drop `KeyF`, `KeyD`, `DeleteAll`, `ConfirmYes`, `ConfirmNo`,
`star_icon`, `delete_icon`, `STAR_ICON`, `DELETE_ICON`, the modal-swallow guard,
`confirm_yes`, `do_delete_all`.

Keep `corner_icon` in `gui/mod.rs` and the files in `icons/` — the selection
badge and Phase 6 want both. Keep `rfd`; the folder picker returns in Phase 5.

Also update `README.md` (its keybinding tables and its link to the deleted
`RUST_REWRITE_PLAN.md`) and the `SHORTCUTS` table in `gui/mod.rs`.

Deletion disappears entirely until Phase 4. That gap is intended: it returns as
a selection operation rather than as a per-image mark. Existing `.photo-viewer/`
cache directories in users' photo folders are left in place — inert, and useful
when Phase 6 arrives.

## Phase 1 — Modes, selection model, rendering

`selection: HashSet<PathBuf>` on `Library`, so it crosses the `w` toggle and is
pruned alongside `paths`. `WallMode` on `WallState`. All the transitions,
rendering and status-bar work above. No operations yet — this phase is purely
about being able to build, edit and see a selection.

## Phase 2 — Mouse

- **NORMAL**: click opens the image (unchanged); `Cmd`+click enters `SELECT`
  with that tile — the mouse-only route in, for anyone who never learns `v`.
- **SELECT / VISUAL**: click toggles a tile, `Shift`+click toggles the run from
  the last-clicked tile, double-click opens that one image without disturbing
  the selection.

`ThumbClicked(index)` needs modifier state, which iced's `button` does not carry
on press. Track modifiers from the existing global `keyboard::listen()`
subscription rather than wrapping every tile in a `mouse_area`.

Rubber-band drag selection is deliberately deferred past Phase 6:
`WallLayout::slots` already holds exact `y`/`height`/`col` per index, so the hit
test is a rectangle intersection with no new layout pass, but pointer capture
needs a custom `advanced` widget — the most iced-specific work in the plan.

## Phase 3 — Lossless rotation

`rotate_in_place` currently does `image::open` → `rotate90` → `save`: a full
decode and re-encode. For JPEG that is generation loss on every keypress.

**Rule: JPEG rewrites the EXIF orientation tag; PNG keeps the pixel rotate**
(PNG is a lossless codec, so re-encoding it costs only time). The JPEG path then
touches no pixels at all — a metadata write, microseconds, zero loss, correct
for any dimensions — which also makes Phase 4's batch rotate effectively
instant.

The tag write is the easy half. The work is honouring orientation on read:

- **`imaging::full`, `thumbnail`, and `thumb_height` must all apply it.** For
  orientations 5–8 width and height swap, and the masonry is built from
  `thumb_height` — miss it and the layout is wrong for every rotated file.
  `image` 0.25 offers `ImageDecoder::orientation()` and
  `DynamicImage::apply_orientation`; the `jpeg-decoder` fast path needs the tag
  read separately and applied by hand.
- **`thumb_height` must keep matching `thumbnail` exactly** — the existing
  invariant (and its test) that stops the wall shifting when a decode lands.
  Orientation has to be applied identically on both sides.
- **Writing the tag.** Done in `src/exif.rs` with no new dependency, contrary to
  the original sketch below. `little_exif` pulls six crates (brotli, quick-xml,
  crc, …) and re-serialises the whole EXIF block to change two bytes. The risk
  in editing EXIF is that moving bytes inside the TIFF block invalidates every
  interior offset past the move — including ones inside a camera `MakerNote`,
  which are not always self-describing. So `exif.rs` never moves an existing
  byte:
  - orientation already present: overwrite its two value bytes in place;
  - EXIF present but no orientation tag: append a *copy* of IFD0 carrying the
    new entry to the end of the TIFF block and repoint the header at it, leaving
    the original as dead space;
  - no EXIF at all: splice in a minimal APP1.

  ~~New dependency for writing EXIF: `little_exif`, or `img-parts` plus
  `kamadak-exif`.~~
- **Trade-off**: applications that ignore EXIF orientation will show the old
  rotation. Nearly all modern ones honour it; a few command-line tools do not.
  The alternative — `turbojpeg`'s lossless DCT-block transform, which really
  moves the pixels — is also lossless but trims partial edge blocks when the
  dimensions are not a multiple of 16, and pulls in libjpeg-turbo.

## Phase 4 — Operations on the selection

Every single-image operation maps over the selection independently, bounded the
way the decode scheduler is bounded.

```rust
struct Batch {
    kind: BatchKind,            // Rotate { clockwise } | Move { dest } | Copy { dest }
    pending: VecDeque<PathBuf>,
    in_flight: HashSet<PathBuf>,
    done: usize,
    total: usize,
    failed: Vec<(PathBuf, String)>,
}
```

`refill()` mirrors `schedule()`: dispatch up to `max_in_flight()` for
CPU-bound rotates, around 4 for IO-bound moves and copies, refilling as each
lands. The existing `rotating: HashSet` still guards per-path write races; a
batch simply claims each path once.

While a batch runs:

- **Never call `reveal()` per completion** — the wall would scroll around under
  the user. Revealing is a `NORMAL`-mode navigation concern only.
- `refocus()` and `desired_y = None` run once at the end, not per image.
- Per completion, reuse the existing `rotated()` logic for thumbnail
  invalidation (`thumbs.remove`, `stale` marking, provisional height swap) — it
  is already correct for a single path.
- Progress overlay: `Rotating 37/200 — Esc to cancel`. Cancelling drains
  `pending`; in-flight jobs are allowed to land.
- On partial failure, report the count and **leave the failed paths selected**,
  so a retry hits exactly those.

**Delete selected** (`d`, behind a confirm overlay):

- One blocking task, not a batch queue — unlinking is cheap.
- Not the old cursor-walk from `delete_all`. Unlink the set, rebuild the
  `PointedList`, land the cursor on the **nearest surviving index by old
  position**, then clear the selection.
- Use the `trash` crate rather than `fs::remove_file`. With no undo in the app,
  recoverable deletion is the difference between a mistake and a disaster.

Generalise the confirm overlay at the same time: move it out of
`SingleState::confirm_delete` into `App { confirm: Option<Pending> }` carrying a
prompt string, since deletion now originates in the wall.

`w` from `SELECT` opens the single view on the **first selected image** with the
selection preserved and badged; single-view operations still touch only that
image.

## Phase 5 — Move and copy to a folder

`m` moves, `c` copies, both through the `rfd` folder picker, both through the
`Batch` runner, both behind a confirm showing the count and the destination.

- **Collisions**: an overlay offering *Skip / Rename (`name-1.jpg`) / Overwrite*
  with an "apply to all" toggle. The old `save_favourites` merely `eprintln!`ed
  a skip, which is not acceptable for an interactive operation.
- **Cross-filesystem**: `fs::rename` fails with `EXDEV`; fall back to copy plus
  remove.
- **Destination inside `image_dir`**: the scan is non-recursive, so moved files
  vanish from the wall. Correct, but surprising — say so in the confirm.
- **Partial failure**: keep the survivors selected.
- Move prunes the moved paths from `paths` and `selection`; copy changes
  nothing but the disk.

## Phase 6 — Favourites, reborn

With selection in place, favourites shrink to a tag applied to a selection plus
a filter that shows only tagged images. Two constraints carry over: the filter
must be **disabled while a selection is active**, and storage should probably be
general tags rather than one hard-coded flag — which turns the old
`save_favourites` into a special case of Phase 5's `c`.

---

## Commit order

Phase 0 alone → Phase 1 alone → Phases 2 and 3 in either order (independent) →
Phase 4 → Phase 5 → Phase 6.
