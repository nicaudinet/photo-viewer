//! What makes two photos "the same shot", and the runs that follow from it.
//!
//! This is the whole of the grouping policy (`GROUP_MODE_PLAN.md`), as pure
//! functions over a tiny per-photo summary. Nothing here reads the library,
//! touches the wall, or decides how a stack is drawn — it answers one question,
//! "do these two belong together?", and one that follows from it, "so where are
//! the runs?".
//!
//! ## Why two signals
//!
//! A [`Fingerprint`] carries a perceptual hash *and* a capture time, and
//! [`links`] wants both. Each alone is wrong in a way the other covers:
//!
//! - **Time alone** stacks a burst of a fast-moving subject whose frames look
//!   nothing alike, and cannot see that you took the same photo twice ten
//!   minutes apart.
//! - **Pixels alone** stacks two unrelated shots of a white wall, or two dark
//!   frames, that happen to be neighbours.
//!
//! The time term is *skipped* rather than failed when either photo has no
//! timestamp, which is what keeps PNGs and exported files working.

use std::ops::Range;
use std::path::Path;

use ::image::imageops::FilterType;
use ::image::GenericImageView;

use crate::core::{exif, imaging};

/// Width of the dHash grid's comparisons, and so the side of the bit square:
/// 8 x 8 comparisons is 64 bits, which is one `u64`.
const HASH_SIDE: u32 = 8;

/// How wide a decode the hash is built from.
///
/// Far larger than the 9 x 8 grid it becomes, because it is not the size that
/// matters but the smoothing: downsampling a real decode averages away the
/// sensor noise and JPEG ringing that would otherwise flip low-order bits
/// between two shots of the same thing. `jpeg-decoder` cannot decode below 1/8
/// anyway, so asking for less would buy nothing.
const HASH_SOURCE_WIDTH: u32 = 32;

/// How far apart two capture times may be, in seconds.
///
/// Generous on purpose. It is not there to define a burst — the hash does that
/// — but to stop two visually similar photos from opposite ends of an afternoon
/// being called the same shot.
const MAX_GAP: i64 = 60;

/// How much looser the drift guard is than the pairwise distance.
const DRIFT_FACTOR: f32 = 1.6;

/// What grouping knows about one photo.
///
/// Deliberately tiny and `Copy`: one of these exists per image in the folder,
/// they are compared in a tight loop every time the threshold moves, and they
/// are what phase 2 writes to the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    /// 8 x 8 difference hash of the photo as displayed.
    pub dhash: u64,
    /// When the shutter fired, if the file says. See [`exif::taken_at`] for what
    /// the number does and does not mean.
    pub taken: Option<i64>,
    /// Whether it is at least as wide as it is tall, as displayed. Square counts
    /// as landscape; it has to count as something, and a square photo sits
    /// happily beside a wide one.
    pub landscape: bool,
}

/// How alike two photos may be and still be told apart.
///
/// A rung on a fixed ladder rather than a raw number, because the right value is
/// a property of *your* photos — how much you move between frames — and there is
/// no way to find it except by turning the dial and watching the wall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Threshold(usize);

impl Threshold {
    /// Hamming distances out of 64. Spaced wider as they loosen, because a bit
    /// of slack matters much less once there is a lot of it.
    const LADDER: [u32; 5] = [4, 7, 10, 14, 18];

    /// The distance two adjacent photos must be within to link.
    pub fn distance(self) -> u32 {
        Self::LADDER[self.0]
    }

    /// The distance a photo must be within of its group's *first* member.
    ///
    /// Loose enough that a burst with a hand-shake drift stays whole, tight
    /// enough that a slow pan breaks into several stacks instead of one.
    pub fn drift(self) -> u32 {
        (self.distance() as f32 * DRIFT_FACTOR).round() as u32
    }

    /// One rung looser, or the loosest if there is no more ladder. Saturating
    /// rather than wrapping: `+` held down should end at "as loose as it goes",
    /// not spring back to the tightest rung.
    pub fn looser(self) -> Self {
        Self((self.0 + 1).min(Self::LADDER.len() - 1))
    }

    pub fn tighter(self) -> Self {
        Self(self.0.saturating_sub(1))
    }

    /// Which rung this is, for the mode bar — which grows one in phase 4.
    #[allow(dead_code)]
    pub fn rung(self) -> (usize, usize) {
        (self.0 + 1, Self::LADDER.len())
    }
}

impl Default for Threshold {
    /// The middle of the ladder — the rung to start turning the dial from.
    fn default() -> Self {
        Self(2)
    }
}

/// Summarise `path` for grouping. `None` if it cannot be read or decoded.
pub fn fingerprint(path: &Path) -> Option<Fingerprint> {
    let source = imaging::oriented_source(path, HASH_SOURCE_WIDTH).ok()?;
    let (width, height) = source.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    Some(Fingerprint {
        dhash: dhash(&source),
        taken: exif::taken_at(path),
        landscape: width >= height,
    })
}

/// The 8 x 8 difference hash: is each cell brighter than the one to its right?
///
/// Squashed to a fixed grid without regard to aspect ratio, which is the usual
/// dHash and the right thing here — a photo and a lightly cropped version of it
/// should read as the same shot. Nothing is lost by it, because [`links`]
/// refuses to pair a portrait with a landscape separately.
///
/// Comparing *neighbours* rather than a mean is what makes this robust to the
/// thing that actually varies between two frames of a burst: exposure. Brighten
/// the whole photo and every comparison holds.
fn dhash(source: &::image::DynamicImage) -> u64 {
    // One extra column: 9 cells across give the 8 comparisons a row needs.
    let grid = ::image::imageops::resize(
        &source.to_luma8(),
        HASH_SIDE + 1,
        HASH_SIDE,
        FilterType::Triangle,
    );
    let mut bits = 0u64;
    for y in 0..HASH_SIDE {
        for x in 0..HASH_SIDE {
            let left = grid.get_pixel(x, y)[0];
            let right = grid.get_pixel(x + 1, y)[0];
            bits = (bits << 1) | u64::from(left > right);
        }
    }
    bits
}

/// How many of the 64 comparisons two photos disagree on.
pub fn distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Whether `candidate` continues the run that `first` started and `previous`
/// currently ends.
///
/// Three tests, cheapest first:
///
/// 1. **Orientation must match.** Free, and it kills a whole class of false
///    positives.
/// 2. **`previous` and `candidate` must be alike**, and — the drift guard —
///    `candidate` must still resemble `first`. Without the second test a slow
///    pan links each frame to the next while its ends have nothing in common,
///    and the whole pan collapses into one stack.
/// 3. **They must be close in time**, when both say when they were taken.
pub fn links(
    previous: &Fingerprint,
    candidate: &Fingerprint,
    first: &Fingerprint,
    threshold: Threshold,
) -> bool {
    if previous.landscape != candidate.landscape {
        return false;
    }
    if distance(previous.dhash, candidate.dhash) > threshold.distance() {
        return false;
    }
    if distance(first.dhash, candidate.dhash) > threshold.drift() {
        return false;
    }
    match (previous.taken, candidate.taken) {
        // Skipped, not failed: a photo with no timestamp still groups on its
        // pixels, which is what keeps PNGs and stripped exports working.
        (Some(a), Some(b)) => (b - a).abs() <= MAX_GAP,
        _ => true,
    }
}

/// The runs of adjacent photos that belong together, as index ranges into
/// `prints`, in order and non-overlapping.
///
/// Only runs of two or more are returned — a stack of one is a photo. A `None`
/// (an image that could not be read) links to nothing and so breaks any run it
/// falls in, which is the honest answer: nothing is known about it.
///
/// Only neighbours are ever compared. This is not a clustering problem and must
/// not become one: photos in a folder already sit in a meaningful order, and a
/// burst is by definition contiguous within it.
pub fn chain(prints: &[Option<Fingerprint>], threshold: Threshold) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut start = 0;
    while start < prints.len() {
        let Some(first) = prints[start].as_ref() else {
            start += 1;
            continue;
        };
        let mut end = start + 1;
        while end < prints.len() {
            let (Some(previous), Some(candidate)) =
                (prints[end - 1].as_ref(), prints[end].as_ref())
            else {
                break;
            };
            if !links(previous, candidate, first, threshold) {
                break;
            }
            end += 1;
        }
        if end - start >= 2 {
            runs.push(start..end);
        }
        // A run that failed at its first link still consumed one photo.
        start = end.max(start + 1);
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;
    use std::path::PathBuf;

    // --- Fingerprinting real files ---

    fn unique_path(tag: &str, ext: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("pv-fp-{tag}-{}-{n}.{ext}", std::process::id()))
    }

    /// A stand-in for a photograph: smooth, low-frequency brightness over
    /// fractional coordinates, panned sideways by `shift` of its own width.
    ///
    /// Both properties are load-bearing. **Low-frequency**, because fine detail
    /// aliases differently at every resolution and a fixture full of it would
    /// test the sampler rather than the hash — real photographs are mostly
    /// large soft shapes. **Asymmetric** between the axes (one cycle across,
    /// two down), so that a quarter turn genuinely changes the image.
    fn write_image(tag: &str, ext: &str, w: u32, h: u32, shift: f32) -> PathBuf {
        let path = unique_path(tag, ext);
        let mut buf = ::image::RgbImage::new(w, h);
        for (x, y, px) in buf.enumerate_pixels_mut() {
            let u = x as f32 / w as f32 + shift;
            let t = y as f32 / h as f32;
            let level = 0.5 + 0.25 * (u * TAU).cos() + 0.15 * (t * 2.0 * TAU).sin();
            let v = (level.clamp(0.0, 1.0) * 255.0) as u8;
            *px = ::image::Rgb([v, v, v]);
        }
        ::image::DynamicImage::ImageRgb8(buf).save(&path).unwrap();
        path
    }

    #[test]
    fn a_photo_is_identical_to_itself() {
        let path = write_image("self", "jpg", 800, 600, 0.0);
        let a = fingerprint(&path).unwrap();
        let b = fingerprint(&path).unwrap();
        assert_eq!(distance(a.dhash, b.dhash), 0);
        assert!(a.landscape);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_same_image_hashes_the_same_through_a_different_codec() {
        // The one that matters most: a JPEG's ringing and a PNG's exactness
        // must not be enough to call two copies of a photo different shots.
        let jpg = write_image("codec", "jpg", 800, 600, 0.0);
        let png = write_image("codec", "png", 800, 600, 0.0);
        let d = distance(
            fingerprint(&jpg).unwrap().dhash,
            fingerprint(&png).unwrap().dhash,
        );
        assert!(d <= Threshold::default().distance(), "distance {d}");
        let _ = std::fs::remove_file(&jpg);
        let _ = std::fs::remove_file(&png);
    }

    #[test]
    fn a_photo_at_a_different_size_still_matches() {
        let big = write_image("size-big", "jpg", 1600, 1200, 0.0);
        let small = write_image("size-small", "jpg", 400, 300, 0.0);
        let d = distance(
            fingerprint(&big).unwrap().dhash,
            fingerprint(&small).unwrap().dhash,
        );
        assert!(d <= Threshold::default().distance(), "distance {d}");
        let _ = std::fs::remove_file(&big);
        let _ = std::fs::remove_file(&small);
    }

    #[test]
    fn a_photo_the_camera_moved_a_little_still_matches() {
        // What a burst actually looks like: the same scene, panned slightly.
        let still = write_image("pan-still", "jpg", 800, 600, 0.0);
        let moved = write_image("pan-moved", "jpg", 800, 600, 0.02);
        let d = distance(
            fingerprint(&still).unwrap().dhash,
            fingerprint(&moved).unwrap().dhash,
        );
        assert!(d <= Threshold::default().distance(), "distance {d}");
        let _ = std::fs::remove_file(&still);
        let _ = std::fs::remove_file(&moved);
    }

    #[test]
    fn an_unrelated_photo_is_far_away() {
        let a = write_image("far-a", "png", 400, 300, 0.0);
        let b = unique_path("far-b", "png");
        // Horizontal bands: the opposite structure, so most comparisons flip.
        let mut buf = ::image::RgbImage::new(400, 300);
        for (_, y, px) in buf.enumerate_pixels_mut() {
            let v = if (y * 6 / 300) % 2 == 0 { 20 } else { 235 };
            *px = ::image::Rgb([v, v, v]);
        }
        ::image::DynamicImage::ImageRgb8(buf).save(&b).unwrap();

        let d = distance(
            fingerprint(&a).unwrap().dhash,
            fingerprint(&b).unwrap().dhash,
        );
        assert!(d > Threshold::LADDER[4], "distance {d}");
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    fn a_portrait_photo_says_so() {
        let path = write_image("portrait", "png", 300, 900, 0.0);
        assert!(!fingerprint(&path).unwrap().landscape);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_square_photo_counts_as_landscape() {
        // It has to count as something, and a square sits happily beside a wide
        // photo. What matters is only that the answer is stable.
        let path = write_image("square", "png", 400, 400, 0.0);
        assert!(fingerprint(&path).unwrap().landscape);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_rotated_photo_is_hashed_the_way_it_is_shown() {
        let path = write_image("rotated", "jpg", 800, 600, 0.0);
        let before = fingerprint(&path).unwrap();
        imaging::rotate_in_place(&path, true).unwrap();
        let after = fingerprint(&path).unwrap();

        // Rotation rewrites one EXIF tag and moves no pixels, so a hasher
        // reading stored pixels would see no change at all. Reading it as
        // displayed, the photo is now portrait and looks quite different.
        assert!(before.landscape && !after.landscape);
        assert!(distance(before.dhash, after.dhash) > Threshold::default().distance());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_photo_with_no_capture_time_is_still_fingerprinted() {
        let path = write_image("timed", "jpg", 400, 300, 0.0);
        // `write_image` goes through the `image` crate, which writes no EXIF.
        // The hash is what carries the photo; the timestamp is a bonus.
        assert_eq!(fingerprint(&path).unwrap().taken, None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_unreadable_file_has_no_fingerprint() {
        assert_eq!(fingerprint(Path::new("/nonexistent-photo.jpg")), None);
        let path = unique_path("garbage", "jpg");
        std::fs::write(&path, b"not an image").unwrap();
        assert_eq!(fingerprint(&path), None);
        let _ = std::fs::remove_file(&path);
    }

    // --- The policy, over hand-made fingerprints ---

    /// A fingerprint whose hash is `bits` ones — so `n` and `m` ones are
    /// `|n - m|` apart, which makes the distances in these tests readable.
    fn fp(bits: u32) -> Fingerprint {
        Fingerprint {
            dhash: if bits == 64 {
                u64::MAX
            } else {
                (1u64 << bits) - 1
            },
            taken: None,
            landscape: true,
        }
    }

    fn at(bits: u32, taken: i64) -> Fingerprint {
        Fingerprint {
            taken: Some(taken),
            ..fp(bits)
        }
    }

    fn linked(previous: &Fingerprint, candidate: &Fingerprint) -> bool {
        links(previous, candidate, previous, Threshold::default())
    }

    #[test]
    fn identical_photos_link() {
        assert!(linked(&fp(0), &fp(0)));
    }

    #[test]
    fn the_threshold_is_inclusive() {
        let d = Threshold::default().distance();
        assert!(linked(&fp(0), &fp(d)));
        assert!(!linked(&fp(0), &fp(d + 1)));
    }

    #[test]
    fn a_portrait_never_links_to_a_landscape() {
        let portrait = Fingerprint {
            landscape: false,
            ..fp(0)
        };
        // Pixel-identical, and still not the same shot.
        assert!(!linked(&fp(0), &portrait));
    }

    #[test]
    fn photos_taken_far_apart_do_not_link_however_alike() {
        assert!(linked(&at(0, 1_000), &at(0, 1_000 + MAX_GAP)));
        assert!(!linked(&at(0, 1_000), &at(0, 1_000 + MAX_GAP + 1)));
        // And the same backwards: order of capture is not order in the folder.
        assert!(!linked(&at(0, 1_000 + MAX_GAP + 1), &at(0, 1_000)));
    }

    #[test]
    fn the_time_term_is_skipped_when_either_timestamp_is_missing() {
        // PNGs, exports, anything stripped: these still group on pixels rather
        // than being refused outright.
        assert!(linked(&fp(0), &at(0, 999_999)));
        assert!(linked(&at(0, 999_999), &fp(0)));
    }

    #[test]
    fn the_drift_guard_stops_a_pan_becoming_one_stack() {
        let threshold = Threshold::default();
        let first = fp(0);
        let previous = fp(threshold.drift());
        let candidate = fp(threshold.drift() + 1);

        // Each frame is a hair from the one before it — well inside the
        // pairwise threshold — but the run has now travelled too far from where
        // it started.
        assert!(distance(previous.dhash, candidate.dhash) <= threshold.distance());
        assert!(!links(&previous, &candidate, &first, threshold));
        assert!(links(&previous, &candidate, &previous, threshold));
    }

    // --- Chaining ---

    fn prints(bits: &[u32]) -> Vec<Option<Fingerprint>> {
        bits.iter().map(|b| Some(fp(*b))).collect()
    }

    #[test]
    fn a_burst_becomes_one_run() {
        let prints = prints(&[0, 1, 2, 3]);
        assert_eq!(chain(&prints, Threshold::default()), vec![0..4]);
    }

    #[test]
    fn a_lone_photo_is_not_a_stack() {
        // Two similar, then one far away, then two similar.
        let prints = prints(&[0, 1, 40, 63, 63]);
        assert_eq!(chain(&prints, Threshold::default()), vec![0..2, 3..5]);
    }

    #[test]
    fn nothing_similar_means_no_stacks() {
        let prints = prints(&[0, 20, 40, 60]);
        assert!(chain(&prints, Threshold::default()).is_empty());
    }

    #[test]
    fn a_pan_breaks_into_several_stacks() {
        // Three bits per step: inside the pairwise threshold every time, so
        // only the drift guard can stop this becoming one run of ten.
        let prints = prints(&[0, 3, 6, 9, 12, 15, 18, 21, 24, 27]);
        let runs = chain(&prints, Threshold::default());
        assert!(runs.len() > 1, "the whole pan collapsed into {runs:?}");
        assert_eq!(runs.first().unwrap().start, 0);
        // Non-overlapping and in order, which the library fold relies on.
        for pair in runs.windows(2) {
            assert!(pair[0].end <= pair[1].start);
        }
    }

    #[test]
    fn an_unreadable_photo_breaks_the_run() {
        // Nothing is known about it, so it belongs with nothing.
        let mut prints = prints(&[0, 1, 2, 3, 4]);
        prints[2] = None;
        assert_eq!(chain(&prints, Threshold::default()), vec![0..2, 3..5]);
    }

    #[test]
    fn a_looser_threshold_makes_bigger_stacks() {
        // Same photos, same coverage — what the dial changes is how many piles
        // they fall into, which is the whole reason it exists.
        let prints = prints(&[0, 6, 12, 18]);
        let tight = chain(&prints, Threshold::default().tighter());
        let loose = chain(&prints, Threshold::default().looser().looser());
        assert_eq!(tight, vec![0..2, 2..4]);
        assert_eq!(loose, vec![0..4]);
    }

    #[test]
    fn chaining_nothing_finds_nothing() {
        assert!(chain(&[], Threshold::default()).is_empty());
        assert!(chain(&prints(&[0]), Threshold::default()).is_empty());
    }

    // --- The ladder ---

    #[test]
    fn the_ladder_stops_at_both_ends() {
        // Held down, `+` should end at "as loose as it goes" rather than
        // springing back to the tightest rung.
        let mut t = Threshold::default();
        for _ in 0..10 {
            t = t.looser();
        }
        assert_eq!(t.distance(), *Threshold::LADDER.last().unwrap());
        for _ in 0..10 {
            t = t.tighter();
        }
        assert_eq!(t.distance(), Threshold::LADDER[0]);
    }

    #[test]
    fn the_drift_guard_is_looser_than_the_pairwise_distance() {
        let mut t = Threshold(0);
        for _ in 0..Threshold::LADDER.len() {
            assert!(t.drift() > t.distance());
            t = t.looser();
        }
    }

    #[test]
    fn the_rung_is_reported_from_one() {
        assert_eq!(Threshold::default().rung(), (3, 5));
        assert_eq!(Threshold::default().tighter().tighter().rung(), (1, 5));
    }
}
