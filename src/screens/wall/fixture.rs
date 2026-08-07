//! Shared test scaffolding for the wall's modules.
//!
//! The state-machine tests all drive a real [`WallState`] through
//! `update`, so they all need the same fixture: a wall of known heights,
//! measured, with the header-read prepass already landed.

use std::collections::HashSet;
use std::path::PathBuf;

use iced::keyboard::Modifiers;
use iced::widget::image;
use iced::Size;

use crate::core::library::{Library, RangeOp};
use crate::core::tags;
use crate::core::transfer::{Collision, TransferKind};

use super::layout::{WallLayout, TILE_WIDTH, WALL_SPACING};
use super::message::{Dir, WallMsg};
use super::mouse::Click;
use super::queue::{BatchKind, FileDone};
use super::thumbs::ThumbState;
use super::WallState;

pub(super) fn paths(n: usize) -> Vec<PathBuf> {
    (0..n).map(|i| PathBuf::from(format!("{i}.jpg"))).collect()
}

/// A wall of thumbnails `THUMB_WIDTH` wide and `heights[i]` tall, sized to
/// fit exactly `col_count` columns. Heights are seeded as if the
/// header-read prepass had already landed.
pub(super) fn wall(heights: &[f32], col_count: usize) -> WallState {
    let files = paths(heights.len());
    let library = Library {
        all: files.clone(),
        paths: crate::core::pointed_list::PointedList::new(files.clone()).unwrap(),
        image_dir: PathBuf::from("/wall"),
        selection: HashSet::new(),
        tags: crate::core::tags::Tags::new(),
        filter: None,
    };
    let mut state = WallState::new(library);
    state.ratios = files.into_iter().zip(heights.iter().copied()).collect();
    state.viewport = Some(Size::new(width_for(col_count), 1000.0));
    state
}

/// The narrowest width that fits exactly `n` columns.
pub(super) fn width_for(n: usize) -> f32 {
    n as f32 * (WALL_SPACING + TILE_WIDTH) + WALL_SPACING
}

/// The selected library indices, sorted — the selection is by path, but
/// indices are what the tests reason in.
pub(super) fn selected(state: &WallState) -> Vec<usize> {
    state
        .library
        .paths
        .iter()
        .enumerate()
        .filter(|(_, p)| state.library.is_selected(p))
        .map(|(i, _)| i)
        .collect()
}

pub(super) fn enter_visual(state: &mut WallState, op: RangeOp) {
    let _ = state.update(WallMsg::EnterVisual { op });
}

pub(super) fn nav(state: &mut WallState, dir: Dir) {
    let _ = state.update(WallMsg::Nav(dir));
}

pub(super) fn rotate(state: &mut WallState) {
    let _ = state.update(WallMsg::Rotate { clockwise: true });
}

/// Land one file of the running batch.
pub(super) fn land(state: &mut WallState, path: PathBuf, result: Result<(), String>) {
    let result = result.map(|()| FileDone::Reshaped);
    let _ = state.update(WallMsg::BatchProgress { path, result });
}

/// Land one file of a transfer batch, saying what became of the source.
pub(super) fn landed(state: &mut WallState, path: PathBuf, done: FileDone) {
    let _ = state.update(WallMsg::BatchProgress {
        path,
        result: Ok(done),
    });
}

pub(super) fn batch_paths(state: &WallState) -> Vec<PathBuf> {
    state
        .batch
        .as_ref()
        .map(|b| b.remaining())
        .unwrap_or_default()
}

pub(super) fn move_kind() -> BatchKind {
    BatchKind::Transfer {
        kind: TransferKind::Move,
        dest: PathBuf::from("/elsewhere"),
        collision: Collision::Skip,
    }
}

pub(super) fn start(state: &mut WallState, kind: BatchKind, paths: Vec<PathBuf>) {
    let _ = state.update(WallMsg::StartBatch { kind, paths });
}

pub(super) fn fav(state: &mut WallState) {
    let _ = state.update(WallMsg::ToggleFavourite);
}

pub(super) fn filter(state: &mut WallState) {
    let _ = state.update(WallMsg::ToggleFilter);
}

pub(super) fn starred(state: &WallState) -> Vec<usize> {
    state
        .library
        .all
        .iter()
        .enumerate()
        .filter(|(_, p)| state.library.is_tagged(tags::FAVOURITE, p))
        .map(|(i, _)| i)
        .collect()
}

pub(super) const PLAIN: Modifiers = Modifiers::NONE;

pub(super) fn click(state: &mut WallState, index: usize, modifiers: Modifiers) -> Click {
    state.click(index, modifiers)
}

pub(super) fn opens(outcome: Click) -> bool {
    matches!(outcome, Click::Open)
}

/// A 1x1 stand-in for a decoded thumbnail of the given height.
pub(super) fn fake_thumb(height: u32) -> ThumbState {
    ThumbState {
        handle: image::Handle::from_rgba(1, 1, vec![0, 0, 0, 255]),
        height,
    }
}

/// Every slot as comparable data, for before/after layout diffs.
pub(super) fn placements(layout: &WallLayout) -> Vec<(usize, usize, usize, f32)> {
    let mut out: Vec<_> = layout
        .slots
        .iter()
        .map(|(i, s)| (*i, s.col, s.row, s.y))
        .collect();
    out.sort_by_key(|p| p.0);
    out
}

/// Heights that lay out as columns `[[0, 3], [1, 5], [2, 4]]` at 3 columns.
pub(super) const SPREAD: [f32; 6] = [200.0, 400.0, 200.0, 300.0, 200.0, 200.0];
