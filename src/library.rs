//! The GUI-free domain core: image collection + favourites/to-delete sets +
//! their on-disk cache.
//!
//! Port of `lib/state.py` (`ImageState` + `load_image_state`). The cache format
//! is kept byte-compatible with the Python app: `<image_dir>/.photo-viewer/`
//! holds `favourites` and `to_delete`, each a newline-separated list of
//! absolute path strings with no trailing newline.

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
    /// A favourite/delete op named a path not in the current image list.
    NotInLibrary(PathBuf),
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
            LibraryError::NotInLibrary(p) => {
                write!(f, "{} is not in the image list", p.display())
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
    pub favourites: HashSet<PathBuf>,
    pub to_delete: HashSet<PathBuf>,

    pub image_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub favourites_file: PathBuf,
    pub to_delete_file: PathBuf,
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

    // --- Favourites ---

    /// Mark `image_path` as a favourite and persist. Errors if it isn't a known
    /// image (the Python `assert image_path in self.image_paths`).
    pub fn favourite(&mut self, image_path: &Path) -> Result<(), LibraryError> {
        if !self.paths.contains(&image_path.to_path_buf()) {
            return Err(LibraryError::NotInLibrary(image_path.to_path_buf()));
        }
        self.favourites.insert(image_path.to_path_buf());
        self.save()
    }

    /// Un-favourite (no-op if not favourited). Persists only when it changed.
    pub fn unfavourite(&mut self, image_path: &Path) -> Result<(), LibraryError> {
        if self.favourites.remove(image_path) {
            self.save()?;
        }
        Ok(())
    }

    // --- Marking for deletion ---

    /// Mark `image_path` for deletion and persist. Errors if unknown.
    pub fn delete(&mut self, image_path: &Path) -> Result<(), LibraryError> {
        if !self.paths.contains(&image_path.to_path_buf()) {
            return Err(LibraryError::NotInLibrary(image_path.to_path_buf()));
        }
        self.to_delete.insert(image_path.to_path_buf());
        self.save()
    }

    /// Un-mark (no-op if not marked). Persists only when it changed.
    pub fn undelete(&mut self, image_path: &Path) -> Result<(), LibraryError> {
        if self.to_delete.remove(image_path) {
            self.save()?;
        }
        Ok(())
    }

    /// Unlink every marked file from disk, drop it from the list, and clear the
    /// set. Walks the ring like the Python version: delete-at-cursor or advance.
    pub fn delete_all(&mut self) -> Result<(), LibraryError> {
        while !self.to_delete.is_empty() {
            let cur = self.paths.current().clone();
            if self.to_delete.contains(&cur) {
                // missing_ok=True: a already-gone file is not an error.
                match fs::remove_file(&cur) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
                self.paths.delete();
                self.to_delete.remove(&cur);
            } else {
                self.paths.next();
            }
        }
        self.save()
    }

    // --- Persistence ---

    /// Write both cache files (creating `.photo-viewer/` if needed). Entries are
    /// sorted for deterministic output; the format stays newline-joined absolute
    /// paths with no trailing newline, matching the Python writer.
    pub fn save(&self) -> Result<(), LibraryError> {
        fs::create_dir_all(&self.cache_dir)?;
        fs::write(&self.favourites_file, join_paths(&self.favourites))?;
        fs::write(&self.to_delete_file, join_paths(&self.to_delete))?;
        Ok(())
    }

    /// Copy every favourite into `new_dir`, skipping any that already exist
    /// there (matches `shutil.copy2` + the skip message).
    pub fn save_favourites(&self, new_dir: &Path) -> Result<(), LibraryError> {
        for favourite in &self.favourites {
            let Some(name) = favourite.file_name() else {
                continue;
            };
            let destination = new_dir.join(name);
            if destination.exists() {
                eprintln!(
                    "Destination {} already exists, skipping.",
                    destination.display()
                );
            } else {
                fs::copy(favourite, &destination)?;
            }
        }
        Ok(())
    }
}

/// Sort a set of paths and newline-join their string forms (no trailing NL).
fn join_paths(paths: &HashSet<PathBuf>) -> String {
    let mut lines: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    lines.sort();
    lines.join("\n")
}

/// Read a cache file, keeping only entries that are in `images`. Missing file is
/// fine (returns empty). Stale entries are logged and dropped, as in Python.
fn load_cache_set(file: &Path, images: &[PathBuf], label: &str) -> HashSet<PathBuf> {
    let mut set = HashSet::new();
    let contents = match fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("{label} file not found in cache. Init with empty set");
            return set;
        }
    };
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let path = PathBuf::from(trimmed);
        if images.contains(&path) {
            set.insert(path);
        } else {
            eprintln!("{label} {} not in image paths", path.display());
        }
    }
    set
}

/// Scan `image_dir` and load its cached favourites/to-delete.
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

    let cache_dir = image_dir.join(".photo-viewer");
    let favourites_file = cache_dir.join("favourites");
    let to_delete_file = cache_dir.join("to_delete");

    let mut images: Vec<PathBuf> = fs::read_dir(image_dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| is_image(p))
        .collect();
    if images.is_empty() {
        return Ok(None);
    }
    images.sort();

    let favourites = load_cache_set(&favourites_file, &images, "Favourites");
    let to_delete = load_cache_set(&to_delete_file, &images, "To delete");

    // Safe: images is non-empty (checked above).
    let paths = PointedList::new(images).expect("images is non-empty");

    Ok(Some(Library {
        paths,
        favourites,
        to_delete,
        image_dir: image_dir.to_path_buf(),
        cache_dir,
        favourites_file,
        to_delete_file,
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

        let cache_dir = dir.join(".photo-viewer");
        let lib = Library {
            paths: PointedList::new(images.clone()).unwrap(),
            favourites: HashSet::new(),
            to_delete: HashSet::new(),
            image_dir: dir.clone(),
            cache_dir: cache_dir.clone(),
            favourites_file: cache_dir.join("favourites"),
            to_delete_file: cache_dir.join("to_delete"),
        };
        Fixture { dir, images, lib }
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

    // --- Favourites ---

    #[test]
    fn favourite_adds_and_persists() {
        let f = fixture("fav-add");
        let mut lib = f.lib.clone();
        lib.favourite(&f.images[0]).unwrap();
        assert!(lib.favourites.contains(&f.images[0]));
        let content = fs::read_to_string(&lib.favourites_file).unwrap();
        assert!(content.contains(&f.images[0].to_string_lossy().into_owned()));
    }

    #[test]
    fn favourite_rejects_unknown_path() {
        let f = fixture("fav-bad");
        let mut lib = f.lib.clone();
        let bad = f.dir.join("nonexistent.png");
        assert!(matches!(
            lib.favourite(&bad),
            Err(LibraryError::NotInLibrary(_))
        ));
    }

    #[test]
    fn unfavourite_removes() {
        let f = fixture("fav-rm");
        let mut lib = f.lib.clone();
        lib.favourite(&f.images[0]).unwrap();
        lib.unfavourite(&f.images[0]).unwrap();
        assert!(!lib.favourites.contains(&f.images[0]));
    }

    #[test]
    fn unfavourite_noop_if_absent() {
        let f = fixture("fav-noop");
        let mut lib = f.lib.clone();
        lib.unfavourite(&f.images[0]).unwrap();
        assert!(!lib.favourites.contains(&f.images[0]));
    }

    // --- Delete marking ---

    #[test]
    fn delete_adds_and_persists() {
        let f = fixture("del-add");
        let mut lib = f.lib.clone();
        lib.delete(&f.images[0]).unwrap();
        assert!(lib.to_delete.contains(&f.images[0]));
        let content = fs::read_to_string(&lib.to_delete_file).unwrap();
        assert!(content.contains(&f.images[0].to_string_lossy().into_owned()));
    }

    #[test]
    fn delete_rejects_unknown_path() {
        let f = fixture("del-bad");
        let mut lib = f.lib.clone();
        let bad = f.dir.join("nonexistent.png");
        assert!(matches!(
            lib.delete(&bad),
            Err(LibraryError::NotInLibrary(_))
        ));
    }

    #[test]
    fn undelete_removes() {
        let f = fixture("del-rm");
        let mut lib = f.lib.clone();
        lib.delete(&f.images[0]).unwrap();
        lib.undelete(&f.images[0]).unwrap();
        assert!(!lib.to_delete.contains(&f.images[0]));
    }

    // --- delete_all ---

    #[test]
    fn delete_all_removes_files_from_disk() {
        let f = fixture("dall-disk");
        let mut lib = f.lib.clone();
        lib.delete(&f.images[0]).unwrap();
        lib.delete(&f.images[1]).unwrap();
        lib.delete_all().unwrap();
        assert!(!f.images[0].exists());
        assert!(!f.images[1].exists());
        assert!(f.images[2].exists());
    }

    #[test]
    fn delete_all_removes_from_paths_and_clears_set() {
        let f = fixture("dall-paths");
        let mut lib = f.lib.clone();
        lib.delete(&f.images[0]).unwrap();
        lib.delete_all().unwrap();
        assert!(!lib.paths.contains(&f.images[0]));
        assert_eq!(lib.to_delete.len(), 0);
    }

    #[test]
    fn delete_all_noop_when_empty() {
        let f = fixture("dall-empty");
        let mut lib = f.lib.clone();
        lib.delete_all().unwrap();
        assert!(f.images.iter().all(|p| p.exists()));
        assert_eq!(lib.paths.len(), 3);
    }

    #[test]
    fn delete_all_skips_unmarked_images() {
        let f = fixture("dall-skip");
        let mut lib = f.lib.clone();
        lib.delete(&f.images[1]).unwrap();
        lib.delete_all().unwrap();
        assert!(f.images[0].exists());
        assert!(!f.images[1].exists());
        assert!(f.images[2].exists());
        assert_eq!(lib.to_delete.len(), 0);
    }

    // --- save ---

    #[test]
    fn save_creates_cache_dir() {
        let f = fixture("save-mkdir");
        assert!(!f.lib.cache_dir.exists());
        f.lib.save().unwrap();
        assert!(f.lib.cache_dir.exists());
    }

    #[test]
    fn save_handles_empty_sets() {
        let f = fixture("save-empty");
        f.lib.save().unwrap();
        assert_eq!(fs::read_to_string(&f.lib.favourites_file).unwrap(), "");
        assert_eq!(fs::read_to_string(&f.lib.to_delete_file).unwrap(), "");
    }

    // --- save_favourites ---

    #[test]
    fn save_favourites_copies() {
        let f = fixture("sf-copy");
        let mut lib = f.lib.clone();
        let dest = f.dir.join("exported");
        fs::create_dir_all(&dest).unwrap();
        lib.favourites.insert(f.images[0].clone());
        lib.favourites.insert(f.images[1].clone());
        lib.save_favourites(&dest).unwrap();
        assert!(dest.join(f.images[0].file_name().unwrap()).exists());
        assert!(dest.join(f.images[1].file_name().unwrap()).exists());
    }

    #[test]
    fn save_favourites_skips_existing() {
        let f = fixture("sf-skip");
        let mut lib = f.lib.clone();
        let dest = f.dir.join("exported");
        fs::create_dir_all(&dest).unwrap();
        let existing = dest.join(f.images[0].file_name().unwrap());
        fs::write(&existing, b"existing content").unwrap();
        lib.favourites.insert(f.images[0].clone());
        lib.save_favourites(&dest).unwrap();
        assert_eq!(fs::read_to_string(&existing).unwrap(), "existing content");
    }

    // --- load_library ---

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

    #[test]
    fn load_reads_cached_favourites_and_to_delete() {
        let dir = unique_dir("load-cache");
        fs::create_dir_all(&dir).unwrap();
        let img = dir.join("a.png");
        fs::write(&img, "image").unwrap();
        let cache = dir.join(".photo-viewer");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("favourites"), img.to_string_lossy().as_bytes()).unwrap();
        fs::write(cache.join("to_delete"), img.to_string_lossy().as_bytes()).unwrap();
        let lib = load_library(&dir).unwrap().unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert!(lib.favourites.contains(&img));
        assert!(lib.to_delete.contains(&img));
    }

    #[test]
    fn load_ignores_stale_cached_paths() {
        let dir = unique_dir("load-stale");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.png"), "image").unwrap();
        let cache = dir.join(".photo-viewer");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("favourites"), b"/nonexistent/image.png").unwrap();
        fs::write(cache.join("to_delete"), b"/nonexistent/image.png").unwrap();
        let lib = load_library(&dir).unwrap().unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(lib.favourites.len(), 0);
        assert_eq!(lib.to_delete.len(), 0);
    }

    #[test]
    fn load_initializes_empty_without_cache() {
        let dir = unique_dir("load-nocache");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.png"), "image").unwrap();
        let lib = load_library(&dir).unwrap().unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(lib.favourites.len(), 0);
        assert_eq!(lib.to_delete.len(), 0);
    }

    /// Round-trip: what we save must load back identically (format compat).
    #[test]
    fn save_then_load_round_trips() {
        let f = fixture("roundtrip");
        let mut lib = f.lib.clone();
        lib.favourite(&f.images[0]).unwrap();
        lib.delete(&f.images[1]).unwrap();

        let reloaded = load_library(&f.dir).unwrap().unwrap();
        assert!(reloaded.favourites.contains(&f.images[0]));
        assert!(reloaded.to_delete.contains(&f.images[1]));
        assert_eq!(reloaded.favourites.len(), 1);
        assert_eq!(reloaded.to_delete.len(), 1);
    }
}
