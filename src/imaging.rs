//! Image decoding/rotation, off the GUI thread.
//!
//! All functions here are plain synchronous CPU work run under
//! `spawn_blocking` by the GUI modules; the returned [`image::Handle`] is inert
//! pixel data, safe to hand back to the render thread. Note the leading `::` on
//! `::image`: that is the `image` crate (decoders), as opposed to
//! `iced::widget::image` (the handle type) imported below.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use ::image::{DynamicImage, GenericImageView};
use iced::widget::image;

/// Decode a thumbnail off the GUI thread: runs [`thumbnail`] on the blocking
/// pool. How many of these run at once is bounded by the wall's decode
/// scheduler, which only dispatches up to its in-flight cap.
pub(crate) async fn thumbnail_async(
    path: PathBuf,
    width: u32,
) -> Result<(image::Handle, u32), String> {
    tokio::task::spawn_blocking(move || thumbnail(&path, width))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

/// Decode + downscale to a `width`-wide thumbnail. Returns the handle and its
/// scaled height (for masonry column bookkeeping).
///
/// JPEGs take a scale-on-decode fast path (see [`decode_jpeg_prescaled`]); PNGs
/// (and any JPEG that path can't handle) fall back to a full decode.
pub(crate) fn thumbnail(path: &Path, width: u32) -> Result<(image::Handle, u32), String> {
    let source = load_thumb_source(path, width)?;
    let (w, h) = source.dimensions();
    let target_h = (((h as f32) / (w as f32)) * width as f32).round().max(1.0) as u32;
    let resized = source.resize(width, target_h, ::image::imageops::FilterType::Triangle);
    let height = resized.height();
    Ok((to_handle(resized), height))
}

/// Read every path's on-disk dimensions and return the height each would have
/// as a `width`-wide thumbnail, off the GUI thread.
///
/// Only the file header is parsed — no pixels are decoded — so this is orders
/// of magnitude cheaper than [`thumbnail`] and lands long before the decodes
/// do. The wall uses it to lay its masonry out once instead of reflowing as
/// each thumbnail arrives. Unreadable files are skipped; the wall falls back to
/// a square guess for anything missing.
pub(crate) async fn thumb_heights_async(paths: Vec<PathBuf>, width: u32) -> Vec<(PathBuf, f32)> {
    tokio::task::spawn_blocking(move || {
        paths
            .into_iter()
            .filter_map(|path| {
                let height = thumb_height(&path, width)?;
                Some((path, height))
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// The height a `width`-wide thumbnail of `path` will have, from its header
/// alone. Deliberately mirrors [`thumbnail`]'s arithmetic, so the masonry
/// doesn't shift when the real decode lands. `None` if the header is unreadable.
fn thumb_height(path: &Path, width: u32) -> Option<f32> {
    let (w, h) = ::image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    if w == 0 {
        return None;
    }
    Some((((h as f32) / (w as f32)) * width as f32).round().max(1.0))
}

/// Decode an image to full-res RGBA (single view).
pub(crate) fn full(path: &Path) -> Result<image::Handle, String> {
    let img = ::image::open(path).map_err(|e| e.to_string())?;
    Ok(to_handle(img))
}

/// Rotate the image at `path` 90° and overwrite it, preserving its format (the
/// destination extension picks the encoder). `clockwise` matches `Shift+R`; the
/// anticlockwise case matches the Python `Image.rotate(90)`.
pub(crate) fn rotate_in_place(path: &Path, clockwise: bool) -> Result<(), String> {
    let img = ::image::open(path).map_err(|e| e.to_string())?;
    let rotated = if clockwise {
        img.rotate90()
    } else {
        img.rotate270()
    };
    rotated.save(path).map_err(|e| e.to_string())
}

/// Pick a decode source for a `width`-wide thumbnail: the JPEG fast path when it
/// applies, otherwise a full `image::open`. Any failure of the fast path (odd
/// pixel format, decode error) silently falls back to the generic decode.
fn load_thumb_source(path: &Path, width: u32) -> Result<DynamicImage, String> {
    if is_jpeg(path) {
        if let Ok(Some(img)) = decode_jpeg_prescaled(path, width) {
            return Ok(img);
        }
    }
    ::image::open(path).map_err(|e| e.to_string())
}

/// Decode a JPEG at the smallest built-in scale (1/8, 1/4, 1/2, 1) that still
/// covers `width`, doing the IDCT at that reduced size — most of a full-res
/// decode is skipped. Returns `Ok(None)` for pixel formats we don't unpack
/// here (L16, CMYK32), so the caller can fall back.
fn decode_jpeg_prescaled(path: &Path, width: u32) -> Result<Option<DynamicImage>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = jpeg_decoder::Decoder::new(BufReader::new(file));
    // Request a square `width` box; scale() keeps >= width on at least one axis.
    decoder
        .scale(width as u16, width as u16)
        .map_err(|e| e.to_string())?;
    let info = decoder.info().ok_or("jpeg: missing image info")?;
    let pixels = decoder.decode().map_err(|e| e.to_string())?;
    let (w, h) = (info.width as u32, info.height as u32);

    let dynamic = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            ::image::RgbImage::from_raw(w, h, pixels).map(DynamicImage::ImageRgb8)
        }
        jpeg_decoder::PixelFormat::L8 => {
            ::image::GrayImage::from_raw(w, h, pixels).map(DynamicImage::ImageLuma8)
        }
        // L16 / CMYK32: rare; let the caller fall back to a generic decode.
        _ => None,
    };
    Ok(dynamic)
}

/// Flatten any decoded image to an RGBA handle for iced.
fn to_handle(img: DynamicImage) -> image::Handle {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    image::Handle::from_rgba(w, h, rgba.into_raw())
}

fn is_jpeg(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a `w`x`h` gradient image to a temp file with the given extension.
    fn write_test_image(w: u32, h: u32, ext: &str) -> std::path::PathBuf {
        let mut buf = ::image::RgbImage::new(w, h);
        for (x, y, px) in buf.enumerate_pixels_mut() {
            *px = ::image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]);
        }
        let path = std::env::temp_dir().join(format!("pv-test-{w}x{h}.{ext}"));
        DynamicImage::ImageRgb8(buf).save(&path).unwrap();
        path
    }

    #[test]
    fn thumbnail_scales_jpeg_to_width_preserving_aspect() {
        let path = write_test_image(4000, 3000, "jpg");
        let (handle, height) = thumbnail(&path, 300).unwrap();
        assert_eq!(height, 225); // 300 * 3000/4000
        match handle {
            image::Handle::Rgba { width, height, .. } => {
                assert_eq!(width, 300);
                assert_eq!(height, 225);
            }
            _ => panic!("expected an rgba handle"),
        }
    }

    #[test]
    fn thumb_height_matches_the_decoded_thumbnail() {
        // The wall lays out from the header-derived height and must not shift
        // when the decode replaces it, so the two have to agree exactly.
        let path = write_test_image(4000, 3000, "jpg");
        let (_handle, decoded) = thumbnail(&path, 300).unwrap();
        assert_eq!(thumb_height(&path, 300), Some(decoded as f32));
        assert_eq!(thumb_height(&path, 300), Some(225.0));
    }

    #[test]
    fn rotate_in_place_swaps_the_dimensions() {
        // A size no other test writes: this one mutates its fixture.
        let path = write_test_image(640, 480, "jpg");
        rotate_in_place(&path, true).unwrap();
        assert_eq!(thumb_height(&path, 300), Some(400.0)); // 300 * 640/480
        // Back to the original orientation, and the thumbnail with it.
        rotate_in_place(&path, false).unwrap();
        assert_eq!(thumb_height(&path, 300), Some(225.0)); // 300 * 480/640
    }

    #[test]
    fn thumb_height_is_none_for_unreadable_files() {
        assert_eq!(thumb_height(Path::new("/nonexistent.jpg"), 300), None);
    }

    #[test]
    fn thumbnail_scales_png_via_fallback() {
        let path = write_test_image(4000, 3000, "png");
        let (_handle, height) = thumbnail(&path, 300).unwrap();
        assert_eq!(height, 225);
    }

    #[test]
    fn full_keeps_source_resolution() {
        let path = write_test_image(800, 600, "jpg");
        let handle = full(&path).unwrap();
        match handle {
            image::Handle::Rgba { width, height, .. } => {
                assert_eq!((width, height), (800, 600));
            }
            _ => panic!("expected an rgba handle"),
        }
    }
}
