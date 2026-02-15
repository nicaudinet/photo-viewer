from pathlib import Path
from unittest.mock import MagicMock

import pytest
from PIL import Image
from PySide6.QtCore import Qt
from PySide6.QtGui import QPixmap, QMouseEvent

from lib.photo import Photo, Thumbnail, ThumbnailMaker, image_to_pixmap

from tests.fixtures import tmp_image_dir, tmp_images


class TestImageToPixmap:

    def test_rgb_image(self, qtbot):
        img = Image.new("RGB", (10, 10), color="red")
        pixmap = image_to_pixmap(img)
        assert isinstance(pixmap, QPixmap)
        assert not pixmap.isNull()

    def test_rgba_image(self, qtbot):
        img = Image.new("RGBA", (10, 10), color=(255, 0, 0, 128))
        pixmap = image_to_pixmap(img)
        assert isinstance(pixmap, QPixmap)
        assert not pixmap.isNull()

    def test_grayscale_image_converts_to_rgb(self, qtbot):
        img = Image.new("L", (10, 10), color=128)
        pixmap = image_to_pixmap(img)
        assert isinstance(pixmap, QPixmap)
        assert not pixmap.isNull()


class TestPhotoIcons:

    def test_show_favourite(self, qtbot, tmp_images):
        photo = Photo(tmp_images[0], is_favourite=False, to_delete=False)
        qtbot.addWidget(photo)
        assert photo.star_label.isHidden()
        photo.show_favourite()
        assert not photo.star_label.isHidden()

    def test_hide_favourite(self, qtbot, tmp_images):
        photo = Photo(tmp_images[0], is_favourite=True, to_delete=False)
        qtbot.addWidget(photo)
        assert not photo.star_label.isHidden()
        photo.hide_favourite()
        assert photo.star_label.isHidden()

    def test_show_to_delete(self, qtbot, tmp_images):
        photo = Photo(tmp_images[0], is_favourite=False, to_delete=False)
        qtbot.addWidget(photo)
        assert photo.delete_label.isHidden()
        photo.show_to_delete()
        assert not photo.delete_label.isHidden()

    def test_hide_to_delete(self, qtbot, tmp_images):
        photo = Photo(tmp_images[0], is_favourite=False, to_delete=True)
        qtbot.addWidget(photo)
        assert not photo.delete_label.isHidden()
        photo.hide_to_delete()
        assert photo.delete_label.isHidden()


class TestThumbnailMaker:

    def test_run_emits_pixmap(self, qtbot, tmp_images):
        maker = ThumbnailMaker(tmp_images[0], width=100, height=50)
        received = []
        maker.signals.finished.connect(lambda px: received.append(px))
        maker.run()
        assert len(received) == 1
        assert isinstance(received[0], QPixmap)
        assert not received[0].isNull()


class TestThumbnailClick:

    def test_mouse_click_fires_callback(self, qtbot, tmp_images):
        callback = MagicMock()
        thumb = Thumbnail(
            image_path=tmp_images[0],
            is_favourite=False,
            to_delete=False,
            index=42,
            click_callback=callback,
            parent=None,
        )
        qtbot.addWidget(thumb)
        qtbot.mouseClick(thumb, Qt.MouseButton.LeftButton)
        callback.assert_called_once_with(42)
