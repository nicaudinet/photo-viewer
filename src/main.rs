//! PhotoViewer — Rust + iced rewrite.
//!
//! Phase 0: just an empty window that opens. The roadmap (feature parity with
//! the outgoing PySide6 app, phased migration) lives in `RUST_REWRITE_PLAN.md`.

use iced::widget::{center, column, text};
use iced::{Element, Size, Theme};

pub fn main() -> iced::Result {
    iced::application("Photo Viewer", App::update, App::view)
        .theme(|_app| Theme::Dark)
        .window_size(Size::new(800.0, 600.0))
        .run()
}

#[derive(Default)]
struct App;

/// No interactions wired up yet — Phase 2 onward fills this in.
#[derive(Debug, Clone)]
enum Message {}

impl App {
    fn update(&mut self, message: Message) {
        match message {}
    }

    fn view(&self) -> Element<'_, Message> {
        center(
            column![
                text("Photo Viewer").size(32),
                text("Rust + iced — Phase 0 scaffold").size(16),
            ]
            .spacing(12)
            .align_x(iced::Center),
        )
        .into()
    }
}
