from typing import Callable
from pathlib import Path
from PIL import Image

from PySide6.QtWidgets import QWidget, QLabel, QVBoxLayout
from PySide6.QtGui import QIcon, QPixmap, QResizeEvent, QMouseEvent, QImage
from PySide6.QtCore import Qt


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

        self.image: Image.Image = Image.open(image_path)

        ###########
        # Widgets #
        ###########

        self.image_label = QLabel(self)
        self.image_label.setAlignment(Qt.AlignmentFlag.AlignCenter)

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

        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.addWidget(self.image_label)

    def resizeEvent(self, event: QResizeEvent):
        super().resizeEvent(event)

        width, height = self.image_label.size().toTuple()
        ratio = min(width / self.image.width, height / self.image.height)
        size = (int(ratio * self.image.width), int(ratio * self.image.height))
        resized_image = self.image.resize(
            size=size,
            resample=Image.Resampling.BICUBIC,
        )
        self.image_label.setPixmap(self.image_to_pixmap(resized_image))

        x = self.width() - self.ICON_MARGIN - self.ICON_SIZE
        y = self.ICON_MARGIN
        self.star_label.move(x, y)
        self.delete_label.move(x, y)

    def image_to_pixmap(self, image: Image.Image) -> QPixmap:
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
                image.width * 3,
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

        self.setCursor(Qt.CursorShape.PointingHandCursor)

        aspect_ratio = self.image.height / self.image.width
        thumbnail_height = int(aspect_ratio * self.THUMBNAIL_WIDTH)
        resized_image = self.image.copy()
        resized_image.thumbnail(
            size=(self.THUMBNAIL_WIDTH, thumbnail_height),
            resample=Image.Resampling.BILINEAR,
        )
        self.image_label.setPixmap(self.image_to_pixmap(resized_image))
        self.setFixedSize(self.THUMBNAIL_WIDTH, thumbnail_height)

    def mousePressEvent(self, event: QMouseEvent):
        if event.button() == Qt.MouseButton.LeftButton:
            self.click_callback(self.index)

    def resizeEvent(self, event):
        pass
