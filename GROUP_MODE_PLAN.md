# Grouping — plan

`g` on the wall stacks runs of adjacent, similar photos into single tiles. A
stack draws as a slightly messy pile of cards; Enter descends into it, where the
usual wall commands apply plus `p` — keep this one, trash the rest.

The problem it solves: a burst of eight near-identical shots takes eight tiles
of wall to say one thing. Stacked, it takes one, and picking the keeper is two
keystrokes.

Built on top of select mode (`SELECT_MODE_PLAN.md`), which is assumed
throughout.

---

## Design decisions

Recorded here because the reasoning is not recoverable from the code.

### Only adjacent photos are compared

Grouping links photo `i` to photo `i+1` or it doesn't; a stack is a maximal run
of links. Nothing else is ever compared.

This is not a clustering problem and must not become one. Photos in a folder are
already in a meaningful order — chronological, by filename — and a burst is by
definition contiguous within it. Comparing all pairs would be quadratic, would
reorder the wall, and would let two shots from opposite ends of a holiday land
in the same pile because they were both of a beach.

### Similarity is pixels *and* time, not either alone

Two adjacent photos link when all of:

- **same orientation** — portrait never stacks with landscape. Free, and it
  kills a whole class of false positives.
- **dHash distance ≤ `D`** — 8×8 difference hash, Hamming distance over 64 bits.
- **EXIF gap ≤ `T`**, *when both have a timestamp.* Absent on one or both, this
  term is skipped rather than failing.

Each signal alone is wrong in a way the other covers:

- **Time alone** stacks a burst of a fast-moving subject whose frames look
  nothing alike, and can't see that you took the same photo twice ten minutes
  apart.
- **Pixels alone** stacks two unrelated shots of a white wall, or two dark
  frames, that happen to be neighbours.

Skipping the time term rather than refusing to group is what keeps PNGs,
exports, and anything with stripped EXIF working. Falling back to file mtime was
considered and rejected: mtime is right for an untouched camera dump and a lie
after any copy or sync, and a signal that is silently wrong is worse than one
that is silently absent.

### The chain has a drift guard

A slow pan links each frame to the next while frame 1 and frame 9 have nothing
in common, so a naive chain swallows the whole pan into one stack. A candidate
must therefore *also* be within a looser distance `D2` of the group's **first**
member. Failing that starts a new stack.

`D2 = 1.6 × D`. Loose enough that a burst with a hand-shake drift stays whole,
tight enough that a pan breaks into several stacks instead of one.

### Selection always holds real photo paths, never representatives

A stack contributes one photo — its first member — to the visible list. The
obvious next move is to let the selection hold that representative and expand it
at the point of use. Don't.

Instead, `Space` on a stack adds **every member** to the selection, and a
painted range adds every member it covers. `operable_selection`, the batch
queue, `ops.rs`, trash and transfer then need no changes at all: they already
see real paths, and they cannot see anything else.

This is the same argument as the filter's (`SELECT_MODE_PLAN.md`, "the filter
narrows the list, not the drawing"). An expansion hook at the point of use means
every *other* point of use is a latent bug — one that operates on a
representative and quietly leaves seven files behind. Expanding at the point of
selection means there is no such point.

Three consequences, and they are the whole cost:

- `Library::apply_range` expands representatives to members.
- `relist`'s `selection.retain(…)` keeps members of visible stacks, not just
  visible paths.
- `tile_look` goes tri-state: all members selected → full tint and badge; some →
  half tint, no badge; none → nothing.

### A stack is one thing to every command

`d` on a stack trashes all four photos. `f` favourites all four. `m` and `c`
move all four. This follows from the decision above rather than being bolted on
top of it.

The alternative — commands touch the representative only — makes the wall lie:
you'd press `d`, watch the stack vanish, and still have three files on disk.

Confirmations name the real count, so `d` on a stack of four reads
"Trash 4 photos?" and not "Trash 1 photo?".

### Grouping happens after filtering

`relist` applies the filter, then chains what survives. A photo the filter hides
therefore breaks a chain: its neighbours are tested against each other.

That is the honest reading. Grouping describes what is on the wall, and a photo
that is not on the wall is not part of a run.

### Nothing is persisted but the fingerprint cache

`g` recomputes. There is no saved stack layout, no manual split, no manual
merge. Fingerprints *are* cached — in `.photo-viewer/fingerprints`, next to the
tags — because they cost a decode; but they are keyed by content (path, mtime,
size) and are pure derived data, so losing the file costs time and nothing else.

Manual editing is a later feature if the automatic grouping turns out to annoy,
not a launch requirement. A saved layout would have to be reconciled against
every trash, move and rename, which is a lot of machinery to protect a judgement
the user can remake with one keypress.

### Regrouping is automatic

After a trash, a move, or a filter change, the surviving photos are re-chained.
A stack that drops to one member dissolves back into a plain thumbnail.

Fingerprints are already computed by then, so re-chaining is a pure pass over a
list. Frozen groups would mean two photos that become adjacent — because you
trashed the one between them — stay unstacked until you pressed `g` again, and
the wall would be describing a folder that no longer exists.

### Fingerprints are computed on first `g`, not at wall entry

Wall entry stays exactly as fast as it is today. The first `g` on an
un-cached folder runs a bounded pass with progress in the mode bar (the batch
machinery from select-mode phase 4 already does this shape of thing), then
groups. Later presses hit the cache and are instant.

Folding it into the existing `thumb_heights_async` pass at entry was the
alternative: it makes `g` always instant, at the price of a full pixel pass over
every folder you open — including all the ones you never group.

The hash itself is cheap: `jpeg-decoder`'s `scale()` does the IDCT at 1/8 size,
so a fingerprint skips almost all of a full decode. This is the same trick
`imaging::decode_jpeg_prescaled` already uses for thumbnails.

### Descending into a stack reuses the whole wall

Entering a stack builds a second `WallState` over a `Library` narrowed to that
stack's members, and hangs the outer wall off it as a parent. Every wall command
— navigation, selection, visual ranges, rotate, favourite, trash, move — works
inside a stack without a line of new code, and `Screen::Wall` dispatch is
untouched.

Enter is already the key whose meaning depends on context (it commits a painted
range rather than opening an image), so overloading it once more to descend is
in keeping. Esc pops back, restoring the outer wall's scroll position and
selection, because they were never dismantled.

The selection inside a stack is its own, starting empty and discarded on the way
out. Inheriting one would mean Esc had to clear it before it could leave, so
going in and straight back out would cost two presses and look like something
had been thrown away. What *does* come back out is everything about the shared
folder: the tags (edited in there against the same files, so the copy taken on
the way in is the stale one) and the decoded thumbnails (a photo may have been
rotated in there).

Two things this genuinely complicates, both found in phase 6:

- `App::removed` matches on the live screen only. With a parent chain it has to
  prune **every** wall, or popping back reveals one showing files that are gone.
  The walls underneath are pruned *quietly* — no tasks — because a task from a
  hidden wall is answered by the live one, and would scroll it.
- Opening a photograph moves the library to the single view, which would drop
  the chain and strand the user in a folder of four photographs. `App` parks it
  in `beneath` for the trip and hands it back when a wall returns.

### The threshold is tunable live

`+` and `-` walk a ladder of distances and re-group instantly, with the current
rung and the resulting stack count in the mode bar. The hashes are already in
hand, so a rung change is a pure pass.

A single hard-coded default was the alternative. Rejected because the right
threshold is a property of *your photos* — how much you move between frames —
and there is no way to find it except by watching the wall change as you turn
the dial.

---

## Parameters

| Name | Default | What it is |
|---|---|---|
| `D` | 10 | dHash Hamming distance, adjacent pair. Ladder: 4, 7, 10, 14, 18 |
| `D2` | `1.6 × D` | dHash distance to the group's first member — the drift guard |
| `T` | 60 s | EXIF gap, applied only when both photos have a timestamp |
| min size | 2 | A "stack" of one is a photo |

---

## Modes

`g` toggles grouping. It is orthogonal to `NORMAL`/`VISUAL`/`SELECT` — grouped
or not, the wall is still in one of those three, and every motion and selection
key means what it always meant.

Inside a stack, `g` backs out and ungroups the wall, which leaves the
photographs that were in the stack spread across it in place.

Exploding *only* that stack was the first reading, and phase 6 rejected it: the
split would have to survive a re-chain, and a re-chain happens after every
trash, filter change and turn of the dial. Keeping a manual split alive across
those is exactly the machinery "nothing is persisted" set out not to build.

## Tile rendering

A stack draws as `widget::stack!`: up to two blank cards filling the tile,
rotated ±5° via `Rotation::Floating`, then the real thumbnail on top. `Floating`
keeps layout bounds, so the masonry does not reflow.

The cards do **not** overhang the tile, which phase 5 found out the hard way.
iced clips a rotated image to its *unrotated* layout bounds, and a `Stack` gives
every layer at most the size of its base layer, so a card can never be drawn
outside the tile it sits in. Left at full size it would be clipped back to the
photograph's own rect and then hidden behind it — an invisible fan. The
photograph is therefore inset by 10px when it is a stack, and what shows of each
card is the wedge between its leaning edge and that inset. Scaled rather than
trimmed, so a stacked photo is the same shape as an unstacked one.

Two cards at most, and only one behind a pair — a third says nothing the count
badge does not, and a pile of two should look like two.

Blank cards rather than real member thumbnails: members are near-duplicates by
construction, so decoding two more of them buys an effect the eye cannot
distinguish from a tint. The card is a stretched single pixel, because rotation
in iced belongs to images alone; its grey is baked in rather than taken from the
palette for the same reason.

A `×4` count badge sits in the **bottom**-right, using the existing
`corner_badge`. Top-right is the selection tick's and top-left the favourite
star's, so a selected, favourited stack shows all three at once.

## Key routing

| Key | Where | Means |
|---|---|---|
| `g` | wall | toggle grouping |
| `g` | inside a stack | pop, and ungroup the wall |
| `+` / `-` | wall, grouped | loosen / tighten, re-group |
| Enter | wall, on a stack | descend into it |
| Esc | inside a stack | pop (after the usual selection ladder) |
| `p` | inside a stack | keep the cursor photo, trash the rest |

---

## Phase 0 — Read the time a photo was taken

`core/exif.rs` gains a read path beside its write path.

- `taken_at(path) -> Option<i64>`: seconds, from `DateTimeOriginal` (`0x9003`)
  in the Exif SubIFD (`0x8769`), falling back to `DateTime` (`0x0132`) in IFD0.
- Reads a bounded prefix of the file rather than the whole thing — the APP1 is
  near the front and cannot exceed 64 KiB, and this runs over every photo in a
  folder.
- Every failure is `None`. A timestamp is an optional signal, so a malformed one
  is indistinguishable from an absent one and nothing is gained by saying which.

No UI. Tests build JPEGs carrying hand-made EXIF, since `image` cannot write it.

## Phase 1 — Fingerprints

New `core/fingerprint.rs`.

- `Fingerprint { dhash: u64, taken: Option<i64>, landscape: bool }`.
- `fingerprint(path) -> Option<Fingerprint>`, via a 1/8-scale decode.
- `links(a, b, first, threshold) -> bool` — the entire similarity policy, as one
  pure function, drift guard included.

Pure and unit-tested against synthetic images. Nothing else knows it exists.

## Phase 2 — The fingerprint cache

`.photo-viewer/fingerprints`, following `core/tags.rs` for format and placement.
Keyed by path plus mtime plus size, so an edited photo re-hashes.

## Phase 3 — Grouping in the library

`Library` gains `grouping: Option<Grouping>`; `relist` folds groups in after the
filter. Then the three edits from "selection always holds real photo paths":
`apply_range`, `selection.retain`, and the tri-state input for `tile_look`.

Tests only — nothing is on screen yet. This is the phase that decides whether
the feature is clean: if grouping folds into `relist` without any other
subsystem noticing, everything after it is mechanical.

## Phase 4 — `g` on the wall

The key, the async fingerprint pass with progress, the `+`/`-` ladder, and the
mode bar text (`12 stacks · 47 photos · d≤10`).

## Phase 5 — The fanned tile

`wall/tile.rs` and `wall/view.rs`: the three-layer stack, the count badge, the
tri-state tint.

## Phase 6 — Descending into a stack

`WallState.parent`, Enter descends, Esc pops, and `App::removed` walks the
parent chain.

## Phase 7 — `p`

Keep the cursor photo, confirm against the real count, trash the rest, pop on
success.

---

## Commit order

Phase order. Each phase compiles, passes `cargo test`, and leaves the app usable
— phases 0 through 3 add no behaviour at all, which is the point: the risky
thinking is done and tested before anything is drawn.
