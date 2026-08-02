# PhotoViewer

A keyboard-driven image viewer built with **Rust + [iced](https://iced.rs)**
(Elm architecture). Built and deployed with Nix flakes.

Rewritten from an earlier PySide6/Qt implementation — see
[`RUST_REWRITE_PLAN.md`](RUST_REWRITE_PLAN.md) for the migration history.

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
| `f` | Toggle favourite (un-favouriting also un-deletes) |
| `Cmd+F` | Save favourites: copy every favourited file into a chosen dir |
| `d` | Toggle mark-to-delete (refused if favourited) |
| `Cmd+D` | Delete all marked — confirm, then unlink files from disk |
| `w` | Switch to wall view |

### Wall view

| Key | Action |
|-----|--------|
| `w` | Switch to single view |
| `f` | Toggle "show only favourites" filter |
| `d` | Toggle "show only to-delete" filter |
| click | Go to that image in single view |

Favourites and to-delete marks persist in a `.photo-viewer/` cache directory
alongside the images (newline-separated absolute paths).

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
