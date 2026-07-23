# PhotoViewer

A simple image viewer built with PySide6. Built and deployed with Nix flakes.

## Development

Enter the dev shell (provides Python, PySide6, pytest, etc.):

```bash
nix develop
```

Then:

```bash
python -m lib.main [path/to/image/or/directory]   # run
pytest                                            # test
pytest --cov=lib                                  # test with coverage
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

This makes PhotoViewer available in Spotlight and in Finder's
**Open With** menu, and lets you set it as the default viewer for an image
type via **Get Info > Open with > Change All**.

Double-clicking or "Open With" passes the file to the app via a macOS
`QFileOpenEvent` (handled in `lib/main.py`).

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
