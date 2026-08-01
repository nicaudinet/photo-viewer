# PhotoViewer — Rust + iced Rewrite Plan

Living doc. Rewrite the existing PySide6/Qt app as **Rust + iced (Elm architecture)**,
packaged with a **Nix flake (rust-overlay + crane)**. Update checkboxes as work lands.

Motivation: escape Qt's mutable-widget footguns (off-thread QPixmap, stale relayout,
worker GC, QTimer sequencing). iced makes `view = f(state)` pure and typed; message enum
+ exhaustive `match` removes whole crash classes seen in recent git history.

---

## 1. Feature inventory (parity target — from the Python app)

### Screens (`lib/view/*`)
- **LoadingView** — shown while a dir/file is being scanned+decoded. Indicator revealed
  only after 250ms (no flash on fast loads). Empty fallback after 200ms if nothing opened.
- **EmptyView** — "No image loaded / Press ? for help", dashed border box.
- **SingleView** — one large image, fit-to-window (`LargePhoto`), decoded off-thread,
  re-decoded on resize with generation tag to drop stale results.
- **WallView** — masonry (Pinterest columns) of async thumbnails in a vertical scroll area.
  Current image highlighted with border. Click a thumbnail → jump to it in single view.

### Global commands (`PhotoViewer`)
| Key | Action |
|-----|--------|
| `?` | Toggle help overlay |
| `Esc` | Close help |
| `q` | Quit |
| `e` | Toggle fullscreen |
| `o` | Open directory (native dir picker) |

### SingleView commands
| Key | Action |
|-----|--------|
| `←` / `h` | Previous image (circular) |
| `→` / `l` | Next image (circular) |
| `r` | Rotate anticlockwise 90° — **writes file to disk** |
| `Shift+R` | Rotate clockwise 90° — **writes file to disk** |
| `f` | Toggle favourite (un-favouriting also un-deletes) |
| `Ctrl+F` | Save favourites: copy all favourited files into a chosen dir |
| `d` | Toggle mark-to-delete (refused if favourited) |
| `Ctrl+D` | Delete-all-marked — confirm dialog, then **unlinks files from disk** |
| `w` | Switch to wall view |

### WallView commands
| Key | Action |
|-----|--------|
| `w` | Switch to single view |
| `f` | Toggle "show only favourites" filter |
| `d` | Toggle "show only to-delete" filter |
| click | Go to that image in single view |

### Domain / state (`lib/state.py`, `lib/pointed_list.py`)
- **PointedList<T>**: circular list + cursor. `current/next/prev/goto/goto_value/delete`.
- **ImageState**: `image_paths: PointedList<Path>`, `favourites: Set<Path>`,
  `to_delete: Set<Path>`, plus dir paths.
- Scan: `IMAGE_EXTENSIONS = .png .jpg .jpeg` (case-insensitive), sorted.
- **Persistence** (must stay format-compatible with existing caches):
  - Cache dir: `<image_dir>/.photo-viewer/`
  - `favourites` and `to_delete` files: newline-separated absolute paths.
  - On load, entries not in the current dir's images are dropped (with a log).
- `delete_all`: walks list, unlinks files in `to_delete`, removes from list, saves.
- `save_favourites(dir)`: `shutil.copy2` each favourite into dir, skip existing.

### Photo widgets (`lib/photo.py`)
- Icon overlays top-right: **star** (favourite), **delete** (to-delete). Icons in `icons/`.
- Thumbnail width 300px; true aspect ratio adopted after decode. Masonry spacing 20px,
  shortest-column placement, centered columns, ≥1 column floor.
- Selected thumbnail: 4px highlight border.

### Platform glue (`lib/main.py`, `flake.nix`)
- **macOS "Open With" / double-click**: file arrives as `QFileOpenEvent`, possibly before
  the window exists → buffered. **Hardest parity item in iced (see Phase 6).**
- **Linux**: force Wayland backend + scale factor.
- **Nix**: cross-platform CLI, macOS `.app` bundle (`Info.plist` w/ `CFBundleDocumentTypes`
  so Finder offers "Open With" + default-viewer), `.icns` from `icons/camera.png`,
  `install-app` copies bundle into `~/Applications`.

---

## 2. Target architecture (iced / Elm)

Single `App` = the whole model. One `Message` enum. `update` + `view` are total functions.

```
enum Screen { Loading, Empty, Single, Wall }

struct App {
    screen: Screen,
    library: Option<Library>,   // None until a dir is loaded
    help_open: bool,
    fullscreen: bool,
    // decode caches (keyed by path); generation counter for the large image
    large: Option<LargeImage>,          // current fit-to-window decode
    thumbs: HashMap<PathBuf, ThumbState>,
    wall_filter: WallFilter,            // All | FavouritesOnly | ToDeleteOnly
}

// Pure domain, no iced types — lives in its own module, unit-tested.
struct Library {
    paths: PointedList<PathBuf>,
    favourites: HashSet<PathBuf>,
    to_delete: HashSet<PathBuf>,
    image_dir: PathBuf,
    cache_dir: PathBuf,
}
```

`Message` (sketch): `KeyPressed{key,mods}`, `ThumbDecoded{path, result}`,
`LargeDecoded{generation, result}`, `Resized`, `OpenDirPicked(Option<PathBuf>)`,
`SaveFavDirPicked(...)`, `ConfirmDelete(bool)`, `Tick`, window events.

### Mapping the Qt footguns → iced idioms
- Off-thread QPixmap crash → **gone**: decode returns plain RGBA `Vec<u8>`; the GUI thread
  builds `image::Handle::from_rgba`. Decoding is a `Task`, not a QRunnable.
- Stale-resize generation tags → keep a `generation: u64`; drop `LargeDecoded` whose gen ≠ current.
  (Or lean on `Task` cancellation; tag is simpler and explicit.)
- Worker GC ("Signal source deleted") → **gone**: `Task::perform` owns the future.
- QTimer relayout coalescing → **gone**: view is recomputed from state each frame.

### Known hard spots (research as reached)
- **Masonry**: iced has no masonry widget. Use `responsive` to get width → compute column
  count → distribute thumbnails round-robin into N `column`s inside a `scrollable`. This is
  the standard iced masonry workaround; not pixel-identical to shortest-column but close.
  Decide shortest-column vs round-robin in Phase 4.
- **macOS Open-With event**: winit/iced don't surface `application:openFile:` cleanly.
  Options to spike: (a) custom winit event via `iced::event::listen_raw`; (b) small Obj-C
  `NSApplicationDelegate` shim feeding a channel `subscription`. Fallback: argv only + a
  bundle wrapper that passes `%F`. Track parity honestly.
- **Delete confirm dialog**: iced modal = overlay state (`confirm_delete: Option<usize>`),
  or use `rfd::MessageDialog`. Prefer in-app overlay for keyboard control.

### Crates
- `iced` — pin latest (0.14.x); features for image + tokio + wgpu.
- `image` — decode/encode png+jpeg, rotate90/270, save.
- `fast_image_resize` (or `image`'s resizer) — thumbnail + fit-to-window scaling.
- `rfd` — native file/dir pickers.
- `tokio` (`spawn_blocking`) or `rayon` — CPU-bound decode off the UI runtime.
- (later) `kamadak-exif` — EXIF orientation (NOT in Python app; parity = skip; note as improvement).

---

## 3. Nix flake plan

Replace the Python flake. Keep the same UX: `nix run`, `nix build .#app`,
`nix run .#install-app`, `nix develop`.

- Inputs: `nixpkgs`, `rust-overlay`, `crane`, (opt) `flake-utils`.
- **devShell**: toolchain from `rust-bin.stable.latest.default` + `rust-analyzer`, `pkg-config`.
  - Linux `buildInputs`: `wayland libxkbcommon vulkan-loader libGL fontconfig freetype`
    + `xorg.{libX11,libXcursor,libXrandr,libXi}`.
  - **Runtime trap**: set `LD_LIBRARY_PATH` via `makeLibraryPath [vulkan-loader libGL wayland libxkbcommon]`
    or iced panics at launch ("no suitable graphics adapter").
  - macOS: Apple frameworks pulled by wgpu; usually just needs the toolchain.
- **Package** (`crane.buildPackage`): the `photo-viewer` binary. Embed `icons/` via
  `include_bytes!` so there's no runtime path lookup (kills the `__file__`-relative fragility).
- **macOS `.app`**: reuse the existing `mkDerivation` bundle almost verbatim — same
  `Info.plist` (bundle id `com.nicaudinet.photo-viewer`, `CFBundleDocumentTypes`), same
  `png2icns` from `icons/camera.png`; just `makeWrapper` the Rust binary instead of the
  Python entrypoint. `install-app` script unchanged.

---

## 4. Phased execution

- [x] **Phase 0 — Scaffold.** `Cargo.toml` + `src/main.rs` (empty iced 0.13 window, dark
      theme, 800×600). New `flake.nix`: rust-overlay + crane, `packages.viewer`/`default`,
      `apps.default` → viewer, rust `devShells.default` (+ rust-analyzer/clippy), legacy
      Python kept as `packages.photo-viewer-py` / `devShells.python`. Verified: `cargo build`
      OK (iced 0.13.1), binary launches a window without panic, flake evaluates.
      **Notes:** iced pinned to **0.13.1** (0.14.0 available — bump later). Nix requires files
      to be **git-tracked/staged** or crane can't see `Cargo.toml`. Icons not yet embedded.
- [x] **Phase 1 — Pure domain core.** `src/pointed_list.rs` (`PointedList<T>`) + `src/library.rs`
      (`Library` = ImageState, `load_library`, cache persistence, dir scan), GUI-free. 50 Rust
      `#[test]`s ported from `test_pointed_list` + `test_state`, all green; clippy clean.
      Cache stays byte-compatible with Python (`.photo-viewer/{favourites,to_delete}`,
      newline-joined absolute paths, no trailing NL) — covered by a save→load round-trip test.
      **Notes:** typed instead of Python asserts — `new` returns `Option` (empty list → None),
      `goto` returns `bool`, mutations return `Result<_, LibraryError>` (`NotInLibrary` etc.).
      Save sorts entries for deterministic output (order-irrelevant to the format). `test_command`
      is Qt key-display (view layer) → deferred to a later phase, not domain. Modules wired into
      `main.rs` under `#[allow(dead_code)]` until Phase 2 consumes them.
- [x] **Phase 2 — Single view MVP.** `src/main.rs` now a real Elm app: argv path opened
      via `App::open` (file → parent dir + `goto_value`; dir → single view; no images → empty
      view), fit-to-window `image(Handle)` with `ContentFit::Contain`, `←/→ h/l` nav, `q` quit,
      `e` fullscreen (`window::change_mode`), `?`/`Esc` help overlay (via `stack!`). Decode runs
      on `tokio::spawn_blocking` inside a `Task::perform`, returns an RGBA `image::Handle`
      (plain data, safe off-thread — the Qt QPixmap footgun is gone), tagged with a `generation`
      counter so a superseded nav's `LargeDecoded` is dropped. Keys via `keyboard::on_key_press`
      subscription. Added deps: `image` 0.25 (png+jpeg), `tokio` (rt). Build + clippy clean,
      50 domain tests green, smoke-ran against `icons/` (window opens, decodes, no panic).
      **Notes:** Python opens a *directory* in the wall view; wall view is Phase 4, so for now a
      dir opens single view. No LoadingView yet (shows "Loading…" text while first decode runs);
      real LoadingView timing is Phase 5. `o` open-dir picker deferred to Phase 5 (needs `rfd`).
- [ ] **Phase 3 — Favourite / delete.** Toggle `f`/`d`, star+delete icon overlays, persistence,
      `Ctrl+D` delete-all w/ in-app confirm overlay, `Ctrl+F` save-favourites via `rfd` dir picker.
- [ ] **Phase 4 — Wall view.** Async thumbnails, masonry columns in `scrollable`, selection
      highlight, click→single, `f`/`d` filter toggles, `w` swap both ways.
- [ ] **Phase 5 — Rotate + open + empty/loading.** `r`/`Shift+R` rotate+save to disk,
      `o` open-dir picker, EmptyView, LoadingView with delayed indicator + fallback timing.
- [ ] **Phase 6 — Platform + packaging.** macOS Open-With spike (see hard spots), Linux
      Wayland/HiDPI, `.app` bundle + `install-app` parity, embed icons.
- [ ] **Phase 7 — Cutover.** Delete `lib/` + Python flake bits, rewrite README, final test pass,
      confirm all keybindings match the table above.

---

## 5. Parity checklist (verify before deleting Python)

- [ ] All keybindings in §1 behave identically (incl. `h/l` aliases, Shift/Ctrl variants).
- [x] Cache format round-trips with existing `.photo-viewer/favourites` + `to_delete`.
      (Phase 1 `save_then_load_round_trips` test; same newline-joined absolute-path format.)
- [ ] Un-favourite also un-deletes; can't delete a favourite.
- [ ] Rotate writes back to the source file; delete-all unlinks from disk.
- [ ] Save-favourites copies (not moves), skips existing.
- [ ] Circular nav wraps at both ends.
- [ ] macOS double-click / "Open With" opens the file (or documented fallback).
- [ ] `nix run`, `nix build .#app`, `nix run .#install-app`, `nix develop` all work.

## 6. Open decisions (resolve as encountered)
- iced version pin (0.14.x) — confirm masonry/`responsive` API shape on that version.
- Masonry: shortest-column (match Qt) vs round-robin (simpler). Default round-robin unless it looks bad.
- macOS Open-With: which of the three approaches actually works.
- Decode pool: tokio `spawn_blocking` vs rayon bridged to iced `Task`.
