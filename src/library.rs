//! The GUI-free domain core: the image collection for one open directory.
//!
//! Favourites and mark-to-delete used to live here, along with their
//! `<image_dir>/.photo-viewer/` cache. Both were removed (see
//! `SELECT_MODE_PLAN.md` phase 0) — they come back in phase 6 on top of the
//! selection machinery. Any `.photo-viewer/` directories left on disk are inert
//! and deliberately not cleaned up: phase 6 will want to read them.

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
