import pytest
from pathlib import Path
from unittest.mock import patch

from lib.state import ImageState, load_image_state, IMAGE_EXTENSIONS
from lib.pointed_list import PointedList

from tests.fixtures import tmp_image_dir, tmp_images, image_state


class TestNavigation:
    def test_current_returns_first_image(self, image_state, tmp_images):
        assert image_state.current() == tmp_images[0]

    def test_next_moves_forward(self, image_state, tmp_images):
        image_state.next()
        assert image_state.current() == tmp_images[1]

    def test_prev_moves_backward(self, image_state, tmp_images):
        image_state.next()
        image_state.next()
        image_state.prev()
        assert image_state.current() == tmp_images[1]

    def test_goto_jumps_to_index(self, image_state, tmp_images):
        image_state.goto(2)
        assert image_state.current() == tmp_images[2]


class TestFavourites:
    def test_favourite_adds_to_set(self, image_state, tmp_images):
        image_state.favourite(tmp_images[0])
        assert tmp_images[0] in image_state.favourites

    def test_favourite_persists_to_file(self, image_state, tmp_images):
        image_state.favourite(tmp_images[0])
        assert image_state.favourites_file.exists()
        content = image_state.favourites_file.read_text()
        assert str(tmp_images[0]) in content

    def test_favourite_asserts_invalid_path(self, image_state, tmp_path):
        invalid_path = tmp_path / "nonexistent.png"
        with pytest.raises(AssertionError):
            image_state.favourite(invalid_path)

    def test_unfavourite_removes_from_set(self, image_state, tmp_images):
        image_state.favourite(tmp_images[0])
        image_state.unfavourite(tmp_images[0])
        assert tmp_images[0] not in image_state.favourites

    def test_unfavourite_noop_if_not_favourite(self, image_state, tmp_images):
        image_state.unfavourite(tmp_images[0])
        assert tmp_images[0] not in image_state.favourites


class TestDelete:
    def test_delete_adds_to_set(self, image_state, tmp_images):
        image_state.delete(tmp_images[0])
        assert tmp_images[0] in image_state.to_delete

    def test_delete_persists_to_file(self, image_state, tmp_images):
        image_state.delete(tmp_images[0])
        assert image_state.to_delete_file.exists()
        content = image_state.to_delete_file.read_text()
        assert str(tmp_images[0]) in content

    def test_delete_asserts_invalid_path(self, image_state, tmp_path):
        invalid_path = tmp_path / "nonexistent.png"
        with pytest.raises(AssertionError):
            image_state.delete(invalid_path)

    def test_undelete_removes_from_set(self, image_state, tmp_images):
        image_state.delete(tmp_images[0])
        image_state.undelete(tmp_images[0])
        assert tmp_images[0] not in image_state.to_delete

    def test_undelete_noop_if_not_marked(self, image_state, tmp_images):
        image_state.undelete(tmp_images[0])
        assert tmp_images[0] not in image_state.to_delete


class TestDeleteAll:
    def test_delete_all_removes_files_from_disk(self, image_state, tmp_images):
        image_state.delete(tmp_images[0])
        image_state.delete(tmp_images[1])
        image_state.delete_all()
        assert not tmp_images[0].exists()
        assert not tmp_images[1].exists()
        assert tmp_images[2].exists()

    def test_delete_all_removes_from_image_paths(self, image_state, tmp_images):
        image_state.delete(tmp_images[0])
        image_state.delete_all()
        assert tmp_images[0] not in image_state.image_paths

    def test_delete_all_clears_to_delete_set(self, image_state, tmp_images):
        image_state.delete(tmp_images[0])
        image_state.delete(tmp_images[1])
        image_state.delete_all()
        assert len(image_state.to_delete) == 0

    def test_delete_all_noop_when_empty(self, image_state, tmp_images):
        image_state.delete_all()
        assert all(img.exists() for img in tmp_images)
        assert len(list(image_state.image_paths)) == 3


class TestSave:
    def test_save_creates_cache_directory(self, image_state):
        assert not image_state.cache_dir.exists()
        image_state.save()
        assert image_state.cache_dir.exists()

    def test_save_writes_favourites_file(self, image_state, tmp_images):
        image_state.favourites.add(tmp_images[0])
        image_state.favourites.add(tmp_images[1])
        image_state.save()
        content = image_state.favourites_file.read_text()
        assert str(tmp_images[0]) in content
        assert str(tmp_images[1]) in content

    def test_save_writes_to_delete_file(self, image_state, tmp_images):
        image_state.to_delete.add(tmp_images[0])
        image_state.save()
        content = image_state.to_delete_file.read_text()
        assert str(tmp_images[0]) in content

    def test_save_handles_empty_sets(self, image_state):
        image_state.save()
        assert image_state.favourites_file.read_text() == ""
        assert image_state.to_delete_file.read_text() == ""


class TestSaveFavourites:
    def test_save_favourites_copies_to_destination(
        self, image_state, tmp_images, tmp_path
    ):
        dest_dir = tmp_path / "exported"
        dest_dir.mkdir()
        image_state.favourites.add(tmp_images[0])
        image_state.favourites.add(tmp_images[1])
        image_state.save_favourites(dest_dir)
        assert (dest_dir / tmp_images[0].name).exists()
        assert (dest_dir / tmp_images[1].name).exists()

    def test_save_favourites_skips_existing_files(
        self, image_state, tmp_images, tmp_path, capsys
    ):
        dest_dir = tmp_path / "exported"
        dest_dir.mkdir()
        existing = dest_dir / tmp_images[0].name
        existing.write_text("existing content")
        image_state.favourites.add(tmp_images[0])
        image_state.save_favourites(dest_dir)
        assert existing.read_text() == "existing content"
        captured = capsys.readouterr()
        assert "already exists" in captured.out


class TestLoadImageState:
    def test_load_raises_if_directory_not_exists(self, tmp_path):
        nonexistent = tmp_path / "does_not_exist"
        with pytest.raises(ValueError, match="does not exist"):
            load_image_state(nonexistent)

    def test_load_raises_if_path_not_directory(self, tmp_path):
        file_path = tmp_path / "file.txt"
        file_path.write_text("content")
        with pytest.raises(ValueError, match="is not a directory"):
            load_image_state(file_path)

    def test_load_returns_none_if_no_images(self, tmp_image_dir):
        (tmp_image_dir / "file.txt").write_text("not an image")
        result = load_image_state(tmp_image_dir)
        assert result is None

    def test_load_finds_png_jpg_jpeg_files(self, tmp_image_dir):
        (tmp_image_dir / "a.png").write_text("png")
        (tmp_image_dir / "b.jpg").write_text("jpg")
        (tmp_image_dir / "c.jpeg").write_text("jpeg")
        (tmp_image_dir / "d.gif").write_text("gif")
        state = load_image_state(tmp_image_dir)
        paths = list(state.image_paths)
        assert len(paths) == 3
        assert any(p.suffix == ".png" for p in paths)
        assert any(p.suffix == ".jpg" for p in paths)
        assert any(p.suffix == ".jpeg" for p in paths)

    def test_load_case_insensitive_extensions(self, tmp_image_dir):
        (tmp_image_dir / "a.PNG").write_text("png")
        (tmp_image_dir / "b.JPG").write_text("jpg")
        (tmp_image_dir / "c.JPEG").write_text("jpeg")
        state = load_image_state(tmp_image_dir)
        assert len(list(state.image_paths)) == 3

    def test_load_sorts_images_alphabetically(self, tmp_image_dir):
        (tmp_image_dir / "c.png").write_text("c")
        (tmp_image_dir / "a.png").write_text("a")
        (tmp_image_dir / "b.png").write_text("b")
        state = load_image_state(tmp_image_dir)
        paths = list(state.image_paths)
        names = [p.name for p in paths]
        assert names == ["a.png", "b.png", "c.png"]

    def test_load_reads_cached_favourites(self, tmp_image_dir):
        img = tmp_image_dir / "a.png"
        img.write_text("image")
        cache_dir = tmp_image_dir / ".photo-viewer"
        cache_dir.mkdir()
        favourites_file = cache_dir / "favourites"
        favourites_file.write_text(str(img))
        state = load_image_state(tmp_image_dir)
        assert img in state.favourites

    def test_load_reads_cached_to_delete(self, tmp_image_dir):
        img = tmp_image_dir / "a.png"
        img.write_text("image")
        cache_dir = tmp_image_dir / ".photo-viewer"
        cache_dir.mkdir()
        to_delete_file = cache_dir / "to_delete"
        to_delete_file.write_text(str(img))
        state = load_image_state(tmp_image_dir)
        assert img in state.to_delete

    def test_load_ignores_stale_cached_paths(self, tmp_image_dir, capsys):
        img = tmp_image_dir / "a.png"
        img.write_text("image")
        cache_dir = tmp_image_dir / ".photo-viewer"
        cache_dir.mkdir()
        favourites_file = cache_dir / "favourites"
        favourites_file.write_text("/nonexistent/image.png")
        state = load_image_state(tmp_image_dir)
        assert len(state.favourites) == 0
        captured = capsys.readouterr()
        assert "not in image paths" in captured.out

    def test_load_ignores_stale_to_delete_paths(self, tmp_image_dir, capsys):
        img = tmp_image_dir / "a.png"
        img.write_text("image")
        cache_dir = tmp_image_dir / ".photo-viewer"
        cache_dir.mkdir()
        to_delete_file = cache_dir / "to_delete"
        to_delete_file.write_text("/nonexistent/image.png")
        state = load_image_state(tmp_image_dir)
        assert len(state.to_delete) == 0
        captured = capsys.readouterr()
        assert "not in image paths" in captured.out

    def test_load_initializes_empty_without_cache(self, tmp_image_dir, capsys):
        (tmp_image_dir / "a.png").write_text("image")
        state = load_image_state(tmp_image_dir)
        assert len(state.favourites) == 0
        assert len(state.to_delete) == 0
        captured = capsys.readouterr()
        assert "not found in cache" in captured.out


class TestDeleteAllSkipsNonMarked:
    def test_delete_all_skips_unmarked_images(self, image_state, tmp_images):
        """Mark only the second image; delete_all must skip past the first."""
        image_state.delete(tmp_images[1])
        image_state.delete_all()
        assert tmp_images[0].exists()
        assert not tmp_images[1].exists()
        assert tmp_images[2].exists()
        assert len(image_state.to_delete) == 0
