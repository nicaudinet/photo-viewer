from typing import Callable, List
from pathlib import Path

from PySide6.QtWidgets import QWidget, QLabel, QScrollArea
from PySide6.QtGui import QPixmap, QMouseEvent, QShortcut, QResizeEvent
from PySide6.QtCore import Qt

from lib.state import ImageState


class Thumbnail(QLabel):

    THUMBNAIL_WIDTH = 300

    def __init__(
        self,
        file_path: Path,
        index: int,
        click_callback: Callable[[int], None],
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

    SPACING: int = 10

    def __init__(
        self,
        state: ImageState,
        swap_to_single_view: Callable[[ImageState], None],
        parent=None,
    ):

        super().__init__(parent)

        #########
        # State #
        #########

        self.thumbnails: List[Thumbnail] = []
        self.state = state

        ########
        # Init #
        ########

        def click_callback(index: int) -> None:
            self.state.goto(index)
            return swap_to_single_view(self.state)

        for i, image_path in enumerate(self.state.image_paths):
            thumbnail = Thumbnail(image_path, i, click_callback, self)
            self.thumbnails.append(thumbnail)

    ######################
    # Function Overloads #
    ######################

    def resizeEvent(self, event: QResizeEvent):
        super().resizeEvent(event)

        column_width = Thumbnail.THUMBNAIL_WIDTH
        item_width = self.SPACING + column_width
        column_count = (self.width() - self.SPACING) // item_width
        column_heights = [self.SPACING] * column_count
        left_padding = (self.width() - column_count * item_width) // 2

        for thumbnail in self.thumbnails:
            shortest_column = column_heights.index(min(column_heights))
            x = left_padding + self.SPACING + shortest_column * item_width
            y = column_heights[shortest_column]
            thumbnail.move(x, y)
            column_heights[shortest_column] += thumbnail.height() + self.SPACING

        for thumbnail in self.thumbnails:
            thumbnail.show()

        self.setMinimumHeight(max(column_heights))


class WallView(QScrollArea):

    def __init__(
        self,
        state: ImageState,
        swap_to_single_view: Callable[[ImageState], None],
        parent=None,
    ):

        super().__init__(parent)

        ###########
        # Widgets #
        ###########

        self.masonry_wall = MasonryWall(state, swap_to_single_view)
        self.setWidget(self.masonry_wall)
        self.setWidgetResizable(True)
        self.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAsNeeded)

        #############
        # Shortcuts #
        #############

        QShortcut(
            Qt.Key.Key_W,
            self,
            lambda: swap_to_single_view(state),
        )
