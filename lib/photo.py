from typing import Callable
from pathlib import Path
from PIL import Image

from PySide6.QtWidgets import QWidget, QLabel, QVBoxLayout
from PySide6.QtGui import QIcon, QPixmap, QMouseEvent, QImage
from PySide6.QtCore import Qt, QObject, Signal, Slot, QRunnable, QThreadPool


def image_to_pixmap(image: Image.Image) -> QPixmap:
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
    return QPixmap.fromImage(qimage.copy())


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

        self.image_label = QLabel(self)
        self.image_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.addWidget(self.image_label)

        self.star_label = QLabel(self)
        star_icon = QIcon("./icons/star.png")
        star_pixmap = star_icon.pixmap(self.ICON_SIZE, self.ICON_SIZE)
        self.star_label.setPixmap(star_pixmap)
        self.star_label.setFixedSize(self.ICON_SIZE, self.ICON_SIZE)
        self.star_label.hide()
        self.star_label.raise_()

        self.delete_label = QLabel(self)
        delete_icon = QIcon("./icons/delete.png")
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

    def resizeEvent(self, event):
        super().resizeEvent(event)
        image = Image.open(self.image_path)
        width, height = self.image_label.size().toTuple()
        ratio = min(width / image.width, height / image.height)
        resized_width = int(ratio * image.width)
        resized_height = int(ratio * image.height)
        resized_image = image.resize(
            size=(resized_width, resized_height),
            resample=Image.Resampling.BICUBIC,
        )
        self.image_label.setPixmap(image_to_pixmap(resized_image))


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
