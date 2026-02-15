from unittest.mock import patch

import pytest
from PySide6.QtCore import Qt

from lib.command import NoModifier
from lib.photo_viewer import PhotoViewer
from lib.view.single_view import SingleView
from tests.fixtures import tmp_image_dir, tmp_images


ALL_MODIFIERS = (
    NoModifier,
    Qt.KeyboardModifier.ShiftModifier,
    Qt.KeyboardModifier.ControlModifier,
    Qt.KeyboardModifier.ShiftModifier | Qt.KeyboardModifier.ControlModifier,
)


class TestPhotoViewer:

    @pytest.mark.parametrize("modifier", ALL_MODIFIERS)
    def test_pressing_q_quits_application(self, qtbot, modifier):

        viewer = PhotoViewer(None)
        qtbot.addWidget(viewer)

        viewer.show()
        qtbot.waitExposed(viewer)
        qtbot.waitActive(viewer)

        qtbot.keyClick(viewer, Qt.Key.Key_Q, modifier)
        qtbot.waitUntil(lambda: not viewer.isVisible(), timeout=1000)

    @pytest.mark.parametrize("modifier", ALL_MODIFIERS)
    def test_pressing_question_mark_toggles_help(self, qtbot, modifier):

        viewer = PhotoViewer(None)
        qtbot.addWidget(viewer)

        viewer.show()
        qtbot.waitExposed(viewer)
        qtbot.waitActive(viewer)

        assert viewer.help_overlay is None

        qtbot.keyClick(viewer, Qt.Key.Key_Question, modifier)
        assert viewer.help_overlay is not None
        assert viewer.help_overlay.isVisible()

        qtbot.keyClick(viewer, Qt.Key.Key_Question, modifier)
        assert viewer.help_overlay is None

    def test_pressing_e_toggles_fullscreen(self, qtbot):

        viewer = PhotoViewer(None)
        qtbot.addWidget(viewer)

        viewer.show()
        qtbot.waitExposed(viewer)
        qtbot.waitActive(viewer)

        assert not viewer.isFullScreen()

        qtbot.keyClick(viewer, Qt.Key.Key_E)
        assert viewer.isFullScreen()

        qtbot.keyClick(viewer, Qt.Key.Key_E)
        assert not viewer.isFullScreen()

    def test_pressing_o_opens_directory_dialog(
        self,
        qtbot,
        tmp_image_dir,
    ):
        viewer = PhotoViewer(None)
        qtbot.addWidget(viewer)

        viewer.show()
        qtbot.waitExposed(viewer)
        qtbot.waitActive(viewer)

        with patch(
            "lib.photo_viewer.QFileDialog.getExistingDirectory",
            return_value=str(tmp_image_dir),
        ) as mock_dialog:
            qtbot.keyClick(viewer, Qt.Key.Key_O)

        mock_dialog.assert_called_once_with(
            parent=viewer,
            caption="Open Image Directory",
            dir="/Users/audinet/Pictures/Camera/2025 China/Favourites",
        )

    def test_opening_directory_swaps_to_single_view(
        self,
        qtbot,
        tmp_image_dir,
        tmp_images,
    ):
        viewer = PhotoViewer(None)
        qtbot.addWidget(viewer)

        viewer.show()
        qtbot.waitExposed(viewer)
        qtbot.waitActive(viewer)

        with patch(
            "lib.photo_viewer.QFileDialog.getExistingDirectory",
            return_value=str(tmp_image_dir),
        ):
            qtbot.keyClick(viewer, Qt.Key.Key_O)

        assert isinstance(viewer.centralWidget(), SingleView)

    def test_load_path_with_file(self, qtbot, tmp_image_dir, tmp_images):
        viewer = PhotoViewer(tmp_images[1])
        qtbot.addWidget(viewer)
        assert isinstance(viewer.centralWidget(), SingleView)

    def test_load_path_with_directory(self, qtbot, tmp_image_dir, tmp_images):
        viewer = PhotoViewer(tmp_image_dir)
        qtbot.addWidget(viewer)
        assert isinstance(viewer.centralWidget(), SingleView)

    def test_load_path_raises_for_empty_directory(self, qtbot, tmp_image_dir):
        with pytest.raises(ValueError, match="is empty"):
            PhotoViewer(tmp_image_dir)

    def test_resize_with_help_overlay(self, qtbot, tmp_image_dir, tmp_images):
        viewer = PhotoViewer(tmp_image_dir)
        qtbot.addWidget(viewer)
        viewer.show()
        qtbot.waitExposed(viewer)
        qtbot.waitActive(viewer)

        # Open help overlay
        qtbot.keyClick(viewer, Qt.Key.Key_Question)
        assert viewer.help_overlay is not None

        # Trigger resize — should call help_overlay.resize_and_center
        viewer.resize(900, 700)

    def test_open_directory_cancelled(self, qtbot):
        viewer = PhotoViewer(None)
        qtbot.addWidget(viewer)
        viewer.show()
        qtbot.waitExposed(viewer)
        qtbot.waitActive(viewer)

        with patch(
            "lib.photo_viewer.QFileDialog.getExistingDirectory",
            return_value="",
        ):
            qtbot.keyClick(viewer, Qt.Key.Key_O)

        # Should still be the EmptyView since dialog was cancelled
        from lib.view.empty_view import EmptyView

        assert isinstance(viewer.centralWidget(), EmptyView)
