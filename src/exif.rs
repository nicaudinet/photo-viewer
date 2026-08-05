//! Writing the EXIF orientation tag of a JPEG, so rotation costs no quality.
//!
//! Rotating a JPEG by decoding, turning the pixels and re-encoding loses a
//! little of the image every time. Cameras don't do that — they leave the
//! pixels alone and record which way up the photo is in one EXIF tag, and so do
//! we. A rotation is then a handful of bytes, exact and instant, however large
//! the photo.
//!
//! Reading orientation is left to the `image` crate (see `imaging`); only
//! writing is done here, because the three cases below need control over which
//! bytes move.
//!
//! ## Layout
//!
//! A JPEG is `FFD8` followed by segments: a `FF` marker byte, a marker id, then
//! a big-endian length covering the length field itself. EXIF lives in an
//! `APP1` (`FFE1`) segment whose payload starts `Exif\0\0` and continues with a
//! TIFF block: an endianness mark, `42`, and the offset of IFD0. Each IFD is a
//! `u16` entry count, that many 12-byte entries (tag, type, count, value —
//! where values longer than 4 bytes are stored elsewhere and referenced by an
//! offset from the start of the TIFF block), then the offset of the next IFD.
//!
//! ## Why it is written this way
//!
//! Those interior offsets are what makes editing EXIF dangerous: move any bytes
//! inside the TIFF block and every offset past the move — including ones inside
//! a camera's `MakerNote`, which are not always self-describing — silently
//! points at the wrong place. So nothing here ever moves an existing byte:
//!
//! - **Orientation already present**: overwrite its two value bytes in place.
//! - **EXIF present, no orientation tag**: append a *copy* of IFD0 with the new
//!   entry to the end of the TIFF block and repoint the header at it. Every
//!   existing byte, and therefore every existing offset, stays exactly where it
//!   was; the old copy of IFD0 is left behind as dead space.
//! - **No EXIF at all**: build a minimal APP1 and splice it in.

use std::fs;
use std::path::Path;

/// The EXIF tag for orientation, and the type id for `SHORT`.
const TAG_ORIENTATION: u16 = 0x0112;
const TYPE_SHORT: u16 = 3;

/// Bytes in one IFD entry: tag, type, count, value.
const ENTRY_LEN: usize = 12;

/// A JPEG APP1 segment's payload cannot exceed this (the 16-bit length field
/// covers itself, so 65535 - 2).
const MAX_APP1_PAYLOAD: usize = 65533;

/// The orientation an image ends up with after a quarter turn.
///
/// The eight EXIF orientations are four rotations and their mirror images, so a
/// quarter turn walks two separate 4-cycles: the plain rotations and the
/// mirrored ones. Mirrored values only arise from files we didn't write, but
/// they have to keep working — turning a mirrored photo must leave it mirrored.
pub(crate) fn rotate_orientation(current: u8, clockwise: bool) -> u8 {
    // Index by orientation - 1. Clockwise: 1->6->3->8->1 and 2->7->4->5->2.
    const CW: [u8; 8] = [6, 7, 8, 5, 2, 3, 4, 1];
    const CCW: [u8; 8] = [8, 5, 6, 7, 4, 1, 2, 3];
    let table = if clockwise { CW } else { CCW };
    match current {
        1..=8 => table[current as usize - 1],
        // Not a valid orientation: treat the image as un-rotated, which is what
        // every reader does with a value it doesn't recognise.
        _ => table[0],
    }
}

/// Set `path`'s EXIF orientation to `value`, leaving its pixels untouched.
pub(crate) fn write_orientation(path: &Path, value: u8) -> Result<(), String> {
    let mut data = fs::read(path).map_err(|e| e.to_string())?;

    match find_app1_exif(&data)? {
        Some(app1) => match find_orientation_entry(&data, &app1)? {
            // The common case, and the only one that writes no new bytes:
            // cameras record orientation, so the tag is usually already there.
            Some(at) => {
                let bytes = if app1.little_endian {
                    (value as u16).to_le_bytes()
                } else {
                    (value as u16).to_be_bytes()
                };
                data[at..at + 2].copy_from_slice(&bytes);
            }
            None => append_orientation_entry(&mut data, &app1, value)?,
        },
        None => insert_exif_segment(&mut data, value)?,
    }

    write_atomically(path, &data)
}

/// Replace `path`'s contents via a temporary file in the same directory.
///
/// A photo library is not worth truncating: writing in place would leave the
/// file half-written if the process died mid-write, and the pixels we are
/// preserving would go with it.
fn write_atomically(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("pv-rotate-tmp");
    fs::write(&tmp, data).map_err(|e| e.to_string())?;
    // Same directory, so this is a rename within one filesystem: atomic, and it
    // inherits nothing from the temporary file but its contents.
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Where a file's EXIF APP1 segment is, and how to read numbers inside it.
struct App1 {
    /// Offset of the segment's `FF` marker byte.
    marker: usize,
    /// Offset of the TIFF block: the byte after `Exif\0\0`. All the offsets
    /// stored inside the block are relative to this.
    tiff: usize,
    /// One past the last byte of the segment's payload.
    end: usize,
    little_endian: bool,
}

impl App1 {
    fn u16(&self, data: &[u8], at: usize) -> Result<u16, String> {
        let bytes: [u8; 2] = data
            .get(at..at + 2)
            .ok_or("exif: truncated")?
            .try_into()
            .map_err(|_| "exif: truncated")?;
        Ok(if self.little_endian {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    }

    fn u32(&self, data: &[u8], at: usize) -> Result<u32, String> {
        let bytes: [u8; 4] = data
            .get(at..at + 4)
            .ok_or("exif: truncated")?
            .try_into()
            .map_err(|_| "exif: truncated")?;
        Ok(if self.little_endian {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    }

    fn put_u16(&self, out: &mut Vec<u8>, value: u16) {
        if self.little_endian {
            out.extend_from_slice(&value.to_le_bytes());
        } else {
            out.extend_from_slice(&value.to_be_bytes());
        }
    }

    fn put_u32(&self, out: &mut Vec<u8>, value: u32) {
        if self.little_endian {
            out.extend_from_slice(&value.to_le_bytes());
        } else {
            out.extend_from_slice(&value.to_be_bytes());
        }
    }
}

/// Walk the JPEG's header segments looking for the EXIF APP1.
///
/// Stops at `SOS`, after which the file is entropy-coded scan data and no
/// longer a sequence of length-prefixed segments.
fn find_app1_exif(data: &[u8]) -> Result<Option<App1>, String> {
    if data.get(..2) != Some(&[0xFF, 0xD8][..]) {
        return Err("not a JPEG (no SOI)".into());
    }
    let mut pos = 2;
    while pos + 4 <= data.len() {
        if data[pos] != 0xFF {
            return Err("exif: lost segment alignment".into());
        }
        let marker = data[pos + 1];
        // Start of scan, or end of image: no more header segments.
        if marker == 0xDA || marker == 0xD9 {
            return Ok(None);
        }
        // Padding, and the standalone markers that carry no length.
        if marker == 0xFF || (0xD0..=0xD8).contains(&marker) || marker == 0x01 {
            pos += 1;
            continue;
        }

        let length = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        if length < 2 {
            return Err("exif: bad segment length".into());
        }
        let payload = pos + 4;
        let end = pos + 2 + length;
        if end > data.len() {
            return Err("exif: segment runs past end of file".into());
        }

        if marker == 0xE1 && data.get(payload..payload + 6) == Some(b"Exif\0\0") {
            let tiff = payload + 6;
            let little_endian = match data.get(tiff..tiff + 2) {
                Some(b"II") => true,
                Some(b"MM") => false,
                _ => return Err("exif: unknown byte order".into()),
            };
            return Ok(Some(App1 {
                marker: pos,
                tiff,
                end,
                little_endian,
            }));
        }
        pos = end;
    }
    Ok(None)
}

/// The file offset of the orientation entry's *value*, if IFD0 has one.
fn find_orientation_entry(data: &[u8], app1: &App1) -> Result<Option<usize>, String> {
    let ifd0 = app1.tiff + app1.u32(data, app1.tiff + 4)? as usize;
    let count = app1.u16(data, ifd0)? as usize;
    for i in 0..count {
        let entry = ifd0 + 2 + i * ENTRY_LEN;
        if app1.u16(data, entry)? == TAG_ORIENTATION {
            // A SHORT fits in the 4-byte value field, so it is stored inline —
            // in the field's first two bytes, whichever the byte order.
            return Ok(Some(entry + 8));
        }
    }
    Ok(None)
}

/// Add an orientation entry to a file whose EXIF has none.
///
/// Appends a fresh copy of IFD0 — the old entries plus the new one, in tag
/// order — to the end of the TIFF block and points the header at it. Nothing
/// already in the block moves, so no offset inside it (including any inside a
/// `MakerNote`) is invalidated. The original IFD0 is simply orphaned.
fn append_orientation_entry(data: &mut Vec<u8>, app1: &App1, value: u8) -> Result<(), String> {
    let ifd0 = app1.tiff + app1.u32(data, app1.tiff + 4)? as usize;
    let count = app1.u16(data, ifd0)? as usize;
    let entries_at = ifd0 + 2;
    let next_ifd = app1.u32(data, entries_at + count * ENTRY_LEN)?;

    let mut new_ifd = Vec::with_capacity(2 + (count + 1) * ENTRY_LEN + 4);
    app1.put_u16(&mut new_ifd, (count + 1) as u16);

    // Entries are ordered by tag, so the new one is spliced into place rather
    // than appended.
    let mut written = false;
    for i in 0..count {
        let entry = entries_at + i * ENTRY_LEN;
        let tag = app1.u16(data, entry)?;
        if !written && tag > TAG_ORIENTATION {
            put_orientation_entry(app1, &mut new_ifd, value);
            written = true;
        }
        new_ifd.extend_from_slice(
            data.get(entry..entry + ENTRY_LEN)
                .ok_or("exif: truncated IFD")?,
        );
    }
    if !written {
        put_orientation_entry(app1, &mut new_ifd, value);
    }
    app1.put_u32(&mut new_ifd, next_ifd);

    // TIFF offsets are word-aligned; pad rather than start the IFD odd.
    let mut appended = Vec::new();
    if !(app1.end - app1.tiff).is_multiple_of(2) {
        appended.push(0);
    }
    let new_ifd_offset = app1.end - app1.tiff + appended.len();
    appended.extend_from_slice(&new_ifd);

    let payload = app1.end + appended.len() - (app1.marker + 2);
    if payload > MAX_APP1_PAYLOAD {
        return Err("exif: no room left in the APP1 segment".into());
    }

    // Repoint the header at the copy, grow the segment, splice the copy in.
    let header_offset = app1.tiff + 4;
    let offset_bytes = if app1.little_endian {
        (new_ifd_offset as u32).to_le_bytes()
    } else {
        (new_ifd_offset as u32).to_be_bytes()
    };
    data[header_offset..header_offset + 4].copy_from_slice(&offset_bytes);
    data[app1.marker + 2..app1.marker + 4].copy_from_slice(&(payload as u16).to_be_bytes());
    data.splice(app1.end..app1.end, appended);
    Ok(())
}

fn put_orientation_entry(app1: &App1, out: &mut Vec<u8>, value: u8) {
    app1.put_u16(out, TAG_ORIENTATION);
    app1.put_u16(out, TYPE_SHORT);
    app1.put_u32(out, 1);
    // Inline value, padded out to the full four bytes.
    app1.put_u16(out, value as u16);
    app1.put_u16(out, 0);
}

/// Give a JPEG with no EXIF at all a minimal APP1 holding just the orientation.
///
/// Placed after a JFIF `APP0` if there is one, since that is conventionally
/// first, and otherwise straight after `SOI`.
fn insert_exif_segment(data: &mut Vec<u8>, value: u8) -> Result<(), String> {
    if data.get(..2) != Some(&[0xFF, 0xD8][..]) {
        return Err("not a JPEG (no SOI)".into());
    }
    let mut at = 2;
    if data.get(2..4) == Some(&[0xFF, 0xE0][..]) {
        let length = u16::from_be_bytes([
            *data.get(4).ok_or("exif: truncated")?,
            *data.get(5).ok_or("exif: truncated")?,
        ]) as usize;
        // The length field covers itself but not the two marker bytes, so the
        // segment ends `2 + length` past its marker.
        at = 2 + 2 + length;
        if at > data.len() {
            return Err("exif: APP0 runs past end of file".into());
        }
    }

    // Little-endian TIFF, IFD0 immediately after the 8-byte header, one entry.
    let mut payload = Vec::from(*b"Exif\0\0");
    payload.extend_from_slice(b"II");
    payload.extend_from_slice(&42u16.to_le_bytes());
    payload.extend_from_slice(&8u32.to_le_bytes());
    payload.extend_from_slice(&1u16.to_le_bytes());
    payload.extend_from_slice(&TAG_ORIENTATION.to_le_bytes());
    payload.extend_from_slice(&TYPE_SHORT.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&(value as u16).to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    // No further IFDs.
    payload.extend_from_slice(&0u32.to_le_bytes());

    let mut segment = vec![0xFF, 0xE1];
    segment.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    segment.extend_from_slice(&payload);
    data.splice(at..at, segment);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageDecoder;

    /// Every orientation, turned one way and then back, is itself again.
    #[test]
    fn a_turn_each_way_cancels_out() {
        for o in 1..=8u8 {
            assert_eq!(rotate_orientation(rotate_orientation(o, true), false), o);
            assert_eq!(rotate_orientation(rotate_orientation(o, false), true), o);
        }
    }

    #[test]
    fn four_turns_come_back_round() {
        for o in 1..=8u8 {
            let mut turned = o;
            for _ in 0..4 {
                turned = rotate_orientation(turned, true);
            }
            assert_eq!(turned, o);
        }
    }

    #[test]
    fn turning_an_upright_photo_clockwise_gives_the_exif_value_for_90() {
        assert_eq!(rotate_orientation(1, true), 6);
        assert_eq!(rotate_orientation(1, false), 8);
        assert_eq!(rotate_orientation(6, true), 3);
    }

    #[test]
    fn a_mirrored_photo_stays_mirrored() {
        // The mirrored orientations form their own 4-cycle: turning one must
        // never land on a plain rotation, or the image would flip.
        let mirrored = [2u8, 4, 5, 7];
        for o in mirrored {
            assert!(mirrored.contains(&rotate_orientation(o, true)));
            assert!(mirrored.contains(&rotate_orientation(o, false)));
        }
    }

    #[test]
    fn a_nonsense_orientation_is_treated_as_upright() {
        assert_eq!(rotate_orientation(0, true), 6);
        assert_eq!(rotate_orientation(99, true), 6);
    }

    // --- Round trips through real files ---

    fn unique_path(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("pv-exif-{tag}-{}-{n}.jpg", std::process::id()))
    }

    /// A small JPEG with no EXIF of any kind, as `image` writes it.
    fn plain_jpeg(tag: &str, w: u32, h: u32) -> std::path::PathBuf {
        let path = unique_path(tag);
        let mut buf = image::RgbImage::new(w, h);
        for (x, y, px) in buf.enumerate_pixels_mut() {
            *px = image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]);
        }
        image::DynamicImage::ImageRgb8(buf).save(&path).unwrap();
        path
    }

    fn read_orientation(path: &Path) -> u8 {
        let mut decoder = image::ImageReader::open(path)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .into_decoder()
            .unwrap();
        decoder.orientation().unwrap().to_exif()
    }

    fn pixels(path: &Path) -> Vec<u8> {
        image::open(path).unwrap().to_rgb8().into_raw()
    }

    #[test]
    fn writes_orientation_into_a_file_that_had_no_exif() {
        let path = plain_jpeg("no-exif", 64, 48);
        let before = pixels(&path);

        write_orientation(&path, 6).unwrap();
        assert_eq!(read_orientation(&path), 6);
        // The whole point: the compressed image is untouched.
        assert_eq!(pixels(&path), before);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn overwrites_an_orientation_that_is_already_there() {
        let path = plain_jpeg("overwrite", 64, 48);
        write_orientation(&path, 6).unwrap();
        let after_first = fs::metadata(&path).unwrap().len();

        write_orientation(&path, 3).unwrap();
        assert_eq!(read_orientation(&path), 3);
        // The second write patches two bytes in place, so the file cannot grow.
        assert_eq!(fs::metadata(&path).unwrap().len(), after_first);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn every_orientation_round_trips() {
        let path = plain_jpeg("all-values", 32, 32);
        for value in 1..=8u8 {
            write_orientation(&path, value).unwrap();
            assert_eq!(read_orientation(&path), value);
        }
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn appends_an_entry_to_exif_that_has_no_orientation() {
        let path = plain_jpeg("append", 64, 48);
        let before = pixels(&path);

        // Write an EXIF block, then strip the orientation entry back out by
        // rewriting the tag id — leaving a file that has EXIF but no
        // orientation, which is the case that has to grow IFD0.
        write_orientation(&path, 6).unwrap();
        let mut data = fs::read(&path).unwrap();
        let app1 = find_app1_exif(&data).unwrap().unwrap();
        let entry = find_orientation_entry(&data, &app1).unwrap().unwrap() - 8;
        // 0x011A (XResolution) is a plausible tag that is not orientation.
        data[entry..entry + 2].copy_from_slice(&0x011Au16.to_le_bytes());
        fs::write(&path, &data).unwrap();
        assert_eq!(read_orientation(&path), 1);

        write_orientation(&path, 8).unwrap();
        assert_eq!(read_orientation(&path), 8);
        assert_eq!(pixels(&path), before);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn the_appended_entry_leaves_the_original_bytes_where_they_were() {
        let path = plain_jpeg("append-stable", 32, 32);
        write_orientation(&path, 6).unwrap();
        let mut data = fs::read(&path).unwrap();
        let app1 = find_app1_exif(&data).unwrap().unwrap();
        let entry = find_orientation_entry(&data, &app1).unwrap().unwrap() - 8;
        data[entry..entry + 2].copy_from_slice(&0x011Au16.to_le_bytes());
        fs::write(&path, &data).unwrap();

        let before = fs::read(&path).unwrap();
        write_orientation(&path, 8).unwrap();
        let after = fs::read(&path).unwrap();

        // Everything up to the end of the old TIFF block is byte-identical
        // apart from the two header bytes holding the IFD0 offset and the
        // segment's length — which is what keeps interior offsets valid.
        let app1 = find_app1_exif(&before).unwrap().unwrap();
        assert!(after.len() > before.len());
        assert_eq!(before[..app1.marker + 2], after[..app1.marker + 2]);
        assert_eq!(
            before[app1.tiff + 8..app1.end],
            after[app1.tiff + 8..app1.end]
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn refuses_a_file_that_is_not_a_jpeg() {
        let path = unique_path("not-jpeg");
        fs::write(&path, b"this is not a JPEG").unwrap();
        assert!(write_orientation(&path, 6).is_err());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_failed_write_leaves_no_temporary_file_behind() {
        let path = unique_path("no-temp");
        fs::write(&path, b"not a JPEG either").unwrap();
        let _ = write_orientation(&path, 6);
        assert!(!path.with_extension("pv-rotate-tmp").exists());
        let _ = fs::remove_file(&path);
    }
}


