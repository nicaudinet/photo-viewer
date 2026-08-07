//! Remembering fingerprints between runs, so `g` is only slow once.
//!
//! Hashing a folder means decoding every photo in it, which is the one part of
//! grouping that costs real time. Nothing about a fingerprint changes unless the
//! file does, so it is worth writing down.
//!
//! One file, `<image_dir>/.photo-viewer/fingerprints`, placed and shaped after
//! [`crate::core::tags`]: plain text, one record per line, naming photos by
//! **file name** rather than absolute path so that renaming or moving the folder
//! keeps the cache attached to the photos.
//!
//! The store is disposable. There is no version number and no error path for a
//! line that does not parse — a record the code cannot read is simply not a hit,
//! and the photo is hashed again. That is also what makes the format free to
//! change: the old lines quietly stop matching.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::core::fingerprint::{self, Fingerprint};

fn store_file(image_dir: &Path) -> PathBuf {
    image_dir.join(".photo-viewer").join("fingerprints")
}

/// What a file looked like when its fingerprint was taken.
///
/// Modification time *and* size, because either alone is fooled by an edit that
/// happens to leave the other untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    /// Nanoseconds since the epoch, signed so a file dated before 1970 is not a
    /// wrapped enormous number.
    ///
    /// Full precision rather than whole seconds: rotating a photo rewrites one
    /// EXIF tag in place, leaving the size identical, so a second-resolution
    /// stamp would miss a rotation that landed in the same second as the hash —
    /// and rotation is exactly the edit that changes what the hash should say.
    modified: i128,
    size: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = fs::metadata(path).ok()?;
    Some(Stamp {
        modified: nanos(meta.modified().ok()?),
        size: meta.len(),
    })
}

fn nanos(time: SystemTime) -> i128 {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(since) => since.as_nanos() as i128,
        Err(before) => -(before.duration().as_nanos() as i128),
    }
}

/// The fingerprints of one folder, and what the files looked like at the time.
///
/// Ordered, so the store is written the same way twice running.
#[derive(Debug, Default)]
pub struct Cache {
    entries: BTreeMap<PathBuf, (Stamp, Fingerprint)>,
}

impl Cache {
    /// Read the store for `image_dir`, keeping only records for photos that are
    /// still in it. A cache that outlived its photo is dead weight, and the next
    /// [`save`](Self::save) drops it for good.
    ///
    /// A record whose file has *changed* is kept: it costs nothing, and only
    /// [`get`](Self::get) is in a position to say so.
    pub fn load(image_dir: &Path, images: &[PathBuf]) -> Self {
        let known: HashSet<&PathBuf> = images.iter().collect();
        let Ok(contents) = fs::read_to_string(store_file(image_dir)) else {
            return Self::default();
        };

        let entries = contents
            .lines()
            .filter_map(parse)
            .filter_map(|(name, entry)| {
                let path = image_dir.join(Path::new(&name).file_name()?);
                known.contains(&path).then_some((path, entry))
            })
            .collect();
        Self { entries }
    }

    /// The remembered fingerprint of `path`, if the file is still exactly as it
    /// was when that fingerprint was taken.
    pub fn get(&self, path: &Path) -> Option<Fingerprint> {
        let (remembered, print) = self.entries.get(path)?;
        (*remembered == stamp(path)?).then_some(*print)
    }

    /// The fingerprint of `path`, hashing the photo if the cache cannot answer.
    /// `None` if it cannot be read at all, and nothing is remembered then —
    /// there is no point caching an absence that a re-download would fix.
    pub fn fingerprint(&mut self, path: &Path) -> Option<Fingerprint> {
        if let Some(print) = self.get(path) {
            return Some(print);
        }
        // Stamped before the decode, not after. Taken afterwards, a write that
        // landed *during* the decode would be recorded as though it had been
        // hashed, and the stale hash would then look fresh forever.
        let stamp = stamp(path)?;
        let print = fingerprint::fingerprint(path)?;
        self.entries.insert(path.to_path_buf(), (stamp, print));
        Some(print)
    }

    /// Write the whole store, replacing what was there.
    ///
    /// Everything is written from memory each time, which is what prunes the
    /// records [`load`](Self::load) dropped. The file is a few dozen bytes per
    /// photo; there is nothing to gain from writing it in pieces.
    pub fn save(&self, image_dir: &Path) -> Result<(), String> {
        let file = store_file(image_dir);
        let dir = file.parent().unwrap_or(image_dir);
        fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;

        let body: Vec<String> = self
            .entries
            .iter()
            .filter_map(|(path, (stamp, print))| {
                let name = path.file_name()?.to_str()?;
                Some(line(name, stamp, print))
            })
            .collect();

        fs::write(&file, body.join("\n"))
            .map_err(|e| format!("Could not write {}: {e}", file.display()))
    }
}

/// One record: `<dhash> <modified> <size> <taken> <shape> <name>`.
///
/// Hexadecimal hash, decimal everything else, `-` for a photo that does not say
/// when it was taken, and `l` or `p` for the shape. The name comes last so it
/// may hold anything at all but a newline.
fn line(name: &str, stamp: &Stamp, print: &Fingerprint) -> String {
    let taken = print
        .taken
        .map_or_else(|| "-".to_string(), |t| t.to_string());
    let shape = if print.landscape { 'l' } else { 'p' };
    format!(
        "{:016x} {} {} {taken} {shape} {name}",
        print.dhash, stamp.modified, stamp.size
    )
}

fn parse(line: &str) -> Option<(String, (Stamp, Fingerprint))> {
    let mut fields = line.splitn(6, ' ');
    let dhash = u64::from_str_radix(fields.next()?, 16).ok()?;
    let modified = fields.next()?.parse().ok()?;
    let size = fields.next()?.parse().ok()?;
    let taken = match fields.next()? {
        "-" => None,
        text => Some(text.parse().ok()?),
    };
    let landscape = match fields.next()? {
        "l" => true,
        "p" => false,
        _ => return None,
    };
    let name = fields.next()?;
    if name.is_empty() {
        return None;
    }

    let stamp = Stamp { modified, size };
    let print = Fingerprint {
        dhash,
        taken,
        landscape,
    };
    Some((name.to_string(), (stamp, print)))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn dir(tag: &str) -> Dir {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("pv-fpc-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Dir(path)
    }

    /// A decodable photo in `d`, `w` by `h`, whose pixels depend on `shade`.
    fn photo(d: &Dir, name: &str, w: u32, h: u32, shade: u8) -> PathBuf {
        let path = d.0.join(name);
        let mut buf = ::image::RgbImage::new(w, h);
        for (x, y, px) in buf.enumerate_pixels_mut() {
            let v = shade
                .wrapping_add((x * 255 / w) as u8)
                .wrapping_add(y as u8);
            *px = ::image::Rgb([v, v, v]);
        }
        ::image::DynamicImage::ImageRgb8(buf).save(&path).unwrap();
        path
    }

    fn print(dhash: u64, taken: Option<i64>, landscape: bool) -> Fingerprint {
        Fingerprint {
            dhash,
            taken,
            landscape,
        }
    }

    /// A cache holding one made-up record, so the store can be tested without
    /// depending on what the hasher makes of a particular image.
    fn holding(path: &Path, stamp: Stamp, print: Fingerprint) -> Cache {
        Cache {
            entries: BTreeMap::from([(path.to_path_buf(), (stamp, print))]),
        }
    }

    fn touch(path: &Path, at: SystemTime) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }

    // --- The store ---

    #[test]
    fn a_fingerprint_survives_a_round_trip() {
        let d = dir("round-trip");
        let path = photo(&d, "a.png", 40, 30, 0);
        let mut cache = Cache::default();
        let print = cache.fingerprint(&path).unwrap();
        cache.save(&d.0).unwrap();

        assert_eq!(
            Cache::load(&d.0, std::slice::from_ref(&path)).get(&path),
            Some(print)
        );
    }

    #[test]
    fn every_field_survives_a_round_trip() {
        let d = dir("fields");
        let path = photo(&d, "a.png", 30, 40, 7);
        let stamp = stamp(&path).unwrap();
        // A portrait photo, a negative capture time and a hash with its top bit
        // set: the three things a sloppy format loses.
        let print = print(0xf000_0000_0000_000f, Some(-86_400), false);
        holding(&path, stamp, print).save(&d.0).unwrap();

        assert_eq!(
            Cache::load(&d.0, std::slice::from_ref(&path)).get(&path),
            Some(print)
        );
    }

    #[test]
    fn a_photo_with_no_capture_time_round_trips() {
        let d = dir("undated");
        let path = photo(&d, "a.png", 40, 30, 0);
        let print = print(1, None, true);
        holding(&path, stamp(&path).unwrap(), print)
            .save(&d.0)
            .unwrap();

        assert_eq!(
            Cache::load(&d.0, std::slice::from_ref(&path)).get(&path),
            Some(print)
        );
    }

    #[test]
    fn a_folder_with_no_store_loads_empty() {
        let d = dir("fresh");
        let path = photo(&d, "a.png", 40, 30, 0);
        assert!(Cache::load(&d.0, std::slice::from_ref(&path))
            .get(&path)
            .is_none());
    }

    #[test]
    fn the_store_holds_names_not_paths() {
        let d = dir("names");
        let path = photo(&d, "a.png", 40, 30, 0);
        holding(
            &path,
            stamp(&path).unwrap(),
            print(0xdead_beef, Some(12), true),
        )
        .save(&d.0)
        .unwrap();

        // Names, so moving or renaming the folder keeps the cache usable.
        let text = fs::read_to_string(d.0.join(".photo-viewer").join("fingerprints")).unwrap();
        assert!(text.ends_with(" a.png"), "{text}");
        assert!(!text.contains(&*d.0.to_string_lossy()), "{text}");
    }

    #[test]
    fn a_record_for_a_deleted_photo_is_dropped() {
        let d = dir("stale");
        let kept = photo(&d, "a.png", 40, 30, 0);
        let gone = photo(&d, "b.png", 40, 30, 9);
        let mut cache = Cache::default();
        cache.fingerprint(&kept).unwrap();
        cache.fingerprint(&gone).unwrap();
        cache.save(&d.0).unwrap();

        let loaded = Cache::load(&d.0, std::slice::from_ref(&kept));
        assert_eq!(loaded.entries.len(), 1);
        // And once saved, it is gone from the file too.
        loaded.save(&d.0).unwrap();
        let text = fs::read_to_string(d.0.join(".photo-viewer").join("fingerprints")).unwrap();
        assert!(!text.contains("b.png"), "{text}");
    }

    #[test]
    fn a_record_the_code_cannot_read_is_not_a_hit() {
        let d = dir("corrupt");
        let path = photo(&d, "a.png", 40, 30, 0);
        let dir_path = d.0.join(".photo-viewer");
        fs::create_dir_all(&dir_path).unwrap();
        let good = line("a.png", &stamp(&path).unwrap(), &print(5, None, true));
        // A blank line, a truncated record, a hash that is not hex, a shape that
        // is not a shape, and a record for nothing at all.
        let body =
            format!("\nnot a record\nzzzz 1 2 - l a.png\n1 1 2 - x a.png\n1 1 2 - l \n{good}");
        fs::write(dir_path.join("fingerprints"), body).unwrap();

        let loaded = Cache::load(&d.0, std::slice::from_ref(&path));
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.get(&path), Some(print(5, None, true)));
    }

    // --- Freshness ---

    #[test]
    fn a_hit_costs_no_decode() {
        let d = dir("no-decode");
        let path = photo(&d, "a.png", 40, 30, 0);
        let was = fs::metadata(&path).unwrap().modified().unwrap();
        let mut cache = Cache::default();
        let print = cache.fingerprint(&path).unwrap();

        // Replace the photo with bytes that cannot be decoded at all, then put
        // its stamp back. Anything that so much as opened the file would fail.
        let size = fs::metadata(&path).unwrap().len() as usize;
        fs::write(&path, vec![b'x'; size]).unwrap();
        touch(&path, was);

        assert_eq!(cache.fingerprint(&path), Some(print));
    }

    #[test]
    fn a_photo_touched_since_is_hashed_again() {
        let d = dir("touched");
        let path = photo(&d, "a.png", 40, 30, 0);
        let mut cache = Cache::default();
        cache.fingerprint(&path).unwrap();

        touch(
            &path,
            SystemTime::now() + std::time::Duration::from_secs(30),
        );
        assert_eq!(cache.get(&path), None);
    }

    #[test]
    fn a_photo_edited_to_the_same_second_is_hashed_again() {
        let d = dir("same-second");
        let path = photo(&d, "a.png", 40, 30, 0);
        let was = fs::metadata(&path).unwrap().modified().unwrap();
        let mut cache = Cache::default();
        cache.fingerprint(&path).unwrap();

        // The rotation case: same size, same second, different pixels. A stamp
        // that only counted whole seconds would call this unchanged.
        let size = fs::metadata(&path).unwrap().len() as usize;
        fs::write(&path, vec![b'x'; size]).unwrap();
        touch(&path, was + std::time::Duration::from_millis(1));
        assert_eq!(cache.get(&path), None);
    }

    #[test]
    fn a_photo_that_changed_size_is_hashed_again() {
        let d = dir("resized");
        let path = photo(&d, "a.png", 40, 30, 0);
        let was = fs::metadata(&path).unwrap().modified().unwrap();
        let mut cache = Cache::default();
        cache.fingerprint(&path).unwrap();

        fs::write(&path, b"shorter").unwrap();
        touch(&path, was);
        assert_eq!(cache.get(&path), None);
    }

    #[test]
    fn a_vanished_photo_is_not_a_hit() {
        let d = dir("vanished");
        let path = photo(&d, "a.png", 40, 30, 0);
        let mut cache = Cache::default();
        cache.fingerprint(&path).unwrap();

        fs::remove_file(&path).unwrap();
        assert_eq!(cache.get(&path), None);
    }

    #[test]
    fn a_photo_that_cannot_be_read_is_not_remembered() {
        let d = dir("unreadable");
        let path = d.0.join("a.png");
        fs::write(&path, b"not an image").unwrap();

        let mut cache = Cache::default();
        assert_eq!(cache.fingerprint(&path), None);
        assert!(cache.entries.is_empty());
        assert_eq!(cache.fingerprint(&d.0.join("missing.png")), None);
    }

    #[test]
    fn the_second_pass_over_a_folder_agrees_with_the_first() {
        let d = dir("second-pass");
        let paths: Vec<PathBuf> = (0..3)
            .map(|i| photo(&d, &format!("{i}.png"), 40, 30, i as u8 * 40))
            .collect();

        let mut cache = Cache::default();
        let first: Vec<_> = paths.iter().map(|p| cache.fingerprint(p)).collect();
        cache.save(&d.0).unwrap();

        let mut loaded = Cache::load(&d.0, &paths);
        let second: Vec<_> = paths.iter().map(|p| loaded.fingerprint(p)).collect();
        assert_eq!(first, second);
        assert!(first.iter().all(Option::is_some));
    }
}
