//! Runs of near-identical photos, folded into the visible list as stacks.
//!
//! [`crate::core::fingerprint`] decides which photos belong together; this turns
//! that answer into the shape the library needs — a list with each run collapsed
//! to its first member, and a way back from that member to the whole run.
//!
//! A [`Grouping`] holds the fingerprints because it is rebuilt far more often
//! than they are: every trash, filter change and turn of the threshold dial
//! re-chains the folder, and none of those change a single hash.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::core::fingerprint::{self, Fingerprint, Threshold};

/// The stacks over one folder, and what they were built from.
#[derive(Debug, Clone)]
pub struct Grouping {
    /// Every fingerprint that could be taken, by path. A photo missing from
    /// here is one nothing is known about; it stacks with nothing.
    prints: HashMap<PathBuf, Fingerprint>,
    threshold: Threshold,
    /// Members of each stack, by the photo that stands for it. The
    /// representative is its own first member.
    stacks: HashMap<PathBuf, Vec<PathBuf>>,
    /// The other way round: which stack a photo was folded into.
    head: HashMap<PathBuf, PathBuf>,
}

impl Grouping {
    /// Grouping over `prints`, at the middle rung of the ladder. Empty until
    /// [`rebuild`](Self::rebuild) is handed a list of photos.
    pub fn new(prints: HashMap<PathBuf, Fingerprint>) -> Self {
        Self {
            prints,
            threshold: Threshold::default(),
            stacks: HashMap::new(),
            head: HashMap::new(),
        }
    }

    pub fn threshold(&self) -> Threshold {
        self.threshold
    }

    pub fn set_threshold(&mut self, threshold: Threshold) {
        self.threshold = threshold;
    }

    /// How many stacks there are, for the mode bar.
    pub fn len(&self) -> usize {
        self.stacks.len()
    }

    /// The photos in the stack `path` stands for, or `None` if it stands only
    /// for itself.
    pub fn stack(&self, path: &Path) -> Option<&[PathBuf]> {
        self.stacks.get(path).map(Vec::as_slice)
    }

    /// The photo standing for the stack `path` was folded into, if it was.
    pub fn head_of(&self, path: &Path) -> Option<&PathBuf> {
        self.head.get(path)
    }

    /// Chain `photos` — the whole visible folder, in order — into stacks, and
    /// return the one photo each tile is drawn from.
    ///
    /// Recomputed from scratch every time rather than patched. A stack is a
    /// judgement about the photos beside it, so a photo leaving changes the
    /// answer for its neighbours; there is no edit small enough to be worth
    /// tracking, and a pass over a list of hashes costs nothing.
    pub(crate) fn rebuild(&mut self, photos: &[PathBuf]) -> Vec<PathBuf> {
        self.stacks.clear();
        self.head.clear();

        let prints: Vec<Option<Fingerprint>> = photos
            .iter()
            .map(|path| self.prints.get(path).copied())
            .collect();
        let runs = fingerprint::chain(&prints, self.threshold);

        let mut tiles = Vec::with_capacity(photos.len());
        let mut runs = runs.into_iter().peekable();
        let mut at = 0;
        while at < photos.len() {
            // `chain` returns its runs in order and non-overlapping, so the
            // next one is either starting here or starting later.
            let run = runs.next_if(|run| run.start == at);
            let representative = photos[at].clone();
            tiles.push(representative.clone());
            match run {
                Some(run) => {
                    for photo in &photos[run.clone()] {
                        self.head.insert(photo.clone(), representative.clone());
                    }
                    self.stacks
                        .insert(representative, photos[run.clone()].to_vec());
                    at = run.end;
                }
                None => at += 1,
            }
        }
        tiles
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fingerprint whose hash is `bits` ones, so two of them are `|n - m|`
    /// apart and the distances in these tests can be read off the numbers.
    fn print(bits: u32) -> Fingerprint {
        Fingerprint {
            dhash: (1u64 << bits) - 1,
            taken: None,
            landscape: true,
        }
    }

    fn photos(n: usize) -> Vec<PathBuf> {
        (0..n)
            .map(|i| PathBuf::from(format!("/photos/{i}.jpg")))
            .collect()
    }

    /// A grouping over `bits.len()` photos with those fingerprints, already
    /// rebuilt. Returns the photos and the tiles it left.
    fn grouped(bits: &[u32]) -> (Vec<PathBuf>, Grouping, Vec<PathBuf>) {
        let photos = photos(bits.len());
        let prints = photos
            .iter()
            .zip(bits)
            .map(|(path, bits)| (path.clone(), print(*bits)))
            .collect();
        let mut grouping = Grouping::new(prints);
        let tiles = grouping.rebuild(&photos);
        (photos, grouping, tiles)
    }

    #[test]
    fn a_run_collapses_to_its_first_photo() {
        let (photos, grouping, tiles) = grouped(&[0, 1, 2, 40]);
        assert_eq!(tiles, vec![photos[0].clone(), photos[3].clone()]);
        assert_eq!(grouping.stack(&photos[0]).unwrap(), &photos[..3]);
        assert_eq!(grouping.len(), 1);
    }

    #[test]
    fn a_photo_that_stacks_with_nothing_is_not_a_stack() {
        let (photos, grouping, tiles) = grouped(&[0, 30, 60]);
        assert_eq!(tiles, photos);
        assert_eq!(grouping.stack(&photos[1]), None);
        assert_eq!(grouping.len(), 0);
    }

    #[test]
    fn several_runs_keep_their_order() {
        let (photos, grouping, tiles) = grouped(&[0, 1, 30, 31, 60, 61]);
        assert_eq!(
            tiles,
            vec![photos[0].clone(), photos[2].clone(), photos[4].clone()]
        );
        assert_eq!(grouping.len(), 3);
    }

    #[test]
    fn a_run_between_two_lone_photos_lands_in_the_middle() {
        let (photos, _, tiles) = grouped(&[0, 30, 31, 32, 62]);
        assert_eq!(
            tiles,
            vec![photos[0].clone(), photos[1].clone(), photos[4].clone()]
        );
    }

    #[test]
    fn a_photo_knows_which_stack_swallowed_it() {
        let (photos, grouping, _) = grouped(&[0, 1, 2, 40]);
        // What the cursor follows when the photo it was on stops being a tile.
        assert_eq!(grouping.head_of(&photos[2]), Some(&photos[0]));
        assert_eq!(grouping.head_of(&photos[0]), Some(&photos[0]));
        assert_eq!(grouping.head_of(&photos[3]), None);
    }

    #[test]
    fn a_photo_with_no_fingerprint_stacks_with_nothing() {
        let photos = photos(3);
        // Only the outer two are known, and they are not adjacent.
        let prints = [(photos[0].clone(), print(0)), (photos[2].clone(), print(0))]
            .into_iter()
            .collect();
        let mut grouping = Grouping::new(prints);
        assert_eq!(grouping.rebuild(&photos), photos);
    }

    #[test]
    fn rebuilding_forgets_the_stacks_it_had() {
        let (photos, mut grouping, _) = grouped(&[0, 1, 2, 40]);
        // The middle of the run has gone, which is what a trash looks like.
        let left = vec![photos[0].clone(), photos[3].clone()];
        assert_eq!(grouping.rebuild(&left), left);
        assert_eq!(grouping.stack(&photos[0]), None);
        assert_eq!(grouping.head_of(&photos[1]), None);
    }

    #[test]
    fn turning_the_dial_re_chains_the_same_photos() {
        let (photos, mut grouping, tiles) = grouped(&[0, 6, 12, 18]);
        assert_eq!(tiles.len(), 2);

        grouping.set_threshold(Threshold::default().looser().looser());
        assert_eq!(grouping.rebuild(&photos), vec![photos[0].clone()]);
        assert_eq!(grouping.stack(&photos[0]).unwrap().len(), 4);

        grouping.set_threshold(Threshold::default().tighter());
        assert_eq!(grouping.rebuild(&photos).len(), 2);
    }

    #[test]
    fn grouping_nothing_leaves_nothing() {
        let (_, mut grouping, _) = grouped(&[0, 1]);
        assert!(grouping.rebuild(&[]).is_empty());
        assert_eq!(grouping.len(), 0);
    }
}
