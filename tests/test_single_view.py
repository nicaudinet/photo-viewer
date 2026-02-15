from unittest.mock import patch

from PIL import Image
from PySide6.QtCore import Qt
from PySide6.QtWidgets import QDialog

from lib.view.wall_view import WallView
from lib.view.single_view import DeleteConfirmDialog

from tests.fixtures import tmp_image_dir, tmp_images, image_state, single_view


class TestSingleViewNavigation:

    def test_right_arrow_advances_to_next_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        images = image_state.image_paths.list
        assert image_state.current() == images[0]
        qtbot.keyClick(single_view, Qt.Key.Key_Right)
        assert image_state.current() == images[1]

    def test_left_arrow_goes_to_previous_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        images = image_state.image_paths.list
        qtbot.keyClick(single_view, Qt.Key.Key_Right)
        assert image_state.current() == images[1]
        qtbot.keyClick(single_view, Qt.Key.Key_Left)
        assert image_state.current() == images[0]

    def test_right_arrow_wraps_to_first_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        images = image_state.image_paths.list
        for _ in range(len(images) - 1):
            qtbot.keyClick(single_view, Qt.Key.Key_Right)
        assert image_state.current() == images[-1]
        qtbot.keyClick(single_view, Qt.Key.Key_Right)
        assert image_state.current() == images[0]

    def test_left_arrow_wraps_to_last_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        images = image_state.image_paths.list
        assert image_state.current() == images[0]
        qtbot.keyClick(single_view, Qt.Key.Key_Left)
        assert image_state.current() == images[-1]


class TestSingleViewRotation:

    def test_r_rotates_current_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        image_path = image_state.current()
        assert Image.open(image_path).size == (100, 50)
        qtbot.keyClick(single_view, Qt.Key.Key_R)
        assert Image.open(image_path).size == (50, 100)

    def test_r_only_rotates_current_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        images = image_state.image_paths.list
        qtbot.keyClick(single_view, Qt.Key.Key_R)
        for image_path in images[1:]:
            assert Image.open(image_path).size == (100, 50)


class TestSingleViewSwap:

    def test_w_switches_to_wall_view(self, qtbot, single_view):
        qtbot.keyClick(single_view, Qt.Key.Key_W)
        assert isinstance(single_view.centralWidget(), WallView)


class TestSingleViewFavourite:

    def test_f_favourites_current_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        image_path = image_state.current()
        assert image_path not in image_state.favourites
        qtbot.keyClick(single_view, Qt.Key.Key_F)
        assert image_path in image_state.favourites

    def test_f_unfavourites_already_favourited_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        image_path = image_state.current()
        qtbot.keyClick(single_view, Qt.Key.Key_F)
        assert image_path in image_state.favourites
        qtbot.keyClick(single_view, Qt.Key.Key_F)
        assert image_path not in image_state.favourites


class TestSingleViewDelete:

    def test_d_marks_current_image_for_deletion(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        image_path = image_state.current()
        assert image_path not in image_state.to_delete
        qtbot.keyClick(single_view, Qt.Key.Key_D)
        assert image_path in image_state.to_delete

    def test_d_unmarks_already_marked_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        image_path = image_state.current()
        qtbot.keyClick(single_view, Qt.Key.Key_D)
        assert image_path in image_state.to_delete
        qtbot.keyClick(single_view, Qt.Key.Key_D)
        assert image_path not in image_state.to_delete


class TestSingleViewSaveFavourites:

    def test_ctrl_f_copies_favourites_to_chosen_directory(
        self,
        qtbot,
        single_view,
        image_state,
        tmp_path,
    ):
        dest_dir = tmp_path / "favourites"
        dest_dir.mkdir()
        qtbot.keyClick(single_view, Qt.Key.Key_F)
        favourited = image_state.current()
        with patch(
            "lib.view.single_view.QFileDialog.getExistingDirectory",
            return_value=str(dest_dir),
        ):
            qtbot.keyClick(
                single_view,
                Qt.Key.Key_F,
                Qt.KeyboardModifier.ControlModifier,
            )
        assert (dest_dir / favourited.name).exists()


class TestSingleViewDeleteAll:

    def test_ctrl_d_does_nothing_when_dialog_rejected(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        qtbot.keyClick(single_view, Qt.Key.Key_D)
        marked = image_state.current()
        with patch(
            "lib.view.single_view.DeleteConfirmDialog.exec",
            return_value=QDialog.DialogCode.Rejected,
        ):
            qtbot.keyClick(
                single_view,
                Qt.Key.Key_D,
                Qt.KeyboardModifier.ControlModifier,
            )
        assert marked.exists()
        assert marked in image_state.to_delete

    def test_ctrl_d_deletes_marked_images(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        qtbot.keyClick(single_view, Qt.Key.Key_D)
        marked = image_state.current()
        assert marked.exists()
        with patch(
            "lib.view.single_view.DeleteConfirmDialog.exec",
            return_value=QDialog.DialogCode.Accepted,
        ):
            qtbot.keyClick(
                single_view,
                Qt.Key.Key_D,
                Qt.KeyboardModifier.ControlModifier,
            )
        assert not marked.exists()
        assert marked not in image_state.to_delete
        assert marked not in image_state.image_paths


class TestDeleteConfirmDialogMessage:

    def test_singular_message(self, qtbot):
        dialog = DeleteConfirmDialog(1)
        qtbot.addWidget(dialog)
        assert dialog.windowTitle() == "Confirm Deletion"

    def test_plural_message(self, qtbot):
        dialog = DeleteConfirmDialog(5)
        qtbot.addWidget(dialog)
        assert dialog.windowTitle() == "Confirm Deletion"


class TestSingleViewRotateError:

    def test_rotate_handles_invalid_image(
        self,
        qtbot,
        single_view,
        image_state,
        capsys,
    ):
        image_path = image_state.current()
        # Overwrite the current image with invalid data
        image_path.write_text("not an image")
        qtbot.keyClick(single_view, Qt.Key.Key_R)
        captured = capsys.readouterr()
        assert "Error rotating image" in captured.out
        # Remove the corrupt file so LargePhoto.resizeEvent doesn't crash
        image_path.unlink()


class TestSingleViewDeleteDoesNotMarkFavourite:

    def test_d_does_not_mark_favourited_image_for_deletion(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        image_path = image_state.current()
        # First favourite the image
        qtbot.keyClick(single_view, Qt.Key.Key_F)
        assert image_path in image_state.favourites
        # Try to mark for deletion — should be a no-op
        qtbot.keyClick(single_view, Qt.Key.Key_D)
        assert image_path not in image_state.to_delete
