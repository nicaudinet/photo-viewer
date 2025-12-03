from typing import Callable, List

from PySide6.QtWidgets import QWidget, QScrollArea
from PySide6.QtGui import QShortcut, QResizeEvent, QKeySequence
from PySide6.QtCore import Qt, QThreadPool

from lib.state import ImageState
from lib.photo import Thumbnail


class MasonryWall(QWidget):

    SPACING: int = 20

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
        self.threadpool = QThreadPool.globalInstance()
        self.show_only_favourites = False
        self.show_only_to_delete = False

        ########
        # Init #
        ########

        def click_callback(index: int) -> None:
            self.state.goto(index)
            return swap_to_single_view(self.state)

        for i, image_path in enumerate(self.state.image_paths):
            thumbnail = Thumbnail(
                image_path=image_path,
                is_favourite=image_path in self.state.favourites,
                to_delete=image_path in self.state.to_delete,
                index=i,
                click_callback=click_callback,
                parent=self,
            )
            self.thumbnails.append(thumbnail)
            thumbnail.make_thumbnail_async(self.threadpool)

    ######################
    # Function Overloads #
    ######################

    def resizeEvent(self, event: QResizeEvent):
        super().resizeEvent(event)
        min_height = self.build_wall()
        self.setMinimumHeight(min_height)

    ####################
    # Helper Functions #
    ####################

    def build_wall(self) -> int:

        for thumbnail in self.thumbnails:
            if thumbnail.is_favourite:
                thumbnail.show_favourite()
            if thumbnail.to_delete:
                thumbnail.show_to_delete()
            thumbnail.hide()

        thumbnails = self.thumbnails
        if self.show_only_favourites:
            thumbnails = [t for t in thumbnails if t.is_favourite]
            for thumbnail in thumbnails:
                thumbnail.hide_favourite()
        elif self.show_only_to_delete:
            thumbnails = [t for t in thumbnails if t.to_delete]
            for thumbnail in thumbnails:
                thumbnail.hide_to_delete()

        column_width = Thumbnail.THUMBNAIL_WIDTH
        item_width = self.SPACING + column_width
        column_count = (self.width() - self.SPACING) // item_width
        column_heights = [self.SPACING] * column_count
        left_padding = (self.width() - column_count * item_width) // 2

        for thumbnail in thumbnails:
            shortest_column = column_heights.index(min(column_heights))
            x = left_padding + self.SPACING + shortest_column * item_width
            y = column_heights[shortest_column]
            thumbnail.move(x, y)
            column_heights[shortest_column] += thumbnail.height() + self.SPACING

        for thumbnail in thumbnails:
            thumbnail.show()

        return max(column_heights)

    def toggle_only_favourites(self):
        self.show_only_favourites = not self.show_only_favourites
        self.build_wall()

    def toggle_only_to_delete(self):
        self.show_only_to_delete = not self.show_only_to_delete
        self.build_wall()


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

        QShortcut(Qt.Key.Key_W, self, lambda: swap_to_single_view(state))
        QShortcut(QKeySequence("Shift+F"), self, self.toggle_only_favourites)
        QShortcut(QKeySequence("Shift+D"), self, self.toggle_only_to_delete)

    def toggle_only_favourites(self):
        self.masonry_wall.toggle_only_favourites()

    def toggle_only_to_delete(self):
        self.masonry_wall.toggle_only_to_delete()
