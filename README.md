# PhotoViewer

A keyboard-driven image viewer built with **Rust + [iced](https://iced.rs)**
(Elm architecture). Built and deployed with Nix flakes.

Rewritten from an earlier PySide6/Qt implementation.

A modal selection mode — vim-style, for acting on groups of images at once — is
being built; see [`SELECT_MODE_PLAN.md`](SELECT_MODE_PLAN.md). Its first step
removed the old favourites and mark-to-delete features, which will return on
top of the selection machinery.

## Keybindings

### Global

| Key | Action |
|-----|--------|
| `?` | Toggle help overlay |
| `Esc` | Close help |
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
| `i` | Invert the selection |
| `Esc` | Stop a running operation, then cancel the range, then clear |

#### Acting on a selection

Every operation applies to each selected image independently.

| Key | Action |
|-----|--------|
| `r` / `Shift+R` | Rotate every selected image |
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
```

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
`kAEOpenDocuments` Apple Event (handled in `src/platform.rs`), not on argv.

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
