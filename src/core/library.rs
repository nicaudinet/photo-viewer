//! The GUI-free domain core: the image collection for one open directory, the
//! user's selection within it, and the tags over it.
//!
//! Favourites used to be a hard-coded set here with a cache file of its own.
//! It is now one tag among however many (see [`crate::core::tags`]) applied to a
//! selection, and the filter that shows only tagged images narrows `paths`
//! rather than narrowing where thumbnails are drawn — see
//! `SELECT_MODE_PLAN.md` phase 6 for why that difference matters.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::fingerprint::Threshold;
use crate::core::grouping::Grouping;
use crate::core::pointed_list::PointedList;
use crate::core::tags::{self, Tags};

/// Extensions we treat as images (compared case-insensitively, without the dot).
pub const IMAGE_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

/// Errors from domain operations. Everything that touches the filesystem or
/// violates an invariant surfaces here instead of panicking.
#[derive(Debug)]
pub enum LibraryError {
    /// The directory passed to `load` does not exist.
    DirNotFound(PathBuf),
    /// The path passed to `load` exists but is not a directory.
    NotADirectory(PathBuf),
    /// Underlying I/O failure.
    Io(std::io::Error),
}

impl fmt::Display for LibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibraryError::DirNotFound(p) => {
                write!(f, "Directory {} does not exist", p.display())
            }
            LibraryError::NotADirectory(p) => {
                write!(f, "{} is not a directory", p.display())
            }
            LibraryError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for LibraryError {}

impl From<std::io::Error> for LibraryError {
    fn from(e: std::io::Error) -> Self {
        LibraryError::Io(e)
    }
}

/// The full mutable domain state for one open directory.
#[derive(Debug, Clone)]
pub struct Library {
    /// Every image in the directory, in scan order.
    pub all: Vec<PathBuf>,
    /// The images on screen: `all`, or the subset carrying [`Library::filter`].
    ///
    /// A filter narrows *this list* rather than being applied where images are
    /// drawn. That is the whole of why a filter cannot hide a selected photo:
    /// there is no list in which a hidden image can be reached, so nothing —
    /// not `Cmd+A`, not a painted range, not a batch — can act on one. The
    /// alternative, filtering at the point of drawing, leaves every one of
    /// those free to touch images the user cannot see.
    pub paths: PointedList<PathBuf>,
    /// The directory `paths` was scanned from. The move/copy picker opens here,
    /// and it is what tells a destination inside the library from one outside.
    pub image_dir: PathBuf,
    /// The images the user has selected, as paths rather than indices:
    /// deleting or moving files renumbers `paths`, and an index-keyed selection
    /// would silently come to mean different images.
    ///
    /// Deliberately not persisted — it is a scratch buffer, not a judgement
    /// about the photos. It lives here rather than in the wall so that it
    /// survives a switch to the single view and back.
    pub selection: HashSet<PathBuf>,
    /// The user's tags over these images, as loaded from — and written back to
    /// — `<image_dir>/.photo-viewer/tags/`.
    pub tags: Tags,
    /// The tag `paths` is narrowed to, if any.
    pub filter: Option<String>,
    /// The stacks over the visible photos, when grouping is on.
    ///
    /// This is the one thing that makes `paths` shorter than what the wall
    /// stands for: with grouping on it holds one entry per *tile*, a run of
    /// near-identical photos contributing only the first of them. `all` and
    /// `selection` stay in photos throughout — see [`Library::members`].
    pub grouping: Option<Grouping>,
}

impl Library {
    // --- Navigation (delegates to the PointedList) ---

    pub fn current(&self) -> &PathBuf {
        self.paths.current()
    }

    pub fn prev(&mut self) {
        self.paths.prev();
    }

    pub fn next(&mut self) {
        self.paths.next();
    }

    pub fn goto(&mut self, index: usize) -> bool {
        self.paths.goto(index)
    }

    // --- Stacks ---

    /// Every photo on the wall, in order.
    ///
    /// One entry per *photo*, where `paths` is one per *tile*: the two are the
    /// same list unless grouping is on. Anything that acts on files rather than
    /// on what is drawn wants this one.
    pub fn photos(&self) -> impl Iterator<Item = &PathBuf> {
        self.all.iter().filter(|p| self.shows(p))
    }

    /// The photos the tile at `path` stands for: a stack's members, or — for a
    /// plain thumbnail — the photo itself.
    pub fn members(&self, path: &Path) -> Vec<PathBuf> {
        match self.grouping.as_ref().and_then(|g| g.stack(path)) {
            Some(members) => members.to_vec(),
            None => vec![path.to_path_buf()],
        }
    }

    /// How many photos the tile at `path` stands for. One, unless it is a
    /// stack — which is what the tile's count badge shows, and what a
    /// confirmation has to count in.
    pub fn stack_size(&self, path: &Path) -> usize {
        self.grouping
            .as_ref()
            .and_then(|g| g.stack(path))
            .map_or(1, <[PathBuf]>::len)
    }

    // --- Selection ---

    /// Whether this exact photo is selected. A tile standing for several wants
    /// [`selected`](Self::selected) instead.
    pub fn is_selected(&self, path: &Path) -> bool {
        self.selection.contains(path)
    }

    /// How much of what the tile at `path` stands for is selected.
    pub fn selected(&self, path: &Path) -> Selected {
        let Some(members) = self.grouping.as_ref().and_then(|g| g.stack(path)) else {
            return if self.is_selected(path) {
                Selected::All
            } else {
                Selected::None
            };
        };
        match members.iter().filter(|p| self.is_selected(p)).count() {
            0 => Selected::None,
            n if n == members.len() => Selected::All,
            _ => Selected::Some,
        }
    }

    /// Put everything the tile at `path` stands for into the selection, or take
    /// all of it out.
    ///
    /// Every path that reaches the selection goes through here, which is why
    /// nothing downstream — not the batch queue, not trash, not the move — ever
    /// has to know that stacks exist. They see real photos because there is
    /// nothing else to see.
    fn set_selected(&mut self, path: &Path, on: bool) {
        for member in self.members(path) {
            if on {
                self.selection.insert(member);
            } else {
                self.selection.remove(&member);
            }
        }
    }

    /// Add or remove the tile at `index`, whichever it isn't already.
    ///
    /// A half-selected stack fills up rather than emptying, for the same reason
    /// [`toggle_tag`](Self::toggle_tag) works that way: one keypress over a
    /// mixed group has to leave it agreeing, or nobody can predict the result.
    pub fn toggle_selected(&mut self, index: usize) {
        let Some(path) = self.paths.iter().nth(index).cloned() else {
            return;
        };
        let on = self.selected(&path) != Selected::All;
        self.set_selected(&path, on);
    }

    /// Apply `op` to every image in the inclusive index range between `a` and
    /// `b`. The two ends may be given in either order — a range painted upwards
    /// covers the same images as the same range painted downwards.
    pub fn apply_range(&mut self, a: usize, b: usize, op: RangeOp) {
        let (lo, hi) = (a.min(b), a.max(b));
        let paths: Vec<PathBuf> = self
            .paths
            .iter()
            .skip(lo)
            .take(hi + 1 - lo)
            .cloned()
            .collect();
        for path in paths {
            self.set_selected(&path, op == RangeOp::Add);
        }
    }

    pub fn select_all(&mut self) {
        self.selection = self.photos().cloned().collect();
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    // --- Tags ---

    pub fn is_tagged(&self, tag: &str, path: &Path) -> bool {
        self.tags.get(tag).is_some_and(|set| set.contains(path))
    }

    /// Add `tag` to every path in `paths`, or take it off all of them if they
    /// all carry it already.
    ///
    /// Toggling a group means the group ends up agreeing, which is the only
    /// reading that stays useful: with per-image toggling, one keypress over a
    /// mixed selection would invert it into a differently-mixed one and the
    /// user would have to work out what happened.
    ///
    /// Returns whether they are tagged now.
    pub fn toggle_tag(&mut self, tag: &str, paths: &[PathBuf]) -> bool {
        let adding = !paths.iter().all(|p| self.is_tagged(tag, p));
        if adding {
            let set = self.tags.entry(tag.to_string()).or_default();
            set.extend(paths.iter().cloned());
        } else if let Some(set) = self.tags.get_mut(tag) {
            set.retain(|p| !paths.contains(p));
            if set.is_empty() {
                self.tags.remove(tag);
            }
        }
        adding
    }

    // --- Filtering ---

    /// Whether `path` is one the current filter shows.
    fn shows(&self, path: &Path) -> bool {
        match &self.filter {
            None => true,
            Some(tag) => self.is_tagged(tag, path),
        }
    }

    /// Narrow the visible list to images carrying `tag`, or widen it to all of
    /// them with `None`.
    ///
    /// Returns `false` — changing nothing — if no image carries the tag. An
    /// empty wall would say "there are no photos here", which is a lie about a
    /// folder that is merely unfiltered-full.
    pub fn set_filter(&mut self, tag: Option<String>) -> bool {
        let previous = std::mem::replace(&mut self.filter, tag);
        if self.relist() {
            return true;
        }
        self.filter = previous;
        self.relist();
        false
    }

    /// Re-apply the filter after the tags changed under it, dropping it if it
    /// no longer matches anything.
    ///
    /// Un-favouriting the last favourite while looking at the favourites is not
    /// an error and must not empty the screen: the photos are all still there,
    /// so the honest thing is to show them again.
    pub fn refilter(&mut self) -> bool {
        if self.relist() {
            return true;
        }
        self.filter = None;
        self.relist();
        false
    }

    // --- Grouping ---
    //
    // The controls the wall reaches for once `g` exists (phase 4). Allowed dead
    // until then in one place: an allowed item counts as live, so this also
    // keeps the ladder underneath it — `Threshold::looser` and friends — from
    // reading as unreachable.
    /// Turn grouping on over these fingerprints, or off with `None`, handing
    /// back whatever was there before.
    ///
    /// The old grouping is worth having: it holds the fingerprints, so a wall
    /// that keeps it can be re-grouped without hashing the folder again.
    pub fn set_grouping(&mut self, grouping: Option<Grouping>) -> Option<Grouping> {
        let previous = std::mem::replace(&mut self.grouping, grouping);
        // Cannot empty the wall: grouping only ever folds photos into a tile
        // that is on it already.
        let _ = self.relist();
        previous
    }

    pub fn loosen(&mut self) {
        self.retune(Threshold::looser);
    }

    pub fn tighten(&mut self) {
        self.retune(Threshold::tighter);
    }

    /// Move the threshold and re-chain, unless the ladder has run out or there
    /// is no grouping to re-chain.
    fn retune(&mut self, step: fn(Threshold) -> Threshold) {
        let Some(grouping) = &mut self.grouping else {
            return;
        };
        let next = step(grouping.threshold());
        if next == grouping.threshold() {
            return;
        }
        grouping.set_threshold(next);
        let _ = self.relist();
    }

    /// Rebuild `paths` from `all`, the filter and the grouping, keeping the
    /// cursor where it can and landing it near where it was when it cannot.
    ///
    /// The filter runs first and the grouping over what survives it, so a hidden
    /// photo breaks a run and its neighbours are weighed against each other.
    /// That is the honest reading: grouping describes what is on the wall, and a
    /// photo that is not on the wall is not part of a run.
    ///
    /// `false` if that leaves nothing — `PointedList` cannot be empty, so every
    /// caller has to decide what to do about it.
    fn relist(&mut self) -> bool {
        let current = self.paths.current().clone();
        let was = self.paths.index();
        let previous: Vec<PathBuf> = self.paths.iter().cloned().collect();

        let kept: Vec<PathBuf> = self.all.iter().filter(|p| self.shows(p)).cloned().collect();
        let tiles = match &mut self.grouping {
            Some(grouping) => grouping.rebuild(&kept),
            None => kept.clone(),
        };
        let Some(mut paths) = PointedList::new(tiles) else {
            return false;
        };

        if paths.contains(&current) {
            paths.goto_value(&current);
        } else if let Some(head) = self.grouping.as_ref().and_then(|g| g.head_of(&current)) {
            // Not gone, just folded into a pile: follow the photo into it,
            // rather than treating it as one that vanished.
            paths.goto_value(head);
        } else {
            // Gone from view. Take the nearest of where it used to sit, by
            // *old* position rather than by wrapping the way navigation does:
            // after a run disappears, the eye is where the run was.
            let landing = previous
                .iter()
                .enumerate()
                .filter(|(_, p)| paths.contains(p))
                .min_by_key(|(i, _)| i.abs_diff(was))
                .and_then(|(_, p)| paths.iter().position(|q| q == p))
                .unwrap_or(0);
            paths.goto(landing);
        }
        // The selection holds only what is on screen. Un-favouriting a selected
        // photo while looking at the favourites takes it off the wall, and a
        // selection reaching past the wall is the one thing filtering must
        // never allow: it would put images the user cannot see into the next
        // batch.
        //
        // Measured against what the filter kept rather than against `paths`,
        // because a photo inside a stack is on the wall without being in
        // `paths`. Dropping those would empty the selection of every stack.
        let shown: HashSet<&PathBuf> = kept.iter().collect();
        self.selection.retain(|p| shown.contains(p));
        self.paths = paths;
        true
    }

    // --- Removal ---

    /// Drop `gone` from the library, landing the cursor on the surviving image
    /// nearest to where it was.
    ///
    /// Anything removed is dropped from the selection and from every tag too: a
    /// photo that is not there cannot be a favourite.
    ///
    /// Returns `false` if nothing is left, at which point the caller has no
    /// library to show — `PointedList` cannot be empty.
    pub fn remove(&mut self, gone: &HashSet<PathBuf>) -> bool {
        self.all.retain(|p| !gone.contains(p));
        self.selection.retain(|p| !gone.contains(p));
        for paths in self.tags.values_mut() {
            paths.retain(|p| !gone.contains(p));
        }
        self.tags.retain(|_, paths| !paths.is_empty());
        self.relist()
    }
}

/// What a painted range does to the selection when it is committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeOp {
    Add,
    Remove,
}

/// How much of what one tile stands for is selected.
///
/// Only a stack can be `Some`: a plain thumbnail stands for one photo, which is
/// either selected or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selected {
    None,
    Some,
    All,
}

/// Scan `image_dir` for images.
///
/// Returns `Ok(None)` if the directory has no images (the Python early return),
/// `Err` if the path is missing or not a directory.
pub fn load_library(image_dir: &Path) -> Result<Option<Library>, LibraryError> {
    if !image_dir.exists() {
        return Err(LibraryError::DirNotFound(image_dir.to_path_buf()));
    }
    if !image_dir.is_dir() {
        return Err(LibraryError::NotADirectory(image_dir.to_path_buf()));
    }

    let mut images: Vec<PathBuf> = fs::read_dir(image_dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| is_image(p))
        .collect();
    if images.is_empty() {
        return Ok(None);
    }
    images.sort();

    let tags = tags::load(image_dir, &images);
    // Safe: images is non-empty (checked above).
    let paths = PointedList::new(images.clone()).expect("images is non-empty");

    Ok(Some(Library {
        all: images,
        paths,
        image_dir: image_dir.to_path_buf(),
        selection: HashSet::new(),
        tags,
        // Opening a folder shows what is in it. A filter carried over from a
        // previous session would hide photos before the user had seen any.
        filter: None,
        // Fingerprinting a folder costs a decode of every photo in it, so it
        // waits for the first `g` rather than happening at every open.
        grouping: None,
    }))
}

/// Case-insensitive extension check against `IMAGE_EXTENSIONS`.
fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            IMAGE_EXTENSIONS.contains(&e.as_str())
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp directory with three image files (`0.png`, `1.jpg`, `2.jpeg`) and
    /// a `Library` over them, plus a guard that cleans up on drop.
    struct Fixture {
        dir: PathBuf,
        images: Vec<PathBuf>,
        lib: Library,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn unique_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        std::env::temp_dir().join(format!("pv-test-{tag}-{pid}-{n}"))
    }

    /// Mirror of the pytest `tmp_images` + `image_state` fixtures.
    fn fixture(tag: &str) -> Fixture {
        let dir = unique_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        let names = ["0.png", "1.jpg", "2.jpeg"];
        let mut images: Vec<PathBuf> = names.iter().map(|n| dir.join(n)).collect();
        for p in &images {
            fs::write(p, b"fake-image").unwrap();
        }
        images.sort();

        let lib = Library {
            all: images.clone(),
            paths: PointedList::new(images.clone()).unwrap(),
            image_dir: dir.clone(),
            selection: HashSet::new(),
            tags: Tags::new(),
            filter: None,
            grouping: None,
        };
        Fixture { dir, images, lib }
    }

    /// A library of `bits.len()` images, grouped, whose fingerprints are `bits`
    /// ones each — so two of them are `|n - m|` apart and the runs can be read
    /// off the numbers against a default threshold of 10.
    fn grouped(tag: &str, bits: &[u32]) -> Fixture {
        let dir = unique_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        let images: Vec<PathBuf> = (0..bits.len())
            .map(|i| dir.join(format!("{i}.png")))
            .collect();
        for p in &images {
            fs::write(p, b"fake-image").unwrap();
        }

        let mut lib = Library {
            all: images.clone(),
            paths: PointedList::new(images.clone()).unwrap(),
            image_dir: dir.clone(),
            selection: HashSet::new(),
            tags: Tags::new(),
            filter: None,
            grouping: None,
        };
        lib.set_grouping(Some(Grouping::new(prints(&images, bits))));
        Fixture { dir, images, lib }
    }

    /// Fingerprints of `bits` ones each, by path.
    fn prints(
        images: &[PathBuf],
        bits: &[u32],
    ) -> std::collections::HashMap<PathBuf, crate::core::fingerprint::Fingerprint> {
        images
            .iter()
            .zip(bits)
            .map(|(path, bits)| {
                let print = crate::core::fingerprint::Fingerprint {
                    dhash: (1u64 << bits) - 1,
                    taken: None,
                    landscape: true,
                };
                (path.clone(), print)
            })
            .collect()
    }

    /// The visible images, in order.
    fn shown(lib: &Library) -> Vec<PathBuf> {
        lib.paths.iter().cloned().collect()
    }

    const FAV: &str = crate::core::tags::FAVOURITE;

    /// The selection as sorted paths, for order-independent comparison.
    fn selected(lib: &Library) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = lib.selection.iter().cloned().collect();
        out.sort();
        out
    }

    // --- Navigation ---

    #[test]
    fn current_returns_first_image() {
        let f = fixture("nav-cur");
        assert_eq!(f.lib.current(), &f.images[0]);
    }

    #[test]
    fn next_and_prev() {
        let f = fixture("nav-np");
        let mut lib = f.lib.clone();
        lib.next();
        assert_eq!(lib.current(), &f.images[1]);
        lib.next();
        lib.prev();
        assert_eq!(lib.current(), &f.images[1]);
    }

    #[test]
    fn goto_jumps_to_index() {
        let f = fixture("nav-goto");
        let mut lib = f.lib.clone();
        lib.goto(2);
        assert_eq!(lib.current(), &f.images[2]);
    }

    // --- Selection ---

    #[test]
    fn toggle_selects_then_deselects() {
        let f = fixture("sel-toggle");
        let mut lib = f.lib.clone();
        lib.toggle_selected(1);
        assert_eq!(selected(&lib), vec![f.images[1].clone()]);
        lib.toggle_selected(1);
        assert!(lib.selection.is_empty());
    }

    #[test]
    fn toggle_out_of_range_is_a_noop() {
        let f = fixture("sel-toggle-oob");
        let mut lib = f.lib.clone();
        lib.toggle_selected(99);
        assert!(lib.selection.is_empty());
    }

    #[test]
    fn apply_range_adds_the_whole_run() {
        let f = fixture("sel-range");
        let mut lib = f.lib.clone();
        lib.apply_range(0, 1, RangeOp::Add);
        assert_eq!(
            selected(&lib),
            vec![f.images[0].clone(), f.images[1].clone()]
        );
    }

    #[test]
    fn apply_range_is_the_same_in_both_directions() {
        let f = fixture("sel-range-dir");
        let mut up = f.lib.clone();
        let mut down = f.lib.clone();
        // Painting a range upwards must cover the same images as painting the
        // same range downwards — the anchor can be either end.
        up.apply_range(0, 2, RangeOp::Add);
        down.apply_range(2, 0, RangeOp::Add);
        assert_eq!(selected(&up), selected(&down));
        assert_eq!(selected(&up).len(), 3);
    }

    #[test]
    fn apply_range_of_one_selects_one() {
        let f = fixture("sel-range-single");
        let mut lib = f.lib.clone();
        lib.apply_range(1, 1, RangeOp::Add);
        assert_eq!(selected(&lib), vec![f.images[1].clone()]);
    }

    #[test]
    fn apply_range_add_is_a_union_not_a_replacement() {
        let f = fixture("sel-range-union");
        let mut lib = f.lib.clone();
        lib.apply_range(0, 0, RangeOp::Add);
        lib.apply_range(2, 2, RangeOp::Add);
        // The first run survives the second: scattered runs accumulate.
        assert_eq!(
            selected(&lib),
            vec![f.images[0].clone(), f.images[2].clone()]
        );
    }

    #[test]
    fn apply_range_remove_subtracts_only_that_run() {
        let f = fixture("sel-range-remove");
        let mut lib = f.lib.clone();
        lib.select_all();
        lib.apply_range(0, 1, RangeOp::Remove);
        assert_eq!(selected(&lib), vec![f.images[2].clone()]);
    }

    #[test]
    fn removing_an_unselected_run_is_harmless() {
        let f = fixture("sel-range-remove-noop");
        let mut lib = f.lib.clone();
        lib.apply_range(0, 2, RangeOp::Remove);
        assert!(lib.selection.is_empty());
    }

    #[test]
    fn select_all_takes_the_whole_library() {
        let f = fixture("sel-all");
        let mut lib = f.lib.clone();
        lib.toggle_selected(0);
        lib.select_all();
        assert_eq!(selected(&lib), f.images);
    }

    #[test]
    fn clear_empties_the_selection() {
        let f = fixture("sel-clear");
        let mut lib = f.lib.clone();
        lib.select_all();
        lib.clear_selection();
        assert!(lib.selection.is_empty());
    }

    // --- Tags ---

    #[test]
    fn tagging_a_group_tags_all_of_it() {
        let f = fixture("tag-group");
        let mut lib = f.lib.clone();
        assert!(lib.toggle_tag(FAV, &f.images[..2]));
        assert!(lib.is_tagged(FAV, &f.images[0]));
        assert!(lib.is_tagged(FAV, &f.images[1]));
        assert!(!lib.is_tagged(FAV, &f.images[2]));
    }

    #[test]
    fn tagging_a_group_that_already_agrees_untags_it() {
        let f = fixture("tag-untag");
        let mut lib = f.lib.clone();
        lib.toggle_tag(FAV, &f.images[..2]);
        assert!(!lib.toggle_tag(FAV, &f.images[..2]));
        assert!(lib.tags.is_empty());
    }

    #[test]
    fn tagging_a_mixed_group_brings_it_into_agreement() {
        let f = fixture("tag-mixed");
        let mut lib = f.lib.clone();
        lib.toggle_tag(FAV, &f.images[..1]);
        // One of the three carries it. Toggling per image would invert the
        // group into a differently-mixed one, which nobody could predict.
        assert!(lib.toggle_tag(FAV, &f.images));
        assert!(f.images.iter().all(|p| lib.is_tagged(FAV, p)));
    }

    #[test]
    fn untagging_the_last_image_drops_the_tag_itself() {
        let f = fixture("tag-empty");
        let mut lib = f.lib.clone();
        lib.toggle_tag(FAV, &f.images[..1]);
        lib.toggle_tag(FAV, &f.images[..1]);
        // Left behind, an empty tag would still be offered as something to
        // filter by.
        assert!(!lib.tags.contains_key(FAV));
    }

    // --- Filtering ---

    #[test]
    fn a_filter_narrows_the_visible_list() {
        let f = fixture("filter-narrow");
        let mut lib = f.lib.clone();
        lib.toggle_tag(FAV, &[f.images[0].clone(), f.images[2].clone()]);

        assert!(lib.set_filter(Some(FAV.to_string())));
        assert_eq!(shown(&lib), vec![f.images[0].clone(), f.images[2].clone()]);
        // And back.
        assert!(lib.set_filter(None));
        assert_eq!(shown(&lib), f.images);
    }

    #[test]
    fn a_hidden_image_cannot_be_selected_at_all() {
        let f = fixture("filter-select");
        let mut lib = f.lib.clone();
        lib.toggle_tag(FAV, &f.images[..1]);
        lib.set_filter(Some(FAV.to_string()));

        // The whole point of narrowing the list rather than the drawing: there
        // is no reachable index for an image the filter hides, so select-all
        // cannot pick one up and a batch cannot act on one.
        lib.select_all();
        assert_eq!(selected(&lib), vec![f.images[0].clone()]);
        lib.apply_range(0, 99, RangeOp::Add);
        assert_eq!(selected(&lib), vec![f.images[0].clone()]);
    }

    #[test]
    fn filtering_keeps_the_cursor_on_its_image_when_it_can() {
        let f = fixture("filter-cursor-kept");
        let mut lib = f.lib.clone();
        lib.toggle_tag(FAV, &[f.images[1].clone(), f.images[2].clone()]);
        lib.goto(2);

        lib.set_filter(Some(FAV.to_string()));
        assert_eq!(lib.current(), &f.images[2]);
        assert_eq!(lib.paths.index(), 1);
    }

    #[test]
    fn filtering_out_the_cursor_lands_it_nearby() {
        let f = fixture("filter-cursor-moved");
        let mut lib = f.lib.clone();
        lib.toggle_tag(FAV, &f.images[..1]);
        lib.goto(1);

        lib.set_filter(Some(FAV.to_string()));
        assert_eq!(lib.current(), &f.images[0]);
    }

    #[test]
    fn a_filter_matching_nothing_is_refused() {
        let f = fixture("filter-empty");
        let mut lib = f.lib.clone();
        // An empty wall would say the folder has no photos in it, which is a
        // lie about a folder that is merely unfiltered-full.
        assert!(!lib.set_filter(Some(FAV.to_string())));
        assert_eq!(lib.filter, None);
        assert_eq!(shown(&lib), f.images);
    }

    #[test]
    fn untagging_the_last_visible_image_drops_the_filter() {
        let f = fixture("filter-drained");
        let mut lib = f.lib.clone();
        lib.toggle_tag(FAV, &f.images[..1]);
        lib.set_filter(Some(FAV.to_string()));

        lib.toggle_tag(FAV, &f.images[..1]);
        // Not an error, and not an empty screen: the photos are all still here.
        assert!(!lib.refilter());
        assert_eq!(lib.filter, None);
        assert_eq!(shown(&lib), f.images);
    }

    #[test]
    fn untagging_some_of_a_filtered_list_leaves_the_rest() {
        let f = fixture("filter-shrink");
        let mut lib = f.lib.clone();
        lib.toggle_tag(FAV, &f.images);
        lib.set_filter(Some(FAV.to_string()));

        lib.toggle_tag(FAV, &f.images[..1]);
        assert!(lib.refilter());
        assert_eq!(shown(&lib), vec![f.images[1].clone(), f.images[2].clone()]);
    }

    #[test]
    fn an_image_that_leaves_the_wall_leaves_the_selection() {
        let f = fixture("filter-deselect");
        let mut lib = f.lib.clone();
        lib.toggle_tag(FAV, &f.images);
        lib.set_filter(Some(FAV.to_string()));
        lib.select_all();

        lib.toggle_tag(FAV, &f.images[..1]);
        lib.refilter();
        // Left selected, it would be in the next batch — and the user would
        // have no way of seeing that it was.
        assert_eq!(
            selected(&lib),
            vec![f.images[1].clone(), f.images[2].clone()]
        );
    }

    #[test]
    fn removing_a_filtered_image_prunes_the_hidden_list_too() {
        let f = fixture("filter-remove");
        let mut lib = f.lib.clone();
        lib.toggle_tag(FAV, &f.images[..2]);
        lib.set_filter(Some(FAV.to_string()));

        let gone: HashSet<PathBuf> = [f.images[0].clone()].into_iter().collect();
        assert!(lib.remove(&gone));
        assert_eq!(shown(&lib), vec![f.images[1].clone()]);
        // Dropping it from the visible list alone would bring it back the
        // moment the filter came off.
        assert!(!lib.all.contains(&f.images[0]));
        assert!(!lib.is_tagged(FAV, &f.images[0]));
    }

    // --- Removal ---

    #[test]
    fn remove_drops_the_images_and_deselects_them() {
        let f = fixture("rm-basic");
        let mut lib = f.lib.clone();
        lib.select_all();
        let gone: HashSet<PathBuf> = [f.images[0].clone()].into_iter().collect();

        assert!(lib.remove(&gone));
        assert_eq!(lib.paths.len(), 2);
        assert!(!lib.paths.contains(&f.images[0]));
        assert_eq!(
            selected(&lib),
            vec![f.images[1].clone(), f.images[2].clone()]
        );
    }

    #[test]
    fn remove_lands_the_cursor_on_the_nearest_survivor() {
        let f = fixture("rm-cursor");
        let mut lib = f.lib.clone();
        lib.goto(2);
        let gone: HashSet<PathBuf> = [f.images[2].clone()].into_iter().collect();

        assert!(lib.remove(&gone));
        // The image under the cursor went, so the cursor takes the nearest
        // survivor by old position — not a wrap back round to the front.
        assert_eq!(lib.current(), &f.images[1]);
    }

    #[test]
    fn remove_keeps_the_cursor_on_a_surviving_image() {
        let f = fixture("rm-cursor-kept");
        let mut lib = f.lib.clone();
        lib.goto(2);
        let gone: HashSet<PathBuf> = [f.images[0].clone()].into_iter().collect();

        assert!(lib.remove(&gone));
        // Still the same photo, at its new index.
        assert_eq!(lib.current(), &f.images[2]);
        assert_eq!(lib.paths.index(), 1);
    }

    #[test]
    fn removing_everything_reports_an_empty_library() {
        let f = fixture("rm-all");
        let mut lib = f.lib.clone();
        let gone: HashSet<PathBuf> = f.images.iter().cloned().collect();
        // `PointedList` cannot be empty, so the caller has to hear about this.
        assert!(!lib.remove(&gone));
    }

    #[test]
    fn removing_nothing_changes_nothing() {
        let f = fixture("rm-none");
        let mut lib = f.lib.clone();
        lib.goto(1);
        assert!(lib.remove(&HashSet::new()));
        assert_eq!(lib.paths.len(), 3);
        assert_eq!(lib.paths.index(), 1);
    }

    // --- Grouping ---

    #[test]
    fn a_run_of_similar_photos_becomes_one_tile() {
        let f = grouped("group-fold", &[0, 1, 2, 40]);
        assert_eq!(
            shown(&f.lib),
            vec![f.images[0].clone(), f.images[3].clone()]
        );
        assert_eq!(f.lib.stack_size(&f.images[0]), 3);
        assert_eq!(f.lib.members(&f.images[0]), f.images[..3].to_vec());
    }

    #[test]
    fn a_tile_that_is_not_a_stack_stands_for_itself() {
        let f = grouped("group-lone", &[0, 1, 2, 40]);
        assert_eq!(f.lib.stack_size(&f.images[3]), 1);
        assert_eq!(f.lib.members(&f.images[3]), vec![f.images[3].clone()]);
    }

    #[test]
    fn the_photos_are_all_still_there() {
        let f = grouped("group-photos", &[0, 1, 2, 40]);
        // Two tiles, four photos. Everything that acts on files reads the
        // second number, and nothing but the drawing reads the first.
        assert_eq!(f.lib.paths.len(), 2);
        assert_eq!(f.lib.photos().cloned().collect::<Vec<_>>(), f.images);
    }

    #[test]
    fn turning_grouping_off_puts_every_photo_back() {
        let f = grouped("group-off", &[0, 1, 2, 40]);
        let mut lib = f.lib.clone();
        lib.set_grouping(None);
        assert_eq!(shown(&lib), f.images);
        assert_eq!(lib.stack_size(&f.images[0]), 1);
    }

    #[test]
    fn selecting_a_stack_selects_every_photo_in_it() {
        let f = grouped("group-select", &[0, 1, 2, 40]);
        let mut lib = f.lib.clone();
        lib.toggle_selected(0);
        // The whole point of expanding here: `d` on this tile now trashes three
        // files, because three real paths are what the selection holds.
        assert_eq!(selected(&lib), f.images[..3].to_vec());
        assert_eq!(lib.selected(&f.images[0]), Selected::All);
    }

    #[test]
    fn toggling_a_full_stack_empties_it() {
        let f = grouped("group-deselect", &[0, 1, 2, 40]);
        let mut lib = f.lib.clone();
        lib.toggle_selected(0);
        lib.toggle_selected(0);
        assert!(lib.selection.is_empty());
    }

    #[test]
    fn a_half_selected_stack_fills_up_rather_than_emptying() {
        let f = grouped("group-half", &[0, 1, 2, 40]);
        let mut lib = f.lib.clone();
        lib.selection.insert(f.images[1].clone());
        assert_eq!(lib.selected(&f.images[0]), Selected::Some);

        // Emptying it would be the other reading, and it leaves the user unable
        // to say "all of this" with one press.
        lib.toggle_selected(0);
        assert_eq!(selected(&lib), f.images[..3].to_vec());
    }

    #[test]
    fn a_painted_range_takes_whole_stacks() {
        let f = grouped("group-range", &[0, 1, 2, 40]);
        let mut lib = f.lib.clone();
        lib.apply_range(0, 1, RangeOp::Add);
        assert_eq!(selected(&lib), f.images);

        lib.apply_range(0, 0, RangeOp::Remove);
        assert_eq!(selected(&lib), vec![f.images[3].clone()]);
    }

    #[test]
    fn select_all_takes_the_photos_not_the_tiles() {
        let f = grouped("group-all", &[0, 1, 2, 40]);
        let mut lib = f.lib.clone();
        lib.select_all();
        assert_eq!(selected(&lib), f.images);
    }

    #[test]
    fn the_cursor_follows_its_photo_into_the_stack() {
        let f = grouped("group-cursor", &[0, 1, 2, 40]);
        let mut lib = f.lib.clone();
        lib.set_grouping(None);
        lib.goto(2);

        lib.set_grouping(Some(Grouping::new(prints(&f.images, &[0, 1, 2, 40]))));
        // The photo it was on is no longer a tile, but it has not gone
        // anywhere: it is in the pile the cursor now sits on.
        assert_eq!(lib.current(), &f.images[0]);
    }

    #[test]
    fn a_photo_the_filter_hides_breaks_the_run() {
        let f = grouped("group-filter", &[0, 8, 16]);
        let mut lib = f.lib.clone();
        assert_eq!(lib.stack_size(&f.images[0]), 3);

        // The middle photo is what held the two ends together; hidden, they are
        // weighed against each other and found too far apart.
        lib.toggle_tag(FAV, &[f.images[0].clone(), f.images[2].clone()]);
        assert!(lib.set_filter(Some(FAV.to_string())));
        assert_eq!(shown(&lib), vec![f.images[0].clone(), f.images[2].clone()]);
        assert_eq!(lib.stack_size(&f.images[0]), 1);
    }

    #[test]
    fn a_selected_member_the_filter_hides_leaves_the_selection() {
        let f = grouped("group-filter-select", &[0, 1, 2, 40]);
        let mut lib = f.lib.clone();
        lib.toggle_selected(0);
        lib.toggle_tag(
            FAV,
            &[
                f.images[0].clone(),
                f.images[1].clone(),
                f.images[3].clone(),
            ],
        );

        assert!(lib.set_filter(Some(FAV.to_string())));
        // The hidden member goes, the visible ones stay — the retain has to
        // keep stack members, which are on the wall without being in `paths`.
        assert_eq!(selected(&lib), f.images[..2].to_vec());
        assert_eq!(lib.selected(&f.images[0]), Selected::All);
    }

    #[test]
    fn trashing_the_photo_between_two_others_lets_them_stack() {
        let f = grouped("group-regroup", &[0, 40, 1]);
        let mut lib = f.lib.clone();
        assert_eq!(shown(&lib), f.images);

        let gone: HashSet<PathBuf> = [f.images[1].clone()].into_iter().collect();
        assert!(lib.remove(&gone));
        // Frozen groups would leave these two apart until the next `g`, and the
        // wall would be describing a folder that no longer exists.
        assert_eq!(shown(&lib), vec![f.images[0].clone()]);
        assert_eq!(lib.stack_size(&f.images[0]), 2);
    }

    #[test]
    fn a_stack_that_loses_all_but_one_member_dissolves() {
        let f = grouped("group-dissolve", &[0, 1, 40]);
        let mut lib = f.lib.clone();
        assert_eq!(lib.stack_size(&f.images[0]), 2);

        let gone: HashSet<PathBuf> = [f.images[1].clone()].into_iter().collect();
        assert!(lib.remove(&gone));
        assert_eq!(lib.stack_size(&f.images[0]), 1);
        assert_eq!(shown(&lib), vec![f.images[0].clone(), f.images[2].clone()]);
    }

    #[test]
    fn loosening_the_dial_merges_stacks() {
        let f = grouped("group-loosen", &[0, 6, 12, 18]);
        let mut lib = f.lib.clone();
        assert_eq!(lib.paths.len(), 2);

        lib.loosen();
        lib.loosen();
        assert_eq!(shown(&lib), vec![f.images[0].clone()]);
        assert_eq!(lib.stack_size(&f.images[0]), 4);
    }

    #[test]
    fn tightening_the_dial_splits_them() {
        let f = grouped("group-tighten", &[0, 6, 12, 18]);
        let mut lib = f.lib.clone();
        lib.tighten();
        assert_eq!(shown(&lib), vec![f.images[0].clone(), f.images[2].clone()]);
        assert_eq!(lib.stack_size(&f.images[0]), 2);
        assert_eq!(lib.stack_size(&f.images[2]), 2);
    }

    #[test]
    fn the_dial_stops_at_the_end_of_the_ladder() {
        let f = grouped("group-ladder", &[0, 6, 12, 18]);
        let mut lib = f.lib.clone();
        for _ in 0..10 {
            lib.loosen();
        }
        assert_eq!(lib.paths.len(), 1);
        for _ in 0..10 {
            lib.tighten();
        }
        // Tightest rung: six bits apart is too far, so nothing stacks at all.
        assert_eq!(shown(&lib), f.images);
    }

    #[test]
    fn the_dial_does_nothing_when_grouping_is_off() {
        let f = fixture("group-dial-off");
        let mut lib = f.lib.clone();
        lib.loosen();
        lib.tighten();
        assert_eq!(shown(&lib), f.images);
        assert!(lib.grouping.is_none());
    }

    // --- load_library ---

    #[test]
    fn load_starts_with_nothing_selected() {
        let dir = unique_dir("load-sel");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.png"), "image").unwrap();
        let lib = load_library(&dir).unwrap().unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert!(lib.selection.is_empty());
    }

    #[test]
    fn load_errors_if_dir_missing() {
        let dir = unique_dir("load-missing");
        assert!(matches!(
            load_library(&dir),
            Err(LibraryError::DirNotFound(_))
        ));
    }

    #[test]
    fn load_errors_if_not_a_directory() {
        let dir = unique_dir("load-notdir");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("file.txt");
        fs::write(&file, b"content").unwrap();
        let result = load_library(&file);
        let _ = fs::remove_dir_all(&dir);
        assert!(matches!(result, Err(LibraryError::NotADirectory(_))));
    }

    #[test]
    fn load_returns_none_if_no_images() {
        let dir = unique_dir("load-noimg");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("file.txt"), b"not an image").unwrap();
        let result = load_library(&dir).unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert!(result.is_none());
    }

    #[test]
    fn load_finds_png_jpg_jpeg_only() {
        let dir = unique_dir("load-exts");
        fs::create_dir_all(&dir).unwrap();
        for (name, body) in [
            ("a.png", "png"),
            ("b.jpg", "jpg"),
            ("c.jpeg", "jpeg"),
            ("d.gif", "gif"),
        ] {
            fs::write(dir.join(name), body).unwrap();
        }
        let lib = load_library(&dir).unwrap().unwrap();
        let names: Vec<String> = lib
            .paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(lib.paths.len(), 3);
        assert!(!names.contains(&"d.gif".to_string()));
    }

    #[test]
    fn load_case_insensitive_extensions() {
        let dir = unique_dir("load-case");
        fs::create_dir_all(&dir).unwrap();
        for name in ["a.PNG", "b.JPG", "c.JPEG"] {
            fs::write(dir.join(name), "x").unwrap();
        }
        let lib = load_library(&dir).unwrap().unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(lib.paths.len(), 3);
    }

    #[test]
    fn load_sorts_alphabetically() {
        let dir = unique_dir("load-sort");
        fs::create_dir_all(&dir).unwrap();
        for name in ["c.png", "a.png", "b.png"] {
            fs::write(dir.join(name), "x").unwrap();
        }
        let lib = load_library(&dir).unwrap().unwrap();
        let names: Vec<String> = lib
            .paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(names, vec!["a.png", "b.png", "c.png"]);
    }

    /// The old cache directory must not be picked up as an image source, and
    /// must not stop a directory loading.
    #[test]
    fn load_ignores_a_leftover_cache_dir() {
        let dir = unique_dir("load-legacy-cache");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.png"), "image").unwrap();
        let cache = dir.join(".photo-viewer");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("favourites"), b"/nonexistent/image.png").unwrap();
        let lib = load_library(&dir).unwrap().unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(lib.paths.len(), 1);
    }

    #[test]
    fn load_picks_up_favourites_from_the_old_cache() {
        let dir = unique_dir("load-legacy-favs");
        fs::create_dir_all(&dir).unwrap();
        for name in ["a.png", "b.png"] {
            fs::write(dir.join(name), "image").unwrap();
        }
        let cache = dir.join(".photo-viewer");
        fs::create_dir_all(&cache).unwrap();
        fs::write(
            cache.join("favourites"),
            dir.join("b.png").to_str().unwrap(),
        )
        .unwrap();

        let lib = load_library(&dir).unwrap().unwrap();
        let _ = fs::remove_dir_all(&dir);
        // Phase 0 left that file in place on purpose; this is what it was for.
        assert!(lib.is_tagged(FAV, &lib.all[1]));
        assert!(!lib.is_tagged(FAV, &lib.all[0]));
        // But the folder still opens showing everything.
        assert_eq!(lib.filter, None);
        assert_eq!(lib.paths.len(), 2);
    }
}
