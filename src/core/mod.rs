//! Everything the app knows that has nothing to do with being on screen.
//!
//! Nothing in here imports `iced` or names a `Message`: these modules are about
//! photos, folders and files. They are driven by the screens above them, and
//! could be driven by anything else.
//!
//! The one crack in that rule is [`imaging`], which returns iced's
//! `image::Handle` — the decoders produce RGBA and something has to name the
//! type that carries it.

pub mod exif;
pub mod imaging;
pub mod library;
pub mod platform;
// `PointedList` keeps a complete list API (`delete`, `contains`, …) that the
// selection phases will consume; allow the still-unused surface for now.
#[allow(dead_code)]
pub mod pointed_list;
pub mod tags;
pub mod transfer;
