# PhotoViewer

A keyboard-driven image viewer built with **Rust + [iced](https://iced.rs)**
(Elm architecture). Built and deployed with Nix flakes.

Rewritten from an earlier PySide6/Qt implementation.

Selection is modal and vim-shaped: a cursor moves over a wall of thumbnails,
`v` paints a range, and whatever is selected can be rotated, favourited, sent
to another folder or trashed in one go. See
[`SELECT_MODE_PLAN.md`](SELECT_MODE_PLAN.md) for the design and its reasoning.

## Keybindings

### Global

| Key | Action |
|-----|--------|
| `?` | What you can do here — the keys the live screen accepts in the mode it is in, and no others ([`HELP_PLAN.md`](HELP_PLAN.md)). Any key in the list acts and closes it |
| `Esc` | A ladder, one rung per press: the help, then a running operation, a painted range, the selection, and finally the stack you are in |
| `q` | Quit |
| `e` | Toggle fullscreen |
| `o` | Open directory (native dir picker) |

### Single view

| Key | Action |
|-----|--------|
| `←` / `h` | Previous image (circular) |
| `→` / `l` | Next image (circular) |
| `r` | Rotate anticlockwise 90° — writes the file to disk |
| `Shift+R` | Rotate clockwise 90° — writes the file to disk |
| `f` | Favourite this image |
| `w` | Switch to wall view |

Every action in the single view applies to the current image and nothing else.

Rotation is lossless. A JPEG is turned by rewriting its EXIF orientation tag —
the way a camera records which way up a photo was taken — so the compressed
image is never decoded and re-encoded, and can be turned as often as you like
without degrading. It is also instant, whatever the size of the photo. PNG has
no such tag and is turned by its pixels, which costs nothing beyond the time,
the format being lossless anyway.

### Wall view

| Key | Action |
|-----|--------|
| `←` / `h` | Move the selection left |
| `→` / `l` | Move the selection right |
| `↑` / `k` | Move the selection up a row |
| `↓` / `j` | Move the selection down a row |
| `Enter` | Commit a painted range, else open the image in single view |
| `r` | Rotate the selected image anticlockwise 90° |
| `Shift+R` | Rotate the selected image clockwise 90° |
| `f` | Favourite the image under the cursor |
| `Shift+F` | Show only the favourites, or show everything again |
| `w` | Switch to single view |
| click | Go to that image in single view |

#### Selection

Selecting is modal, in the vim sense. `NORMAL` moves a cursor. `v` enters
`VISUAL`, where moving the cursor paints a range; `Enter` commits it and `Esc`
cancels it, returning the cursor to where `v` was pressed. A committed range
puts the wall in `SELECT`, where the cursor still moves freely and further
ranges can be added or subtracted. A bar along the bottom shows the live mode
and count.

The range is a run of images in library order, not a rectangle on screen — so
it means the same thing whatever the window width.

| Key | Action |
|-----|--------|
| `v` | Paint a range that adds to the selection |
| `x` | Paint a range that removes from it (needs a selection) |
| `Enter` | Commit the painted range |
| `Space` | Select or deselect the image under the cursor |
| `Cmd+A` | Select every image |
| `Esc` | Stop a running operation, then cancel the range, then clear |

#### Acting on a selection

Every operation applies to each selected image independently.

| Key | Action |
|-----|--------|
| `r` / `Shift+R` | Rotate every selected image |
| `f` | Favourite every selected image |
| `m` | Move every selected image to another folder |
| `c` | Copy every selected image to another folder |
| `d` | Move every selected image to the trash (asks first) |

Files are moved to the system trash rather than unlinked — there is no undo in
the app, so a mistaken selection should be a nuisance rather than a
catastrophe. Deleting asks for confirmation first.

`m` and `c` open a folder picker and then ask, naming the count and the folder.
Nothing is written before that question is answered. The destination is
inspected first, so if any of the photos would land on a file of the same name
the question says how many and offers what to do about it — *skip those*, *keep
both* (writing alongside as `name-1.jpg`), or *overwrite* — once, for the whole
batch, rather than interrupting per file. Moving is a rename where it can be,
which is instant; across filesystems it falls back to a copy followed by a
delete, and the original is only removed once the copy has succeeded.

A moved photo leaves the wall. If you move photos into a folder inside the one
you are viewing they will still disappear, because the scan is not recursive —
the question says so before you agree to it. Copying changes nothing but the
disk.

Files that could not be sent, and files skipped over a name clash, stay
selected — so answering differently and pressing the key again retries exactly
those.

Work is dispatched a few files at a time rather than all at once, the same way
thumbnail decodes are, and a bar along the bottom shows progress. `Esc` stops
it: nothing further is started, but files already being written are allowed to
finish, since abandoning a write half-done is how a photo gets corrupted.

While a range is being painted, none of these do anything — an uncommitted
range has no settled meaning, so there is no honest answer to "which images?".

While a question is on screen it is the only thing the keyboard talks to: its
answers are named along the bottom of it, and those are the only keys that do
anything (besides `Esc` to cancel and `q` to quit).

The mouse is modal in the same way. With nothing selected a plain click still
opens the image; `Cmd`-click is the way into a selection without touching the
keyboard. Once a selection is live, plain clicks select instead and opening
moves to a double click.

| Mouse | Action |
|-------|--------|
| click (nothing selected) | Open the image in single view |
| click (selection live) | Select or deselect that image |
| `Cmd`-click | Select or deselect, whatever the mode |
| `Shift`-click | Select the run from the cursor to that image |
| double click | Open the image, leaving the selection unchanged |
| click (while painting) | Extend the painted range to that image |

Every click moves the cursor to what it hit, so `v` after a click anchors where
you are looking.

The selection is remembered while you visit the single view, but never acted on
from there: every single-view action applies to the current image alone. It is
not written to disk.

#### Favourites

`f` favourites the whole selection, or — with nothing selected — just the image
under the cursor. Toggling a group makes the group agree: it favourites them
unless they all already are, in which case it takes the mark off all of them.
Favourites carry a star in the top-left corner of the thumbnail, opposite the
selection tick, so a photo that is both shows both.

`Shift+F` narrows the wall to the favourites and back. While it is narrowed a
bar along the bottom says so and how many of the folder you are looking at.

Filtering is refused while anything is selected. A filter that hid a selected
photo would put images you cannot see into the next batch — `Esc` clears the
selection in one key. Underneath, the filter narrows the list that the cursor
and the selection are expressed in, so a hidden photo has no index anything can
name; anything that drops off the wall drops out of the selection with it.

Favouriting the last one and then un-favouriting it puts the whole folder back
rather than leaving you looking at nothing, and asking to filter when nothing is
favourited does nothing at all — a blank wall would claim the folder was empty.

The narrowing follows you into the single view, so opening a favourite and then
pressing `l` walks the favourites rather than the whole folder. Favouriting from
that screen still touches only the photo on it, and never drops it out from
under you — the wall works out what to show on the way back.

Favourites are stored as `<folder>/.photo-viewer/tags/favourite`, a list of file
names — not full paths, so renaming or moving the folder keeps them attached to
the photos. It is written as soon as anything changes; there is no save key. The
format is a general tag store with one file per tag, though only `favourite` has
a key so far. A `favourites` file from the older versions of this app is read
once and carried over.

## Development

Enter the dev shell (Rust toolchain, `rust-analyzer`, `clippy`, plus the
runtime graphics libs on Linux):

```bash
nix develop
```

Then:

```bash
cargo run -- [path/to/image/or/directory]   # launch (a file opens single view, a dir opens wall view)
cargo build
cargo clippy
cargo test                                   # domain + platform unit tests
cargo fmt                                    # the tree is kept rustfmt-clean
```

### Git hooks

The repo carries a pre-commit hook that refuses a commit which would leave the
tree unformatted. Hooks are not installed by cloning, so point git at them
once per clone:

```bash
git config core.hooksPath .githooks
```

Worth setting at the same time, so `git blame` skips the tree-wide reformat
rather than attributing lines to it:

```bash
git config blame.ignoreRevsFile .git-blame-ignore-revs
```

Bypass the hook for a deliberate exception with `git commit --no-verify`.

## Run

```bash
nix run                          # launch
nix run . -- ~/Pictures/foo.jpg  # open a file or directory
```

## Install (macOS)

Build and install a proper `PhotoViewer.app` bundle into `~/Applications`:

```bash
nix run .#install-app
```

This makes PhotoViewer available in Spotlight and in Finder's **Open With**
menu, and lets you set it as the default viewer for an image type via
**Get Info > Open with > Change All**.

Double-clicking or "Open With" delivers the file as a macOS
`kAEOpenDocuments` Apple Event (handled in `src/core/platform.rs`), not on argv.

To build the bundle without installing:

```bash
nix build .#app     # result/Applications/PhotoViewer.app
```

The bundle is unsigned but built locally, so Gatekeeper allows it without a
quarantine prompt. If macOS ever blocks it, right-click the app and choose
**Open** once.

## Install (Linux / CLI)

```bash
nix profile install .   # puts `photo-viewer` on PATH
```

For a desktop launcher, add a `~/.local/share/applications/PhotoViewer.desktop`
entry with `Exec=photo-viewer %F`.
