//! Named tags over the images in one directory, and their on-disk store.
//!
//! Favourites used to be a hard-coded set with its own cache file (removed in
//! phase 0 of `SELECT_MODE_PLAN.md`). It comes back here as one tag among
//! however many, because "favourite" is a judgement the user makes and there is
//! no reason the app should be the one deciding it is the only judgement worth
//! recording. Only [`FAVOURITE`] is reachable from the keyboard so far; the
//! store already handles the rest.
//!
//! One file per tag under `<image_dir>/.photo-viewer/tags/`, each a
//! newline-separated list of **file names** — not absolute paths, as the old
//! cache held. The store lives in the folder it describes, so naming its
//! contents relative to it means renaming or moving that folder keeps the tags
//! attached to the photos.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The one tag the keyboard exposes.
pub(crate) const FAVOURITE: &str = "favourite";

/// Tag name to the images carrying it.
///
/// Keyed by name so filtering is one lookup, and ordered so the store is
/// written the same way twice running.
pub(crate) type Tags = BTreeMap<String, HashSet<PathBuf>>;

fn store_dir(image_dir: &Path) -> PathBuf {
    image_dir.join(".photo-viewer").join("tags")
}

/// Read every tag in `image_dir`, keeping only entries that are still images in
/// it — a tag naming a photo that has since been deleted is noise, and the next
/// [`save`] drops it for good.
pub(crate) fn load(image_dir: &Path, images: &[PathBuf]) -> Tags {
    let known: HashSet<&PathBuf> = images.iter().collect();
    let mut tags = Tags::new();

    if let Ok(entries) = fs::read_dir(store_dir(image_dir)) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let paths = read_names(&path, image_dir, &known);
            if !paths.is_empty() {
                tags.insert(name.to_string(), paths);
            }
        }
    }

    // Phase 0 left the old `.photo-viewer/favourites` in place precisely so it
    // could be picked up here. It holds absolute paths; only its file names
    // survive the move to the new store.
    if !tags.contains_key(FAVOURITE) {
        let legacy = image_dir.join(".photo-viewer").join("favourites");
        let paths = read_names(&legacy, image_dir, &known);
        if !paths.is_empty() {
            tags.insert(FAVOURITE.to_string(), paths);
        }
    }

    tags
}

/// Read one tag file: one entry per line, each taken as a file name in
/// `image_dir`. Absolute paths are accepted too, so the old cache can be read
/// by the same code.
fn read_names(file: &Path, image_dir: &Path, known: &HashSet<&PathBuf>) -> HashSet<PathBuf> {
    let Ok(contents) = fs::read_to_string(file) else {
        return HashSet::new();
    };
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let name = Path::new(line).file_name()?;
            let path = image_dir.join(name);
            known.contains(&path).then_some(path)
        })
        .collect()
}

/// Write the whole store, replacing what was there.
///
/// Every tag is written from memory each time, which is what prunes entries for
/// photos that have since gone. A tag that has emptied loses its file rather
/// than being left behind as a stale one.
pub(crate) fn save(image_dir: &Path, tags: &Tags) -> Result<(), String> {
    let dir = store_dir(image_dir);
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;

    // Anything already there and not in `tags` is a tag the user emptied.
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let stale = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| !tags.contains_key(name));
            if stale {
                let _ = fs::remove_file(path);
            }
        }
    }

    for (name, paths) in tags {
        let mut names: Vec<String> = paths
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        names.sort();
        let file = dir.join(name);
        fs::write(&file, names.join("\n"))
            .map_err(|e| format!("Could not write {}: {e}", file.display()))?;
    }
    Ok(())
}

/// [`save`] off the GUI thread.
///
/// Run after every change rather than behind a key of its own: a favourite the
/// app forgot because nobody saved it is worse than any number of tiny writes,
/// and the write is a list of file names.
pub(crate) async fn save_async(image_dir: PathBuf, tags: Tags) -> Result<(), String> {
    tokio::task::spawn_blocking(move || save(&image_dir, &tags))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
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

    /// A directory holding `a.jpg`, `b.jpg` and `c.jpg`.
    fn dir(tag: &str) -> (Dir, Vec<PathBuf>) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("pv-tags-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        let images: Vec<PathBuf> = ["a.jpg", "b.jpg", "c.jpg"]
            .iter()
            .map(|n| {
                let p = path.join(n);
                fs::write(&p, "image").unwrap();
                p
            })
            .collect();
        (Dir(path), images)
    }

    fn favourites(images: &[PathBuf], which: &[usize]) -> Tags {
        let set: HashSet<PathBuf> = which.iter().map(|i| images[*i].clone()).collect();
        Tags::from([(FAVOURITE.to_string(), set)])
    }

    #[test]
    fn tags_survive_a_round_trip() {
        let (d, images) = dir("round-trip");
        save(&d.0, &favourites(&images, &[0, 2])).unwrap();

        let loaded = load(&d.0, &images);
        assert_eq!(loaded, favourites(&images, &[0, 2]));
    }

    #[test]
    fn an_untagged_directory_loads_empty() {
        let (d, images) = dir("empty");
        assert!(load(&d.0, &images).is_empty());
    }

    #[test]
    fn the_store_holds_names_not_paths() {
        let (d, images) = dir("names");
        save(&d.0, &favourites(&images, &[1])).unwrap();

        let file = d.0.join(".photo-viewer").join("tags").join(FAVOURITE);
        // Names, so renaming or moving the folder keeps the tags attached.
        assert_eq!(fs::read_to_string(file).unwrap(), "b.jpg");
    }

    #[test]
    fn a_tag_naming_a_deleted_photo_is_dropped() {
        let (d, images) = dir("stale");
        save(&d.0, &favourites(&images, &[0, 1])).unwrap();
        fs::remove_file(&images[1]).unwrap();

        let loaded = load(&d.0, &images[..1]);
        assert_eq!(loaded, favourites(&images, &[0]));
    }

    #[test]
    fn an_emptied_tag_loses_its_file() {
        let (d, images) = dir("emptied");
        save(&d.0, &favourites(&images, &[0])).unwrap();
        save(&d.0, &Tags::new()).unwrap();

        // Left behind, it would come back the next time the folder is opened.
        let file = d.0.join(".photo-viewer").join("tags").join(FAVOURITE);
        assert!(!file.exists());
        assert!(load(&d.0, &images).is_empty());
    }

    #[test]
    fn several_tags_live_side_by_side() {
        let (d, images) = dir("several");
        let tags = Tags::from([
            (FAVOURITE.to_string(), HashSet::from([images[0].clone()])),
            ("print".to_string(), HashSet::from([images[2].clone()])),
        ]);
        save(&d.0, &tags).unwrap();
        assert_eq!(load(&d.0, &images), tags);
    }

    #[test]
    fn the_old_favourites_cache_is_read_once() {
        let (d, images) = dir("legacy");
        let cache = d.0.join(".photo-viewer");
        fs::create_dir_all(&cache).unwrap();
        // The old format: absolute paths, newline-joined, no trailing newline.
        let body = format!("{}\n{}", images[0].display(), images[2].display());
        fs::write(cache.join("favourites"), body).unwrap();

        let loaded = load(&d.0, &images);
        assert_eq!(loaded, favourites(&images, &[0, 2]));

        // Once saved into the new store, the new store is what is read — the
        // legacy file is left alone but no longer consulted.
        save(&d.0, &favourites(&images, &[1])).unwrap();
        assert_eq!(load(&d.0, &images), favourites(&images, &[1]));
    }

    #[test]
    fn a_legacy_entry_for_a_missing_photo_is_dropped() {
        let (d, images) = dir("legacy-stale");
        let cache = d.0.join(".photo-viewer");
        fs::create_dir_all(&cache).unwrap();
        let body = format!("{}\n{}/gone.jpg", images[0].display(), d.0.display());
        fs::write(cache.join("favourites"), body).unwrap();

        assert_eq!(load(&d.0, &images), favourites(&images, &[0]));
    }
}
