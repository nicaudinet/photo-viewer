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
- [x] **Phase 3 — Favourite / delete.** `f` toggles favourite (un-favouriting also un-deletes),
      `d` toggles mark-to-delete (refused on a favourite) — both persist via the Phase-1 domain.
      Star/delete icons (`icons/{star,delete}.png`, embedded via `include_bytes!`) overlaid
      top-right with a `Stack`, star winning. `Cmd+D` opens an in-app confirm overlay (modal:
      `update` swallows all but confirm/cancel/quit while it's up; `y`/Enter accept, `n`/Esc
      cancel), then `delete_all` unlinks from disk; if the whole library is deleted it drops to
      the empty view. `Cmd+F` runs a native dir picker (`rfd::AsyncFileDialog` in a `Task`) then
      `save_favourites`. Added dep: `rfd` 0.15. Build + clippy clean, 50 tests green, smoke-ran.
      **Notes:** modifiers read via `modifiers.command()` = Cmd on macOS, matching the Python app
      (Qt maps Cmd→ControlModifier on mac). Favourite/delete toggles don't re-decode — the
      overlay is derived from library state each frame. `rfd` async picker marshals to the main
      thread itself; works from an iced `Task`.
- [x] **Phase 4 — Wall view.** Async 300px thumbnails (`decode_thumb`, cached in a
      `HashMap<PathBuf, ThumbState>` keyed by path, decoded once via a `Task::batch`), laid out
      **shortest-column masonry** (chose this over round-robin — we have each thumb's scaled
      height, so it matches the Python wall) inside a vertical `scrollable`, columns centered,
      ≥1 column floor, 20px spacing. Current image gets a 4px highlight border (a `button` style
      capturing `selected`). Click a thumbnail → `goto(index)` + switch to single view. `f`/`d`
      toggle the favourites / to-delete filter (icons hidden while a filter is active); `w` swaps
      both ways. A directory now opens in wall view, a file in single view (the Phase-2 deferral).
      Layout uses the `responsive` widget (added iced `lazy` feature). Build + clippy clean, 50
      tests green, smoke-ran (dir → wall, thumbnails decode, no panic).
      **Notes:** `f`/`d` are context-dependent, but `on_key_press`'s closure is `'static` and
      can't see `self`, so they emit neutral `KeyF`/`KeyD` messages and `update` dispatches on
      `self.screen`. `WallFilter` is a 3-state enum (`All`/`Favourites`/`ToDelete`); re-pressing
      the active filter returns to `All`. Nav + `Cmd+F`/`Cmd+D` are gated to single view. Masonry
      recomputes each frame from state, so it self-heals as async thumbnails arrive.
- [x] **Phase 5 — Rotate + open + empty/loading.** `r` rotates the current image
      anticlockwise, `Shift+R` clockwise, each re-encoding the file in place off-thread
      (`rotate_file`: `image::open` → `rotate270`/`rotate90` → `save`, format from the
      extension, matching PIL's lossy re-encode); on success the stale thumbnail is dropped
      and the current image re-decoded. `o` runs a native dir picker (`rfd`) then `open`.
      `Screen` gained `Loading`/`Empty` (view now switches on `screen`, not on `library`
      being `None`). Startup with no path sits on a **quiet** loading view: a 250ms timer
      (`INDICATOR_DELAY_MS`) reveals the "Loading …" indicator, a 200ms timer
      (`EMPTY_FALLBACK_MS`) falls back to the empty view — so a fast/absent load flashes
      nothing. Both timers, plus the single-view decode indicator, are generation-tagged so
      a superseded load can't reveal or clobber. Build + clippy clean, 50 tests green,
      smoke-ran (wall opens, no panic).
      **Notes:** `Shift+R` matched both `Character("R")` and `Character("r")` + shift, since
      layouts differ on which they deliver. Delay timers are one-shot `tokio::time::sleep`
      `Task`s (added tokio `time` feature) — iced already drives that runtime. The macOS
      open-event that would feed the empty-fallback window is Phase 6; for now the fallback
      just reaches the empty view after argv-less launch.
- [x] **Phase 6 — Platform + packaging.** macOS "Open With" / double-click: Finder delivers
      the file as a `kAEOpenDocuments` Apple Event, not on argv, so `src/platform.rs` (macOS
      only) registers a handler on the shared `NSAppleEventManager` via objc2 (`define_class!`
      for `PVOpenFileHandler`), walks the event's direct-object list, coerces each item to
      `typeFileURL`, and decodes it back to a path through `NSURL` (handles percent-decoding).
      Opened paths land in a `Mutex<Vec<PathBuf>>` queue that the app drains on a 200ms timer
      subscription (`Message::PollOpenFiles` → `open`), mirroring the Python `_pending_path`
      buffer that also covers events arriving before the window exists. `.app` bundle +
      `install-app` now wrap the Rust `viewer` binary (same `Info.plist` /
      `CFBundleDocumentTypes` / `png2icns`); icons already embedded via `include_bytes!`
      (Phase 3). Build + clippy clean, `nix build .#app .#viewer` green, 51 tests
      (the new one builds a real `odoc` event and asserts the extracted path round-trips a
      space through percent-decode).
      **Notes:** the Apple Event *parsing* is unit-tested in-process, but OS *delivery*
      (LaunchServices routing a Finder double-click / `open -a`) can't be tested headlessly
      here — needs a manual `open -a` / Finder check after `nix run .#install-app`.
      objc2-core-services feature pulled in for the `AEEventClass`/`AEKeyword` types + the
      gated `setEventHandler` API. Linux Wayland/HiDPI needs no code: the Python `QT_*` env
      hacks are Qt-specific; winit auto-detects Wayland + HiDPI, and the runtime graphics
      libs are already handled by the flake (`LD_LIBRARY_PATH` in the devShell, `patchelf`
      rpath in the package).
- [x] **Phase 7 — Cutover.** Deleted `lib/` + `tests/*.py` + `pyproject.toml` / `requirements.txt`
      / `pyrightconfig.json`; stripped the Python package (`photo-viewer-py`) and `.#python`
      devShell from `flake.nix`; cleaned Python entries out of `.gitignore`. README rewritten for
      the Rust app (keybinding tables + Rust/Nix workflow). Final pass: 51 tests green, clippy
      clean, `nix eval` shows outputs are just `app`/`default`/`install-app`/`viewer` +
      `devShells.default`. Every key in §1 verified against `src/main.rs` (`h/l` aliases,
      `Shift+R` dual-match, `Cmd+F`/`Cmd+D`, `o`, `?`/`Esc`).
      **Notes:** kept `RUST_REWRITE_PLAN.md` as migration history. Only manual item still open is
      OS-level macOS Open-With delivery (parsing is unit-tested; needs an `open -a` / Finder check).

---

## 5. Parity checklist (verify before deleting Python)

- [x] All keybindings in §1 behave identically (incl. `h/l` aliases, Shift/Ctrl variants).
      (Phase 7 — verified against `src/main.rs` key mapping.)
- [x] Cache format round-trips with existing `.photo-viewer/favourites` + `to_delete`.
      (Phase 1 `save_then_load_round_trips` test; same newline-joined absolute-path format.)
- [x] Un-favourite also un-deletes; can't delete a favourite. (Phase 3 `toggle_favourite`/`toggle_delete`.)
- [x] Rotate writes back to the source file; delete-all unlinks from disk. (delete-all Phase 3; rotate Phase 5 `rotate_file`.)
- [x] Save-favourites copies (not moves), skips existing. (Phase 1 domain; wired to `Cmd+F` in Phase 3.)
- [x] Circular nav wraps at both ends. (Phase 1 `PointedList` next/prev wrap tests.)
- [~] macOS double-click / "Open With" opens the file. (Apple Event handler wired +
      parsing unit-tested in Phase 6; OS delivery pending a manual `open -a` / Finder check.)
- [x] `nix run`, `nix build .#app`, `nix run .#install-app`, `nix develop` all work.
      (`.app` + `install-app` wrap the Rust binary as of Phase 6.)

## 6. Open decisions (resolve as encountered)
- iced version pin (0.14.x) — confirm masonry/`responsive` API shape on that version.
- Masonry: shortest-column (match Qt) vs round-robin (simpler). Default round-robin unless it looks bad.
- macOS Open-With: which of the three approaches actually works.
- Decode pool: tokio `spawn_blocking` vs rayon bridged to iced `Task`.
