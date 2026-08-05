//! The GUI-free domain core: the image collection for one open directory, plus
//! the user's selection within it.
//!
//! Favourites and mark-to-delete used to live here, along with their
//! `<image_dir>/.photo-viewer/` cache. Both were removed (see
//! `SELECT_MODE_PLAN.md` phase 0) — they come back in phase 6 on top of the
//! selection machinery. Any `.photo-viewer/` directories left on disk are inert
//! and deliberately not cleaned up: phase 6 will want to read them.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::pointed_list::PointedList;

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
    pub paths: PointedList<PathBuf>,
    /// The directory `paths` was scanned from. Unread since the cache files
    /// went away; the move/copy destination picker (phase 5) wants it back.
    #[allow(dead_code)]
    pub image_dir: PathBuf,
    /// The images the user has selected, as paths rather than indices:
    /// deleting or moving files renumbers `paths`, and an index-keyed selection
    /// would silently come to mean different images.
    ///
    /// Deliberately not persisted — it is a scratch buffer, not a judgement
    /// about the photos. It lives here rather than in the wall so that it
    /// survives a switch to the single view and back.
    pub selection: HashSet<PathBuf>,
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

    // --- Selection ---

    pub fn is_selected(&self, path: &Path) -> bool {
        self.selection.contains(path)
    }

    /// Add or remove the image at `index`, whichever it isn't already.
    pub fn toggle_selected(&mut self, index: usize) {
        let Some(path) = self.paths.iter().nth(index).cloned() else {
            return;
        };
        if !self.selection.remove(&path) {
            self.selection.insert(path);
        }
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
            match op {
                RangeOp::Add => {
                    self.selection.insert(path);
                }
                RangeOp::Remove => {
                    self.selection.remove(&path);
                }
            }
        }
    }

    pub fn select_all(&mut self) {
        self.selection = self.paths.iter().cloned().collect();
    }

    pub fn invert_selection(&mut self) {
        self.selection = self
            .paths
            .iter()
            .filter(|p| !self.selection.contains(*p))
            .cloned()
            .collect();
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    // --- Removal ---

    /// Drop `gone` from the library, landing the cursor on the surviving image
    /// nearest to where it was.
    ///
    /// Nearest by *old* position, not by wrapping the way the cursor does when
    /// it navigates: after deleting a run, the eye is where the run was, so
    /// that is where the cursor belongs. Anything removed is dropped from the
    /// selection too.
    ///
    /// Returns `false` if nothing is left, at which point the caller has no
    /// library to show — `PointedList` cannot be empty.
    pub fn remove(&mut self, gone: &HashSet<PathBuf>) -> bool {
        let was = self.paths.index();
        let survivors: Vec<(usize, PathBuf)> = self
            .paths
            .iter()
            .enumerate()
            .filter(|(_, p)| !gone.contains(*p))
            .map(|(i, p)| (i, p.clone()))
            .collect();

        let landing = survivors
            .iter()
            .enumerate()
            .min_by_key(|(_, (old, _))| old.abs_diff(was))
            .map(|(new, _)| new)
            .unwrap_or(0);

        let kept: Vec<PathBuf> = survivors.into_iter().map(|(_, p)| p).collect();
        self.selection.retain(|p| !gone.contains(p));

        match PointedList::new(kept) {
            Some(mut paths) => {
                paths.goto(landing);
                self.paths = paths;
                true
            }
            None => false,
        }
    }
}

/// What a painted range does to the selection when it is committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeOp {
    Add,
    Remove,
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

    // Safe: images is non-empty (checked above).
    let paths = PointedList::new(images).expect("images is non-empty");

    Ok(Some(Library {
        paths,
        image_dir: image_dir.to_path_buf(),
        selection: HashSet::new(),
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
            paths: PointedList::new(images.clone()).unwrap(),
            image_dir: dir.clone(),
            selection: HashSet::new(),
        };
        Fixture { dir, images, lib }
    }

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
        assert_eq!(selected(&lib), vec![f.images[0].clone(), f.images[1].clone()]);
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
        assert_eq!(selected(&lib), vec![f.images[0].clone(), f.images[2].clone()]);
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
    fn invert_swaps_selected_and_unselected() {
        let f = fixture("sel-invert");
        let mut lib = f.lib.clone();
        lib.toggle_selected(1);
        lib.invert_selection();
        assert_eq!(selected(&lib), vec![f.images[0].clone(), f.images[2].clone()]);
    }

    #[test]
    fn inverting_nothing_selects_everything() {
        let f = fixture("sel-invert-empty");
        let mut lib = f.lib.clone();
        lib.invert_selection();
        assert_eq!(selected(&lib), f.images);
        // And back again.
        lib.invert_selection();
        assert!(lib.selection.is_empty());
    }

    #[test]
    fn clear_empties_the_selection() {
        let f = fixture("sel-clear");
        let mut lib = f.lib.clone();
        lib.select_all();
        lib.clear_selection();
        assert!(lib.selection.is_empty());
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
        assert_eq!(selected(&lib), vec![f.images[1].clone(), f.images[2].clone()]);
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
}
