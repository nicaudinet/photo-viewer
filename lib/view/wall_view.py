from typing import Optional, Callable, List
from pathlib import Path

from PySide6.QtWidgets import (
    QWidget,
    QLabel,
    QScrollArea,
)
from PySide6.QtGui import QPixmap, QMouseEvent
from PySide6.QtCore import Qt


class Thumbnail(QLabel):

    THUMBNAIL_WIDTH = 300

    def __init__(self, file_path: Path, index: int, parent=None):

        super().__init__(parent)

        self.index: int = index
        self.click_callback: Optional[Callable] = None

        pixmap = QPixmap(file_path)
        pixmap = pixmap.scaledToWidth(
            self.THUMBNAIL_WIDTH,
            Qt.TransformationMode.SmoothTransformation,
        )
        self.setPixmap(pixmap)
        self.setFixedSize(pixmap.size())

        self.setCursor(Qt.CursorShape.PointingHandCursor)

    def setClickCallback(self, callback: Callable):
        self.click_callback = callback

    def mousePressEvent(self, event: QMouseEvent):
        if event.button() == Qt.MouseButton.LeftButton and self.click_callback:
            self.click_callback(self.index)


class MasonryWall(QWidget):

    def __init__(self, parent=None):

        super().__init__(parent)

        self.thumbnails: List[Thumbnail] = []
        self.image_paths: List[Path] = []

        self.column_count: int = 3
        self.spacing: int = 10
        self.thumbnail_width: int = 300
        self.click_callback: Optional[Callable] = None

    def setClickCallback(self, callback: Callable):
        self.click_callback = callback
        for thumbnail in self.thumbnails:
            thumbnail.setClickCallback(callback)

    def setImages(self, image_paths: List[Path]):

        self.image_paths = image_paths

        for thumbnail in self.thumbnails:
            thumbnail.deleteLater()
        self.thumbnails.clear()

        for i, image_path in enumerate(image_paths):
            thumbnail = Thumbnail(image_path, i, self)
            if self.click_callback:
                thumbnail.setClickCallback(self.click_callback)
            self.thumbnails.append(thumbnail)

        self.layout_masonry()

    def layout_masonry(self):

        if not self.thumbnails:
            return

        column_width = Thumbnail.THUMBNAIL_WIDTH
        column_heights = [self.spacing] * self.column_count

        for thumbnail in self.thumbnails:
            shortest_column = column_heights.index(min(column_heights))
            x = self.spacing + shortest_column * (column_width + self.spacing)
            y = column_heights[shortest_column]
            thumbnail.move(x, y)
            column_heights[shortest_column] += thumbnail.height() + self.spacing

        for thumbnail in self.thumbnails:
            thumbnail.show()

        self.setMinimumHeight(max(column_heights))


class WallView(QScrollArea):

    def __init__(self, parent=None):

        super().__init__(parent)

        self.masonry_wall = MasonryWall()
        self.setWidget(self.masonry_wall)
        self.setWidgetResizable(True)
        self.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAsNeeded)

    def setImages(self, image_paths: List[Path]):
        self.masonry_wall.setImages(image_paths)

    def setClickCallback(self, callback: Callable):
        self.masonry_wall.setClickCallback(callback)
