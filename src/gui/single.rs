//! The single view: one fit-to-window image with favourite/delete overlays,
//! navigation, rotate, save-favourites, and the delete-all confirmation.

use std::path::{Path, PathBuf};

use iced::widget::{image, Space, Stack};
use iced::{ContentFit, Element, Length, Task};

use super::corner_icon;
use crate::library::Library;
use crate::Message;

/// Messages produced only while the single view is on screen. Routed to
/// [`SingleState::update`] by `App::update`.
#[derive(Debug, Clone)]
pub(crate) enum SingleMsg {
    Next,
    Prev,
    /// `f`: toggle the current image's favourite flag.
    ToggleFavourite,
    /// `d`: toggle the current image's delete mark.
    ToggleDelete,
    /// `r`: rotate the current image anticlockwise, writing it to disk.
    RotateAnticlockwise,
    /// `Shift+R`: rotate the current image clockwise, writing it to disk.
    RotateClockwise,
    /// Result of a rotate: on success, re-decode the (now rotated) file.
    Rotated { result: Result<(), String> },
    /// `Cmd+F`: pick a directory to copy the favourites into.
    SaveFavourites,
    SaveFavDirPicked(Option<PathBuf>),
    /// A fit-to-window decode landed, tagged with the generation it began at.
    LargeDecoded {
        generation: u64,
        result: Result<image::Handle, String>,
    },
}

/// Single-view state: the library plus the current fit-to-window decode and the
/// delete-all confirmation (only reachable from single view).
pub(crate) struct SingleState {
    pub(crate) library: Library,
    /// Latest fit-to-window decode for the current path. `None` until the first
    /// decode lands (the previous image stays on screen meanwhile).
    large: Option<image::Handle>,
    /// `Some(count)` while the delete-all confirmation overlay is showing.
    pub(crate) confirm_delete: Option<usize>,
}

impl SingleState {
    pub(crate) fn new(library: Library) -> Self {
        Self {
            library,
            large: None,
            confirm_delete: None,
        }
    }

    pub(crate) fn update(&mut self, msg: SingleMsg, generation: &mut u64) -> Task<Message> {
        match msg {
            SingleMsg::Next => {
                self.library.next();
                self.decode_current(generation)
            }
            SingleMsg::Prev => {
                self.library.prev();
                self.decode_current(generation)
            }
            SingleMsg::ToggleFavourite => {
                self.toggle_favourite();
                Task::none()
            }
            SingleMsg::ToggleDelete => {
                self.toggle_delete();
                Task::none()
            }
            SingleMsg::RotateAnticlockwise => self.rotate(false),
            SingleMsg::RotateClockwise => self.rotate(true),
            SingleMsg::Rotated { result } => match result {
                // The file changed on disk: re-decode the current image. (Wall
                // thumbnails are rebuilt on next entry, so none is stale here.)
                Ok(()) => self.decode_current(generation),
                Err(e) => {
                    eprintln!("Rotate failed: {e}");
                    Task::none()
                }
            },
            SingleMsg::SaveFavourites => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Select directory to save favourites")
                        .pick_folder()
                        .await
                        .map(|handle| handle.path().to_path_buf())
                },
                |dir| Message::Single(SingleMsg::SaveFavDirPicked(dir)),
            ),
            SingleMsg::SaveFavDirPicked(Some(dir)) => {
                if let Err(e) = self.library.save_favourites(&dir) {
                    eprintln!("Save favourites failed: {e}");
                }
                Task::none()
            }
            SingleMsg::SaveFavDirPicked(None) => Task::none(),
            SingleMsg::LargeDecoded {
                generation: tagged,
                result,
            } => {
                // Generation gate: drop a decode superseded by a newer request.
                if tagged == *generation {
                    match result {
                        Ok(handle) => self.large = Some(handle),
                        Err(e) => eprintln!("Decode error: {e}"),
                    }
                }
                Task::none()
            }
        }
    }

    /// Kick off an off-thread decode of the current image, tagged with a fresh
    /// generation so an earlier in-flight decode can't overwrite it. Bumps the
    /// caller's global generation counter.
    pub(crate) fn decode_current(&self, generation: &mut u64) -> Task<Message> {
        *generation += 1;
        let generation = *generation;
        let path = self.library.current().clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || decode_large(&path))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::Single(SingleMsg::LargeDecoded { generation, result }),
        )
    }

    /// Rotate the current image 90° (clockwise if `clockwise`, else anti-),
    /// writing the result back to its file off-thread.
    fn rotate(&self, clockwise: bool) -> Task<Message> {
        let path = self.library.current().clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || rotate_file(&path, clockwise))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::Single(SingleMsg::Rotated { result }),
        )
    }

    /// Toggle the current image's favourite flag. Un-favouriting also
    /// un-marks it for deletion. The decoded image is unchanged (overlay only).
    fn toggle_favourite(&mut self) {
        let path = self.library.current().clone();
        let result = if self.library.favourites.contains(&path) {
            self.library
                .unfavourite(&path)
                .and_then(|()| self.library.undelete(&path))
        } else {
            self.library.favourite(&path)
        };
        if let Err(e) = result {
            eprintln!("Favourite toggle failed: {e}");
        }
    }

    /// Toggle the current image's delete mark. A favourite can't be marked.
    fn toggle_delete(&mut self) {
        let path = self.library.current().clone();
        let result = if self.library.to_delete.contains(&path) {
            self.library.undelete(&path)
        } else if self.library.favourites.contains(&path) {
            Ok(()) // refuse: can't delete a favourite
        } else {
            self.library.delete(&path)
        };
        if let Err(e) = result {
            eprintln!("Delete toggle failed: {e}");
        }
    }

    pub(crate) fn view<'a>(
        &'a self,
        star_icon: &'a image::Handle,
        delete_icon: &'a image::Handle,
    ) -> Element<'a, Message> {
        let base: Element<'a, Message> = match &self.large {
            Some(handle) => image(handle.clone())
                .content_fit(ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            None => Space::new()
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        };

        let path = self.library.current();
        let icon = if self.library.favourites.contains(path) {
            Some(star_icon.clone())
        } else if self.library.to_delete.contains(path) {
            Some(delete_icon.clone())
        } else {
            None
        };

        match icon {
            Some(handle) => Stack::with_children(vec![base, corner_icon(handle)]).into(),
            None => base,
        }
    }
}

/// Rotate the image at `path` 90° and overwrite it, preserving its format (the
/// destination extension picks the encoder). `clockwise` matches `Shift+R`; the
/// anticlockwise case matches the Python `Image.rotate(90)`. Runs off-thread.
fn rotate_file(path: &Path, clockwise: bool) -> Result<(), String> {
    let img = ::image::open(path).map_err(|e| e.to_string())?;
    let rotated = if clockwise {
        img.rotate90()
    } else {
        img.rotate270()
    };
    rotated.save(path).map_err(|e| e.to_string())
}

/// Decode an image to full-res RGBA. Runs on a blocking thread; the returned
/// handle is plain data, safe on the GUI thread.
fn decode_large(path: &Path) -> Result<image::Handle, String> {
    let img = ::image::open(path).map_err(|e| e.to_string())?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(image::Handle::from_rgba(width, height, rgba.into_raw()))
}
