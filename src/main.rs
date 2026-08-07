//! PhotoViewer — Rust + iced rewrite.
//!
//! - Single view: fit-to-window decode, `←/→ h/l` nav, `r`/`Shift+R` rotate
//!   (writes the file to disk). Every action applies to the current image and
//!   nothing else.
//! - Wall view: async 300px thumbnails laid out shortest-column masonry in a
//!   vertical scroll, current image ringed, `←/→/↑/↓ hjkl` to move the ring
//!   (scrolling it into view), Enter or a click to open it, `r`/`Shift+R` to
//!   rotate it on disk, and a modal selection (`v`/`x`/`Space`, or the mouse)
//!   over groups of images, which can then be rotated, favourited (`f`), sent
//!   to another folder (`m`/`c`) or trashed (`d`) all at once. `Shift+F` narrows
//!   the wall to the favourites.
//! - `w` toggles between the two; `o` opens a directory (native picker); a
//!   directory opens in wall view, a file in single view.
//! - Empty view when no image is loaded. `q` quit, `e` fullscreen, `?`/`Esc`
//!   help.
//!
//! Favourites are a tag over a selection (`core/tags.rs`); mark-to-delete is
//! gone for good, since `d` trashes a selection outright. See
//! `SELECT_MODE_PLAN.md`.
//!
//! Three layers, and the rule for each:
//!
//! - [`core`] — photos, folders and files. Imports no `iced`, names no
//!   `Message`. Driven from above; could be driven by anything.
//! - [`screens`] — one folder per screen, each owning its own state, actions
//!   and view. No screen knows about another.
//! - [`app`] — the only place that knows screens exist: which one is live, the
//!   moves between them, and the keyboard and overlays over all of them.
//!
//! This file is the entry point and nothing else.

mod app;
mod core;
mod screens;

use iced::{Size, Theme};

use app::App;
pub(crate) use app::Message;

pub fn main() -> iced::Result {
    // Register the macOS open-file handler before the event loop starts, so a
    // launch-time "Open With" event is caught (no-op off macOS).
    core::platform::install_open_file_handler();

    // 0.14 moved `boot` to the first argument (was `run_with`) and turned the
    // title into a builder method.
    iced::application(App::new, App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .theme(Theme::Dark)
        .window(iced::window::Settings {
            size: Size::new(800.0, 600.0),
            // Start hidden and reveal on the first rendered frame (see
            // `App::subscription` / `Message::WindowReady`) so the window never
            // shows the OS default background before iced paints — this is what
            // removes the white startup flash.
            visible: false,
            ..iced::window::Settings::default()
        })
        .run()
}
