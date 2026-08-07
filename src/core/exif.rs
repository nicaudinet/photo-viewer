//! Writing the EXIF orientation tag of a JPEG, so rotation costs no quality,
//! and reading the time a photo was taken.
//!
//! Rotating a JPEG by decoding, turning the pixels and re-encoding loses a
//! little of the image every time. Cameras don't do that — they leave the
//! pixels alone and record which way up the photo is in one EXIF tag, and so do
//! we. A rotation is then a handful of bytes, exact and instant, however large
//! the photo.
//!
//! Reading orientation is left to the `image` crate (see `imaging`); only
//! writing is done here, because the three cases below need control over which
//! bytes move. The timestamp is read here because the `image` crate does not
//! expose it at all.
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
use std::io::Read;
use std::path::Path;

/// The EXIF tag for orientation, and the type id for `SHORT`.
const TAG_ORIENTATION: u16 = 0x0112;
const TYPE_SHORT: u16 = 3;

/// IFD0's own timestamp — when the file was last written, per the spec, though
/// cameras write the capture time here too.
const TAG_DATE_TIME: u16 = 0x0132;
/// IFD0's pointer to the Exif SubIFD, where the interesting tags live.
const TAG_EXIF_IFD: u16 = 0x8769;
/// When the shutter fired. In the SubIFD, not IFD0.
const TAG_DATE_TIME_ORIGINAL: u16 = 0x9003;

const TYPE_ASCII: u16 = 2;
const TYPE_LONG: u16 = 4;

/// Bytes in one IFD entry: tag, type, count, value.
const ENTRY_LEN: usize = 12;

/// A JPEG APP1 segment's payload cannot exceed this (the 16-bit length field
/// covers itself, so 65535 - 2).
const MAX_APP1_PAYLOAD: usize = 65533;

/// `"YYYY:MM:DD HH:MM:SS"` — the EXIF datetime format, without its trailing NUL.
const DATETIME_LEN: usize = 19;

/// How much of a file to read when only its header is wanted.
///
/// The EXIF APP1 sits within the first few segments and cannot itself exceed
/// 64 KiB, so this covers it with room to spare — and reading a timestamp must
/// not mean pulling a 30 MB photo through the disk cache, since this runs over
/// every image in a folder.
const HEADER_PREFIX: u64 = 128 * 1024;

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

/// The file offset of IFD0, read from the TIFF header.
fn ifd0(data: &[u8], app1: &App1) -> Result<usize, String> {
    Ok(app1.tiff + app1.u32(data, app1.tiff + 4)? as usize)
}

/// The file offset of the *start* of `tag`'s entry within the IFD at `ifd`, if
/// it has one. The caller reads the type, count and value fields it needs.
fn find_entry(data: &[u8], app1: &App1, ifd: usize, tag: u16) -> Result<Option<usize>, String> {
    let count = app1.u16(data, ifd)? as usize;
    for i in 0..count {
        let entry = ifd + 2 + i * ENTRY_LEN;
        if app1.u16(data, entry)? == tag {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

/// The file offset of the orientation entry's *value*, if IFD0 has one.
fn find_orientation_entry(data: &[u8], app1: &App1) -> Result<Option<usize>, String> {
    let ifd0 = ifd0(data, app1)?;
    // A SHORT fits in the 4-byte value field, so it is stored inline — in the
    // field's first two bytes, whichever the byte order.
    Ok(find_entry(data, app1, ifd0, TAG_ORIENTATION)?.map(|entry| entry + 8))
}

/// Add an orientation entry to a file whose EXIF has none.
///
/// Appends a fresh copy of IFD0 — the old entries plus the new one, in tag
/// order — to the end of the TIFF block and points the header at it. Nothing
/// already in the block moves, so no offset inside it (including any inside a
/// `MakerNote`) is invalidated. The original IFD0 is simply orphaned.
fn append_orientation_entry(data: &mut Vec<u8>, app1: &App1, value: u8) -> Result<(), String> {
    let ifd0 = ifd0(data, app1)?;
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

// --- Reading the time a photo was taken ---

/// When `path` was taken, in seconds.
///
/// `DateTimeOriginal` — when the shutter fired — is preferred, falling back to
/// IFD0's `DateTime`, which cameras also write the capture time into and which
/// is all a stripped-down file may have left.
///
/// Every failure is `None`. This is an optional signal used to decide whether
/// two photos belong together: a malformed timestamp and an absent one lead to
/// exactly the same decision, so distinguishing them would buy nothing.
///
/// ## About the units
///
/// EXIF stores wall-clock time with no zone, so this is seconds since the epoch
/// *as if the photo were taken in UTC*. That is deliberately not a real instant,
/// and it must only ever be used to take differences between photos from the
/// same camera on the same day — which is the only thing grouping asks of it.
/// A pair spanning a daylight-saving change or a flight is off by the offset.
// Nothing calls this until fingerprints land (`GROUP_MODE_PLAN.md` phase 1);
// the whole read path hangs off it, so this one attribute keeps the rest live.
#[allow(dead_code)]
pub(crate) fn taken_at(path: &Path) -> Option<i64> {
    let data = read_header(path).ok()?;
    let app1 = find_app1_exif(&data).ok().flatten()?;
    let ifd0 = ifd0(&data, &app1).ok()?;

    sub_ifd(&data, &app1, ifd0)
        .and_then(|sub| datetime_at(&data, &app1, sub, TAG_DATE_TIME_ORIGINAL))
        .or_else(|| datetime_at(&data, &app1, ifd0, TAG_DATE_TIME))
}

/// Read at most [`HEADER_PREFIX`] bytes of `path`.
fn read_header(path: &Path) -> Result<Vec<u8>, std::io::Error> {
    let mut buf = Vec::new();
    fs::File::open(path)?
        .take(HEADER_PREFIX)
        .read_to_end(&mut buf)?;
    Ok(buf)
}

/// The file offset of the Exif SubIFD, if IFD0 points at one.
fn sub_ifd(data: &[u8], app1: &App1, ifd0: usize) -> Option<usize> {
    let entry = find_entry(data, app1, ifd0, TAG_EXIF_IFD).ok().flatten()?;
    if app1.u16(data, entry + 2).ok()? != TYPE_LONG {
        return None;
    }
    // A LONG fits the value field, so the offset is stored inline.
    Some(app1.tiff + app1.u32(data, entry + 8).ok()? as usize)
}

/// `tag`'s value in the IFD at `ifd`, parsed as an EXIF datetime.
fn datetime_at(data: &[u8], app1: &App1, ifd: usize, tag: u16) -> Option<i64> {
    let entry = find_entry(data, app1, ifd, tag).ok().flatten()?;
    if app1.u16(data, entry + 2).ok()? != TYPE_ASCII {
        return None;
    }
    // 19 characters and a NUL, so the value never fits the 4-byte field and is
    // always stored elsewhere in the TIFF block.
    if (app1.u32(data, entry + 4).ok()? as usize) <= DATETIME_LEN {
        return None;
    }
    let at = app1.tiff + app1.u32(data, entry + 8).ok()? as usize;
    let text = std::str::from_utf8(data.get(at..at + DATETIME_LEN)?).ok()?;
    parse_datetime(text)
}

/// `"YYYY:MM:DD HH:MM:SS"` to seconds since the epoch, read as UTC.
///
/// Strict about the shape, because the one thing that must not happen is
/// reading a partly-blank field — cameras pad an unset timestamp with spaces —
/// as a real time near the year zero.
fn parse_datetime(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != DATETIME_LEN {
        return None;
    }
    for (i, b) in bytes.iter().enumerate() {
        let expected_separator = matches!(i, 4 | 7 | 13 | 16);
        let ok = match i {
            10 => *b == b' ',
            _ if expected_separator => *b == b':',
            _ => b.is_ascii_digit(),
        };
        if !ok {
            return None;
        }
    }

    let num = |range: std::ops::Range<usize>| text[range].parse::<i64>().ok();
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // 60 is a leap second, which is a real thing to find in a file.
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Days from 1970-01-01 to `y-m-d`, proleptic Gregorian.
///
/// Howard Hinnant's `days_from_civil`: shift the year to start in March so the
/// leap day lands at the end of it, then count whole 400-year eras, whose length
/// in days is fixed.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let year_of_era = y - era * 400;
    let march_based = if m > 2 { m - 3 } else { m + 9 };
    let day_of_year = (153 * march_based + 2) / 5 + d - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    // 719468 is the days from 0000-03-01 to 1970-01-01.
    era * 146_097 + day_of_era - 719_468
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

    // --- Capture time ---

    fn push_entry(out: &mut Vec<u8>, tag: u16, ty: u16, count: u32, value: u32) {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }

    /// A little-endian TIFF block carrying up to two datetimes: `DateTime` in
    /// IFD0 and `DateTimeOriginal` in an Exif SubIFD.
    ///
    /// Laid out by hand because the `image` crate cannot write EXIF at all, so
    /// there is no other way to get a file with a real one to read.
    fn tiff_block(ifd0_dt: Option<&str>, sub_dt: Option<&str>) -> Vec<u8> {
        // Offsets are relative to the start of the block: the 8-byte header,
        // then IFD0, then the SubIFD, then the strings both point into.
        let entries = ifd0_dt.is_some() as usize + sub_dt.is_some() as usize;
        let ifd0_at = 8;
        let sub_at = ifd0_at + 2 + entries * ENTRY_LEN + 4;
        let data_at = sub_at
            + if sub_dt.is_some() {
                2 + ENTRY_LEN + 4
            } else {
                0
            };
        let value_len = (DATETIME_LEN + 1) as u32;

        let mut tiff = Vec::from(*b"II");
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&(ifd0_at as u32).to_le_bytes());

        tiff.extend_from_slice(&(entries as u16).to_le_bytes());
        if ifd0_dt.is_some() {
            push_entry(
                &mut tiff,
                TAG_DATE_TIME,
                TYPE_ASCII,
                value_len,
                data_at as u32,
            );
        }
        if sub_dt.is_some() {
            push_entry(&mut tiff, TAG_EXIF_IFD, TYPE_LONG, 1, sub_at as u32);
        }
        // No IFD1.
        tiff.extend_from_slice(&0u32.to_le_bytes());

        if sub_dt.is_some() {
            let at = data_at
                + if ifd0_dt.is_some() {
                    DATETIME_LEN + 1
                } else {
                    0
                };
            tiff.extend_from_slice(&1u16.to_le_bytes());
            push_entry(
                &mut tiff,
                TAG_DATE_TIME_ORIGINAL,
                TYPE_ASCII,
                value_len,
                at as u32,
            );
            tiff.extend_from_slice(&0u32.to_le_bytes());
        }

        assert_eq!(tiff.len(), data_at, "the layout and the offsets must agree");
        // IFD0's string first, matching the offsets computed above.
        for dt in [ifd0_dt, sub_dt].into_iter().flatten() {
            assert_eq!(dt.len(), DATETIME_LEN);
            tiff.extend_from_slice(dt.as_bytes());
            tiff.push(0);
        }
        tiff
    }

    fn jpeg_with_exif(
        tag: &str,
        ifd0_dt: Option<&str>,
        sub_dt: Option<&str>,
    ) -> std::path::PathBuf {
        let path = plain_jpeg(tag, 16, 16);
        let mut payload = Vec::from(*b"Exif\0\0");
        payload.extend_from_slice(&tiff_block(ifd0_dt, sub_dt));

        let mut segment = vec![0xFF, 0xE1];
        segment.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        segment.extend_from_slice(&payload);

        let mut data = fs::read(&path).unwrap();
        // Straight after SOI, ahead of the JFIF APP0 `image` writes. The scan
        // walks segments in order, so where it sits does not matter.
        data.splice(2..2, segment);
        fs::write(&path, &data).unwrap();
        path
    }

    #[test]
    fn reads_the_capture_time_from_the_exif_subifd() {
        let path = jpeg_with_exif("taken-sub", None, Some("2024:02:29 12:00:00"));
        assert_eq!(taken_at(&path), Some(1_709_208_000));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn falls_back_to_the_ifd0_timestamp() {
        // All a stripped-down file may have left.
        let path = jpeg_with_exif("taken-ifd0", Some("2000:01:01 00:00:00"), None);
        assert_eq!(taken_at(&path), Some(946_684_800));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn the_shutter_time_wins_over_the_file_time() {
        let path = jpeg_with_exif(
            "taken-both",
            Some("2000:01:01 00:00:00"),
            Some("2024:02:29 12:00:00"),
        );
        assert_eq!(taken_at(&path), Some(1_709_208_000));
        let _ = fs::remove_file(&path);
    }

    /// The only thing grouping actually asks of a timestamp.
    #[test]
    fn two_photos_a_minute_apart_are_sixty_seconds_apart() {
        let first = jpeg_with_exif("gap-a", None, Some("2024:06:01 14:30:00"));
        let second = jpeg_with_exif("gap-b", None, Some("2024:06:01 14:31:00"));
        assert_eq!(taken_at(&second).unwrap() - taken_at(&first).unwrap(), 60);
        let _ = fs::remove_file(&first);
        let _ = fs::remove_file(&second);
    }

    #[test]
    fn a_photo_with_no_exif_has_no_capture_time() {
        let path = plain_jpeg("taken-none", 16, 16);
        assert_eq!(taken_at(&path), None);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_blank_timestamp_is_not_a_time() {
        // Cameras pad an unset field with spaces. Read loosely it would parse
        // as a real time near the year zero, and every such photo would then
        // look like it was taken at the same moment as every other.
        let path = jpeg_with_exif("taken-blank", None, Some("    :  :     :  :  "));
        assert_eq!(taken_at(&path), None);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_file_that_is_not_a_jpeg_has_no_capture_time() {
        let path = unique_path("taken-notjpeg");
        fs::write(&path, b"this is not a JPEG").unwrap();
        assert_eq!(taken_at(&path), None);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_has_no_capture_time() {
        assert_eq!(taken_at(&unique_path("taken-missing")), None);
    }

    #[test]
    fn only_the_header_of_a_file_is_read() {
        let path = unique_path("prefix");
        fs::write(&path, vec![0u8; HEADER_PREFIX as usize * 2]).unwrap();
        // Reading a timestamp runs over every photo in a folder; pulling whole
        // 30 MB files through the disk cache to do it would not be affordable.
        assert_eq!(read_header(&path).unwrap().len(), HEADER_PREFIX as usize);
        let _ = fs::remove_file(&path);
    }

    // --- Parsing ---

    #[test]
    fn the_epoch_is_zero() {
        assert_eq!(parse_datetime("1970:01:01 00:00:00"), Some(0));
        assert_eq!(parse_datetime("1970:01:02 00:00:00"), Some(86_400));
        assert_eq!(parse_datetime("1970:01:01 01:02:03"), Some(3_723));
    }

    #[test]
    fn leap_days_are_counted() {
        // 2024 is a leap year, 1900 is not: the difference between those two
        // rules is exactly what `days_from_civil` exists to get right.
        assert_eq!(
            parse_datetime("2024:03:01 00:00:00").unwrap()
                - parse_datetime("2024:02:28 00:00:00").unwrap(),
            2 * 86_400
        );
        assert_eq!(
            parse_datetime("1900:03:01 00:00:00").unwrap()
                - parse_datetime("1900:02:28 00:00:00").unwrap(),
            86_400
        );
    }

    #[test]
    fn a_leap_second_is_accepted() {
        // A real thing to find in a file, and rounding it away would be a lie.
        assert!(parse_datetime("2016:12:31 23:59:60").is_some());
    }

    #[test]
    fn a_malformed_timestamp_is_refused() {
        for text in [
            "",
            "2024:02:29",
            "2024:02:29 12:00:00 ",
            "2024-02-29 12:00:00",
            "2024:02:29T12:00:00",
            "20x4:02:29 12:00:00",
            "2024:13:01 12:00:00",
            "2024:00:01 12:00:00",
            "2024:02:32 12:00:00",
            "2024:02:29 24:00:00",
            "2024:02:29 12:60:00",
            "2024:02:29 12:00:61",
        ] {
            assert_eq!(parse_datetime(text), None, "{text:?} should not parse");
        }
    }
}
