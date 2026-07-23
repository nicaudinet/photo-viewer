from typing import Callable
from pathlib import Path
from PIL import Image

from PySide6.QtCore import Qt, QObject, Signal, Slot, QRunnable, QThreadPool
from PySide6.QtWidgets import QWidget, QLabel, QVBoxLayout
from PySide6.QtGui import (
    QIcon,
    QPixmap,
    QMouseEvent,
    QImage,
    QPen,
    QPainter,
    QPalette,
)


def image_to_qimage(image: Image.Image) -> QImage:
    # QImage does not copy the buffer it is handed, so `.copy()` before the
    # source bytes go out of scope. QImage is safe to build off the GUI
    # thread (unlike QPixmap), so decode workers use this directly.
    if image.mode == "RGB":
        data = image.tobytes("raw", "RGB")
        qimage = QImage(
            data,
            image.width,
            image.height,
            image.width * 3,
            QImage.Format.Format_RGB888,
        )
    elif image.mode == "RGBA":
        data = image.tobytes("raw", "RGBA")
        qimage = QImage(
            data,
            image.width,
            image.height,
            image.width * 4,
            QImage.Format.Format_RGBA8888,
        )
    else:
        image = image.convert("RGB")
        data = image.tobytes("raw", "RGB")
        qimage = QImage(
            data,
            image.width,
            image.height,
            image.width * 3,
            QImage.Format.Format_RGB888,
        )
    return qimage.copy()


def image_to_pixmap(image: Image.Image) -> QPixmap:
    return QPixmap.fromImage(image_to_qimage(image))


class ImageLabel(QLabel):

    def __init__(self, parent=None):
        super().__init__(parent)
        self.selected = False

    def paintEvent(self, event):

        super().paintEvent(event)

        if self.selected:

            color = self.palette().color(QPalette.ColorRole.Highlight)

            pen = QPen(color)
            pen.setWidth(4)
            pen.setJoinStyle(Qt.PenJoinStyle.MiterJoin)

            painter = QPainter(self)
            painter.setPen(pen)

            offset: int = (1 + pen.width()) // 2
            rect = self.rect().adjusted(offset, offset, -offset, -offset)

            painter.drawRect(rect)

    def setSelected(self, selected: bool) -> None:
        print("setting selected")
        self.selected = selected
        self.update()


class Photo(QWidget):

    ICON_SIZE: int = 40
    ICON_MARGIN: int = 10

    def __init__(
        self,
        image_path: Path,
        is_favourite: bool,
        to_delete: bool,
        parent=None,
    ):

        super().__init__(parent)

        #########
        # State #
        #########

        self.image_path: Path = image_path
        self.is_favourite = is_favourite
        self.to_delete = to_delete

        ###########
        # Widgets #
        ###########

        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)

        self.image_label = ImageLabel(self)
        self.image_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.addWidget(self.image_label)

        _icons_dir = Path(__file__).parent.parent / "icons"

        self.star_label = QLabel(self)
        star_icon = QIcon(str(_icons_dir / "star.png"))
        star_pixmap = star_icon.pixmap(self.ICON_SIZE, self.ICON_SIZE)
        self.star_label.setPixmap(star_pixmap)
        self.star_label.setFixedSize(self.ICON_SIZE, self.ICON_SIZE)
        self.star_label.hide()
        self.star_label.raise_()

        self.delete_label = QLabel(self)
        delete_icon = QIcon(str(_icons_dir / "delete.png"))
        delete_pixmap = delete_icon.pixmap(self.ICON_SIZE, self.ICON_SIZE)
        self.delete_label.setPixmap(delete_pixmap)
        self.delete_label.setFixedSize(self.ICON_SIZE, self.ICON_SIZE)
        self.delete_label.hide()
        self.delete_label.raise_()

        ########
        # Init #
        ########

        if is_favourite:
            self.star_label.show()
        elif to_delete:
            self.delete_label.show()
        else:
            self.star_label.hide()
            self.delete_label.hide()

    def resizeEvent(self, event):
        x = self.width() - self.ICON_MARGIN - self.ICON_SIZE
        y = self.ICON_MARGIN
        self.star_label.move(x, y)
        self.delete_label.move(x, y)

    def show_favourite(self):
        self.star_label.show()

    def hide_favourite(self):
        self.star_label.hide()

    def show_to_delete(self):
        self.delete_label.show()

    def hide_to_delete(self):
        self.delete_label.hide()


class LargeImageSignals(QObject):
    # generation lets the receiver discard results from superseded resizes
    finished = Signal(int, QImage)


class LargeImageMaker(QRunnable):
    """Decode + resize the full image to fit a target size, off the GUI thread.

    Emits the tagged `generation` back so a stale result (a resize that has
    since been replaced by a newer one) can be dropped rather than painted.
    """

    def __init__(
        self,
        image_path: Path,
        width: int,
        height: int,
        generation: int,
    ):
        super().__init__()
        self.image_path = image_path
        self.size = (width, height)
        self.generation = generation
        self.signals = LargeImageSignals()

    def run(self):
        try:
            image = Image.open(self.image_path)
            width, height = self.size
            ratio = min(width / image.width, height / image.height)
            resized_image = image.resize(
                size=(
                    max(1, int(ratio * image.width)),
                    max(1, int(ratio * image.height)),
                ),
                resample=Image.Resampling.BICUBIC,
            )
            qimage = image_to_qimage(resized_image)
        except Exception as e:
            # A missing or corrupt file should not take down the worker thread
            print(f"Error decoding image {self.image_path}: {e}")
            return
        self.signals.finished.emit(self.generation, qimage)


class LargePhoto(Photo):

    def __init__(
        self,
        image_path: Path,
        is_favourite: bool,
        to_delete: bool,
        parent,
    ):

        super().__init__(
            image_path=image_path,
            is_favourite=is_favourite,
            to_delete=to_delete,
            parent=parent,
        )

        self._threadpool = QThreadPool.globalInstance()
        # Bumped on every resize; only the latest decode is allowed to paint
        self._decode_generation = 0
        self.image_label.setText("Loading …")

    def resizeEvent(self, event):
        super().resizeEvent(event)
        if not self.image_path.exists():
            return
        width, height = self.image_label.size().toTuple()
        if width <= 0 or height <= 0:
            return
        self._decode_generation += 1
        maker = LargeImageMaker(
            image_path=self.image_path,
            width=width,
            height=height,
            generation=self._decode_generation,
        )
        maker.signals.finished.connect(self.on_image_decoded)
        self._threadpool.start(maker)

    @Slot(int, QImage)
    def on_image_decoded(self, generation: int, qimage: QImage):
        if generation != self._decode_generation:
            return  # a newer resize superseded this decode; drop it
        self.image_label.setPixmap(QPixmap.fromImage(qimage))


class ThumbnailSignals(QObject):
    finished = Signal(QPixmap)


class ThumbnailMaker(QRunnable):

    def __init__(self, image_path: Path, width: int, height: int):
        super().__init__()
        self.image_path = image_path
        self.size = (width, height)
        self.signals = ThumbnailSignals()

    def run(self):
        image = Image.open(self.image_path)
        image.thumbnail(self.size, Image.Resampling.BILINEAR)
        pixmap = image_to_pixmap(image)
        self.signals.finished.emit(pixmap)


class Thumbnail(Photo):

    THUMBNAIL_WIDTH: int = 300

    def __init__(
        self,
        image_path: Path,
        is_favourite: bool,
        to_delete: bool,
        index: int,
        click_callback: Callable[[int], None],
        parent,
    ):

        super().__init__(
            image_path=image_path,
            is_favourite=is_favourite,
            to_delete=to_delete,
            parent=parent,
        )

        #########
        # State #
        #########

        self.index: int = index
        self.click_callback: Callable = click_callback

        ########
        # Init #
        ########

        self.image_label.setText("Loading ...")

        self.setCursor(Qt.CursorShape.PointingHandCursor)

        image = Image.open(self.image_path)
        aspect_ratio = image.height / image.width
        thumbnail_height = int(aspect_ratio * self.THUMBNAIL_WIDTH)
        self.setFixedSize(self.THUMBNAIL_WIDTH, thumbnail_height)

    def make_thumbnail_async(self, threadpool: QThreadPool):
        maker = ThumbnailMaker(
            image_path=self.image_path,
            width=self.width(),
            height=self.height(),
        )
        maker.signals.finished.connect(self.on_thumbnail_made)
        threadpool.start(maker)

    @Slot(QPixmap)
    def on_thumbnail_made(self, pixmap: QPixmap):
        self.image_label.setPixmap(pixmap)

    def mousePressEvent(self, event: QMouseEvent):
        if event.button() == Qt.MouseButton.LeftButton:
            self.click_callback(self.index)
