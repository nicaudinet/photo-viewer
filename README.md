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

### Wall view

| Key | Action |
|-----|--------|
| `←` / `h` | Move the selection left |
| `→` / `l` | Move the selection right |
| `↑` / `k` | Move the selection up a row |
| `↓` / `j` | Move the selection down a row |
| `Enter` | Open the selected image in single view |
| `r` | Rotate the selected image anticlockwise 90° |
| `Shift+R` | Rotate the selected image clockwise 90° |
| `w` | Switch to single view |
| click | Go to that image in single view |

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
