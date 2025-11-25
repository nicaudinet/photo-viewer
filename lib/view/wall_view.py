from typing import Callable, List
from pathlib import Path

from PySide6.QtWidgets import QWidget, QLabel, QScrollArea
from PySide6.QtGui import QPixmap, QMouseEvent
from PySide6.QtCore import Qt

from lib.pointed_list import PointedList


class Thumbnail(QLabel):

    THUMBNAIL_WIDTH = 300

    def __init__(
        self,
        file_path: Path,
        index: int,
        click_callback: Callable,
        parent=None,
    ):

        super().__init__(parent)

        self.index: int = index
        self.click_callback: Callable = click_callback

        pixmap = QPixmap(file_path)
        pixmap = pixmap.scaledToWidth(
            self.THUMBNAIL_WIDTH,
            Qt.TransformationMode.SmoothTransformation,
        )
        self.setPixmap(pixmap)
        self.setFixedSize(pixmap.size())

        self.setCursor(Qt.CursorShape.PointingHandCursor)

    def mousePressEvent(self, event: QMouseEvent):
        if event.button() == Qt.MouseButton.LeftButton:
            self.click_callback(self.index)


class MasonryWall(QWidget):

    def __init__(
        self,
        image_paths: PointedList[Path],
        click_callback: Callable,
        parent=None,
    ):

        super().__init__(parent)

        ############
        # Settings #
        ############

        self.column_count: int = 3
        self.spacing: int = 10
        self.thumbnail_width: int = 300

        #########
        # State #
        #########

        self.thumbnails: List[Thumbnail] = []
        self.image_paths: PointedList[Path] = image_paths

        ########
        # Init #
        ########

        def callback(index: int):
            self.image_paths.goto(index)
            return click_callback(image_paths)

        for i, image_path in enumerate(image_paths):
            thumbnail = Thumbnail(image_path, i, callback, self)
            self.thumbnails.append(thumbnail)

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

    def __init__(
        self,
        image_paths: PointedList[Path],
        click_callback: Callable,
        parent=None,
    ):

        super().__init__(parent)

        self.masonry_wall = MasonryWall(image_paths, click_callback)
        self.setWidget(self.masonry_wall)
        self.setWidgetResizable(True)
        self.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAsNeeded)
