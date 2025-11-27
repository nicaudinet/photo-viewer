from pathlib import Path

from PySide6.QtWidgets import QWidget, QLabel, QVBoxLayout
from PySide6.QtGui import QIcon, QPixmap, QResizeEvent
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

        self.image_path = image_path

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

    def resizeEvent(self, event: QResizeEvent):
        super().resizeEvent(event)
        pixmap = QPixmap(self.image_path)
        pixmap = pixmap.scaled(
            self.image_label.size(),
            Qt.AspectRatioMode.KeepAspectRatio,
            Qt.TransformationMode.SmoothTransformation,
        )
        self.image_label.setPixmap(pixmap)
        x = self.width() - self.ICON_MARGIN - self.ICON_SIZE
        y = self.ICON_MARGIN
        self.star_label.move(x, y)
        self.delete_label.move(x, y)
