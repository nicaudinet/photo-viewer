//! Moving and copying image files into another folder.
//!
//! The GUI decides *which* files and *what to do about name clashes*; this
//! module does one file at a time and reports what happened to the source, so
//! the wall knows whether the image is still in the library.
//!
//! Clashes are resolved by a policy chosen once, before any file is written
//! (see `SELECT_MODE_PLAN.md` phase 5), rather than by a prompt per file: a
//! batch runs several files at a time, so a mid-batch question would have to
//! stall dispatch while work already handed to the runtime carried on — and a
//! hundred clashes would mean a hundred questions.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// Whether the source file survives the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransferKind {
    Move,
    Copy,
}

impl TransferKind {
    /// Present participle, for the progress bar.
    pub(crate) fn verb(self) -> &'static str {
        match self {
            TransferKind::Move => "Moving",
            TransferKind::Copy => "Copying",
        }
    }

    /// Imperative, for the confirmation question.
    pub(crate) fn word(self) -> &'static str {
        match self {
            TransferKind::Move => "Move",
            TransferKind::Copy => "Copy",
        }
    }
}

/// What to do when the destination already holds a file of the same name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Collision {
    /// Leave both the source and the file already there untouched.
    Skip,
    /// Write alongside it as `name-1.jpg`.
    KeepBoth,
    /// Replace it.
    Overwrite,
}

/// What became of the source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transferred {
    /// It left the library's folder — a move that went through.
    SourceGone,
    /// It is still there: a copy, or a move that was skipped.
    SourceKept,
}

/// How many of `paths` already have a namesake in `dest`.
///
/// Counted up front so the user is asked once, with the real number, before
/// anything is written. A file could still appear between this and the write,
/// which is why the policy — not this count — is what the write applies.
pub(crate) fn collisions(paths: &[PathBuf], dest: &Path) -> usize {
    paths
        .iter()
        .filter(|src| match src.file_name() {
            Some(name) => {
                let target = dest.join(name);
                target != **src && target.exists()
            }
            None => false,
        })
        .count()
}

/// Move or copy one file into `dest_dir`, applying `collision` if something of
/// that name is already there.
pub(crate) fn transfer(
    src: &Path,
    dest_dir: &Path,
    kind: TransferKind,
    collision: Collision,
) -> Result<Transferred, String> {
    let name = src
        .file_name()
        .ok_or_else(|| format!("{} has no file name", src.display()))?;
    let plain = dest_dir.join(name);

    // The file is already exactly where it is being sent. Both `rename` and
    // `copy` onto oneself are at best pointless and at worst destructive.
    if plain == src {
        return Ok(Transferred::SourceKept);
    }

    let target = if plain.exists() {
        match collision {
            Collision::Skip => return Ok(Transferred::SourceKept),
            Collision::Overwrite => plain,
            Collision::KeepBoth => free_name(dest_dir, name).ok_or_else(|| {
                format!(
                    "No free name for {} in {}",
                    name.to_string_lossy(),
                    dest_dir.display()
                )
            })?,
        }
    } else {
        plain
    };

    match kind {
        TransferKind::Copy => {
            fs::copy(src, &target).map_err(|e| failure("copy", src, &target, &e))?;
            Ok(Transferred::SourceKept)
        }
        TransferKind::Move => {
            move_file(src, &target)?;
            Ok(Transferred::SourceGone)
        }
    }
}

/// Rename, falling back to copy-then-delete across filesystem boundaries.
///
/// `rename` is the right primitive — instant, atomic, and it cannot half-write
/// a photo — but it only works within one filesystem. Sending images to an
/// external disk hits `EXDEV`, and only that error is worth the slow path: any
/// other failure means the copy would fail too, so reporting it is better than
/// retrying it.
fn move_file(src: &Path, target: &Path) -> Result<(), String> {
    match fs::rename(src, target) {
        Ok(()) => Ok(()),
        Err(e) if cross_device(&e) => {
            fs::copy(src, target).map_err(|e| failure("copy", src, target, &e))?;
            // Only now: a failed copy must never cost the original.
            fs::remove_file(src)
                .map_err(|e| format!("Copied {} but could not remove it: {e}", src.display()))
        }
        Err(e) => Err(failure("move", src, target, &e)),
    }
}

/// `EXDEV` on unix, `ERROR_NOT_SAME_DEVICE` on Windows: the two ends are on
/// different filesystems, so the directory entry cannot simply be relinked.
fn cross_device(e: &std::io::Error) -> bool {
    #[cfg(unix)]
    const CODE: i32 = 18;
    #[cfg(windows)]
    const CODE: i32 = 17;
    #[cfg(not(any(unix, windows)))]
    const CODE: i32 = i32::MIN;
    e.raw_os_error() == Some(CODE)
}

fn failure(verb: &str, src: &Path, target: &Path, e: &std::io::Error) -> String {
    format!(
        "Could not {verb} {} to {}: {e}",
        src.display(),
        target.display()
    )
}

/// The first free `stem-N.ext` in `dir`, counting up from 1.
///
/// `None` if the whole run is taken, which the caller reports rather than
/// looping forever.
fn free_name(dir: &Path, name: &OsStr) -> Option<PathBuf> {
    let as_path = Path::new(name);
    let stem = as_path.file_stem()?.to_string_lossy().into_owned();
    let ext = as_path
        .extension()
        .map(|e| e.to_string_lossy().into_owned());
    (1..10_000u32).find_map(|n| {
        let candidate = match &ext {
            Some(ext) => format!("{stem}-{n}.{ext}"),
            None => format!("{stem}-{n}"),
        };
        let path = dir.join(candidate);
        (!path.exists()).then_some(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source and a destination directory, cleaned up on drop.
    struct Dirs {
        root: PathBuf,
        from: PathBuf,
        to: PathBuf,
    }

    impl Drop for Dirs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn dirs(tag: &str) -> Dirs {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("pv-transfer-{tag}-{}-{n}", std::process::id()));
        let from = root.join("from");
        let to = root.join("to");
        fs::create_dir_all(&from).unwrap();
        fs::create_dir_all(&to).unwrap();
        Dirs { root, from, to }
    }

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn a_move_leaves_nothing_behind() {
        let d = dirs("move");
        let src = write(&d.from, "a.jpg", "photo");

        let done = transfer(&src, &d.to, TransferKind::Move, Collision::Skip).unwrap();
        assert_eq!(done, Transferred::SourceGone);
        assert!(!src.exists());
        assert_eq!(read(&d.to.join("a.jpg")), "photo");
    }

    #[test]
    fn a_copy_keeps_the_original() {
        let d = dirs("copy");
        let src = write(&d.from, "a.jpg", "photo");

        let done = transfer(&src, &d.to, TransferKind::Copy, Collision::Skip).unwrap();
        // The wall keeps showing it, because it never left the folder.
        assert_eq!(done, Transferred::SourceKept);
        assert!(src.exists());
        assert_eq!(read(&d.to.join("a.jpg")), "photo");
    }

    #[test]
    fn skip_leaves_both_files_alone() {
        let d = dirs("skip");
        let src = write(&d.from, "a.jpg", "new");
        write(&d.to, "a.jpg", "old");

        let done = transfer(&src, &d.to, TransferKind::Move, Collision::Skip).unwrap();
        // A skipped move is not a move: the source stays selected, so a retry
        // with a different answer hits exactly the files that were skipped.
        assert_eq!(done, Transferred::SourceKept);
        assert!(src.exists());
        assert_eq!(read(&d.to.join("a.jpg")), "old");
    }

    #[test]
    fn overwrite_replaces_what_is_there() {
        let d = dirs("overwrite");
        let src = write(&d.from, "a.jpg", "new");
        write(&d.to, "a.jpg", "old");

        let done = transfer(&src, &d.to, TransferKind::Move, Collision::Overwrite).unwrap();
        assert_eq!(done, Transferred::SourceGone);
        assert_eq!(read(&d.to.join("a.jpg")), "new");
    }

    #[test]
    fn keep_both_writes_alongside() {
        let d = dirs("keep-both");
        let src = write(&d.from, "a.jpg", "new");
        write(&d.to, "a.jpg", "old");

        let done = transfer(&src, &d.to, TransferKind::Move, Collision::KeepBoth).unwrap();
        assert_eq!(done, Transferred::SourceGone);
        assert_eq!(read(&d.to.join("a.jpg")), "old");
        assert_eq!(read(&d.to.join("a-1.jpg")), "new");
    }

    #[test]
    fn keep_both_counts_past_names_already_taken() {
        let d = dirs("keep-both-run");
        write(&d.to, "a.jpg", "old");
        write(&d.to, "a-1.jpg", "older");

        let src = write(&d.from, "a.jpg", "new");
        transfer(&src, &d.to, TransferKind::Copy, Collision::KeepBoth).unwrap();
        assert_eq!(read(&d.to.join("a-2.jpg")), "new");
    }

    #[test]
    fn keep_both_handles_a_name_with_no_extension() {
        let d = dirs("keep-both-noext");
        write(&d.to, "photo", "old");
        let src = write(&d.from, "photo", "new");

        transfer(&src, &d.to, TransferKind::Copy, Collision::KeepBoth).unwrap();
        assert_eq!(read(&d.to.join("photo-1")), "new");
    }

    #[test]
    fn sending_a_file_to_its_own_folder_does_nothing() {
        let d = dirs("self");
        let src = write(&d.from, "a.jpg", "photo");

        // `rename` onto itself is a no-op but `copy` onto itself truncates, so
        // this has to be caught before either runs.
        let done = transfer(&src, &d.from, TransferKind::Copy, Collision::Overwrite).unwrap();
        assert_eq!(done, Transferred::SourceKept);
        assert_eq!(read(&src), "photo");
    }

    #[test]
    fn a_missing_source_is_an_error_not_a_panic() {
        let d = dirs("missing");
        let src = d.from.join("gone.jpg");
        assert!(transfer(&src, &d.to, TransferKind::Move, Collision::Skip).is_err());
    }

    #[test]
    fn collisions_counts_only_real_clashes() {
        let d = dirs("count");
        let a = write(&d.from, "a.jpg", "x");
        let b = write(&d.from, "b.jpg", "x");
        let c = write(&d.from, "c.jpg", "x");
        write(&d.to, "a.jpg", "x");
        write(&d.to, "c.jpg", "x");

        assert_eq!(collisions(&[a, b, c], &d.to), 2);
    }

    #[test]
    fn a_file_does_not_collide_with_itself() {
        let d = dirs("count-self");
        let a = write(&d.from, "a.jpg", "x");
        // Sending a folder to itself is refused before this, but counting it as
        // a clash would put a nonsense number in the question.
        assert_eq!(collisions(&[a], &d.from), 0);
    }
}
