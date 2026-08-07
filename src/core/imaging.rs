//! Image decoding/rotation, off the GUI thread.
//!
//! All functions here are plain synchronous CPU work run under
//! `spawn_blocking` by the GUI modules; the returned [`image::Handle`] is inert
//! pixel data, safe to hand back to the render thread. Note the leading `::` on
//! `::image`: that is the `image` crate (decoders), as opposed to
//! `iced::widget::image` (the handle type) imported below.
//!
//! ## Orientation
//!
//! JPEGs are rotated by rewriting one EXIF tag rather than by turning pixels
//! (see [`crate::core::exif`]), so every path that reads an image has to apply that
//! tag — including [`thumb_height`], which reads no pixels at all: orientations
//! 5 to 8 swap width and height, and the wall's masonry is laid out from the
//! height it reports.
//!
//! [`thumb_height`] and [`thumbnail`] must agree exactly, or the wall shifts
//! when a decode lands, so both derive their height from the same oriented
//! dimensions.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use ::image::metadata::Orientation;
use ::image::{DynamicImage, GenericImageView, ImageDecoder, ImageReader};
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
    let source = oriented_source(path, width)?;
    let (w, h) = source.dimensions();
    let target_h = scaled_height(w, h, width);
    let resized = source.resize(width, target_h, ::image::imageops::FilterType::Triangle);
    let height = resized.height();
    Ok((to_handle(resized), height))
}

/// The height of a `width`-wide thumbnail of a `w` x `h` image.
///
/// The single definition both [`thumbnail`] and [`thumb_height`] use — they
/// have to agree to the pixel or the masonry shifts under the user when a
/// decode replaces a header-derived guess.
fn scaled_height(w: u32, h: u32, width: u32) -> u32 {
    (((h as f32) / (w as f32)) * width as f32).round().max(1.0) as u32
}

/// Whether this orientation is a quarter turn, and so shows the image's stored
/// height as its width.
fn swaps_axes(orientation: Orientation) -> bool {
    matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}

/// The image's EXIF orientation, or `NoTransforms` if it has none or the file
/// can't be read. Reads metadata only — no pixels are decoded.
fn orientation(path: &Path) -> Orientation {
    let Ok(reader) = ImageReader::open(path) else {
        return Orientation::NoTransforms;
    };
    let Ok(reader) = reader.with_guessed_format() else {
        return Orientation::NoTransforms;
    };
    match reader.into_decoder() {
        Ok(mut decoder) => decoder.orientation().unwrap_or(Orientation::NoTransforms),
        Err(_) => Orientation::NoTransforms,
    }
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

/// Decode `path` at no less than `width` across, turned the way up it is shown.
///
/// The shared front half of [`thumbnail`], also used to build a fingerprint (see
/// [`crate::core::fingerprint`]). Applying the orientation here rather than at
/// each call site is what makes a photo hash the same before and after it is
/// rotated: rotation rewrites one EXIF tag, so a hasher reading stored pixels
/// would decide a photo had stopped resembling the ones beside it.
pub(crate) fn oriented_source(path: &Path, width: u32) -> Result<DynamicImage, String> {
    let orientation = orientation(path);
    let mut source = load_thumb_source(path, width, orientation)?;
    // Before measuring: a quarter-turn swaps the dimensions, and the thumbnail
    // is sized from the dimensions the viewer will actually see.
    source.apply_orientation(orientation);
    Ok(source)
}

/// The height a `width`-wide thumbnail of `path` will have, from its header
/// alone. Deliberately mirrors [`thumbnail`]'s arithmetic, so the masonry
/// doesn't shift when the real decode lands. `None` if the header is unreadable.
fn thumb_height(path: &Path, width: u32) -> Option<f32> {
    let mut decoder = ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?;
    // Stored dimensions, which for a quarter-turned photo are the other way
    // round from the ones it is displayed at.
    let (stored_w, stored_h) = decoder.dimensions();
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let (w, h) = match orientation {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270FlipH => (stored_h, stored_w),
        _ => (stored_w, stored_h),
    };
    if w == 0 {
        return None;
    }
    Some(scaled_height(w, h, width) as f32)
}

/// Decode an image to full-res RGBA (single view).
pub(crate) fn full(path: &Path) -> Result<image::Handle, String> {
    let mut decoder = ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_decoder()
        .map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    Ok(to_handle(img))
}

/// Rotate the image at `path` 90°. `clockwise` matches `Shift+R`; the
/// anticlockwise case matches the Python `Image.rotate(90)`.
///
/// JPEGs are turned by rewriting their EXIF orientation, which touches no
/// pixels: a re-encode would throw away a little of the photo on every press,
/// and this is the only way a rotation can be repeated freely. PNG has no such
/// tag, but its codec is lossless, so turning the pixels there costs nothing
/// but time.
pub(crate) fn rotate_in_place(path: &Path, clockwise: bool) -> Result<(), String> {
    if is_jpeg(path) {
        let current = orientation(path).to_exif();
        let turned = crate::core::exif::rotate_orientation(current, clockwise);
        return crate::core::exif::write_orientation(path, turned);
    }

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
fn load_thumb_source(
    path: &Path,
    width: u32,
    orientation: Orientation,
) -> Result<DynamicImage, String> {
    if is_jpeg(path) {
        if let Ok(Some(img)) = decode_jpeg_prescaled(path, width, swaps_axes(orientation)) {
            return Ok(img);
        }
    }
    ::image::open(path).map_err(|e| e.to_string())
}

/// Decode a JPEG at the smallest built-in scale (1/8, 1/4, 1/2, 1) that still
/// covers `width`, doing the IDCT at that reduced size — most of a full-res
/// decode is skipped. Returns `Ok(None)` for pixel formats we don't unpack
/// here (L16, CMYK32), so the caller can fall back.
///
/// `swap_axes` says the file is stored a quarter turn from how it is shown, so
/// the stored axis to hold to `width` is the height.
fn decode_jpeg_prescaled(
    path: &Path,
    width: u32,
    swap_axes: bool,
) -> Result<Option<DynamicImage>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = jpeg_decoder::Decoder::new(BufReader::new(file));
    // `scale` takes the smallest reduction where *either* axis still covers the
    // request, so a square box lets a tall image be chosen on its height with
    // its width left short of `width` — and the thumbnail is then upscaled from
    // it. Constrain the one axis that becomes the displayed width, and leave
    // the other unsatisfiable so it cannot decide the scale.
    let (request_w, request_h) = if swap_axes {
        (u16::MAX, width as u16)
    } else {
        (width as u16, u16::MAX)
    };
    decoder
        .scale(request_w, request_h)
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

    /// The whole point of the EXIF route: the compressed data is byte-identical
    /// however many times the photo is turned. A decode/re-encode rotation
    /// would degrade it on every press.
    #[test]
    fn rotating_a_jpeg_never_touches_its_pixels() {
        let path = write_test_image(320, 240, "jpg");
        let original = ::image::open(&path).unwrap().to_rgb8().into_raw();

        for _ in 0..8 {
            rotate_in_place(&path, true).unwrap();
        }
        // Eight quarter turns is two full circles, so the image is also back
        // the way it started — pixels and orientation both.
        assert_eq!(::image::open(&path).unwrap().to_rgb8().into_raw(), original);
        assert_eq!(thumb_height(&path, 300), Some(225.0));
    }

    /// `full` and `thumbnail` both have to apply the tag, or the single view
    /// and the wall would disagree about which way up a photo is.
    #[test]
    fn a_rotated_jpeg_decodes_the_right_way_up() {
        let path = write_test_image(400, 200, "jpg");
        rotate_in_place(&path, true).unwrap();

        let handle = full(&path).unwrap();
        match handle {
            image::Handle::Rgba { width, height, .. } => assert_eq!((width, height), (200, 400)),
            _ => panic!("expected an rgba handle"),
        }
        let (_thumb, height) = thumbnail(&path, 100).unwrap();
        assert_eq!(height, 200); // 100 wide, now twice as tall as it is wide
    }

    /// The wall lays out from `thumb_height` and repaints from `thumbnail`; if
    /// they disagreed by even a pixel the masonry would jump as decodes landed.
    #[test]
    fn the_header_height_matches_the_decode_for_a_rotated_jpeg() {
        let path = write_test_image(360, 240, "jpg");
        rotate_in_place(&path, false).unwrap();
        let (_handle, decoded) = thumbnail(&path, 300).unwrap();
        assert_eq!(thumb_height(&path, 300), Some(decoded as f32));
    }

    /// PNG carries no orientation tag, so it is still turned by its pixels —
    /// which costs nothing, the codec being lossless.
    #[test]
    fn rotating_a_png_swaps_its_stored_dimensions() {
        let path = write_test_image(200, 100, "png");
        rotate_in_place(&path, true).unwrap();
        let (w, h) = ::image::open(&path).unwrap().dimensions();
        assert_eq!((w, h), (100, 200));
    }

    /// The prescaled decode must never come out narrower than the thumbnail it
    /// feeds, or the resize becomes an upscale and the thumbnail goes soft.
    #[test]
    fn the_prescaled_decode_covers_the_thumbnail_width() {
        // A portrait JPEG is the case a square scale request gets wrong: it can
        // be satisfied on height alone, leaving the width short.
        let path = write_test_image(600, 1200, "jpg");
        let img = decode_jpeg_prescaled(&path, 300, false).unwrap().unwrap();
        assert!(img.width() >= 300, "decoded {} wide", img.width());
    }

    #[test]
    fn the_prescaled_decode_covers_a_quarter_turned_thumbnail() {
        // Stored landscape, shown portrait: it is the stored *height* that has
        // to cover the thumbnail's width.
        let path = write_test_image(1200, 600, "jpg");
        let img = decode_jpeg_prescaled(&path, 300, true).unwrap().unwrap();
        assert!(img.height() >= 300, "decoded {} tall", img.height());
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
