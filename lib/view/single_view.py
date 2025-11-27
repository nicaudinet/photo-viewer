from typing import Callable
from PIL import Image
from pathlib import Path

from PySide6.QtWidgets import QWidget, QVBoxLayout, QLabel
from PySide6.QtGui import QShortcut, QResizeEvent
from PySide6.QtCore import Qt

from lib.state import ImageState
from lib.pointed_list import PointedList
from lib.photo import LargePhoto


class SingleView(QWidget):

    def __init__(
        self,
        init_state: PointedList[Path] | ImageState,
        swap_to_wall_view: Callable[[ImageState], None],
        parent=None,
    ):

        super().__init__(parent)

        #########
        # State #
        #########

        if isinstance(init_state, ImageState):
            self.state: ImageState = init_state
        else:
            self.state = ImageState(
                image_paths=init_state,
                favourites=set(),
                to_delete=set(),
            )

        ###########
        # Widgets #
        ###########

        layout = QVBoxLayout(self)

        self.current_photo = QLabel()
        layout.addWidget(self.current_photo)

        #############
        # Shortcuts #
        #############

        QShortcut(Qt.Key.Key_Left, self, self.action_prev)
        QShortcut(Qt.Key.Key_Right, self, self.action_next)
        QShortcut(Qt.Key.Key_R, self, self.action_rotate)
        QShortcut(Qt.Key.Key_L, self, self.action_like)
        QShortcut(Qt.Key.Key_D, self, self.action_delete)
        QShortcut(Qt.Key.Key_W, self, lambda: swap_to_wall_view(self.state))

    #####################
    # Action Functions  #
    #####################

    def action_prev(self):
        self.state.prev()
        self.open_photo()

    def action_next(self):
        self.state.next()
        self.open_photo()

    def action_rotate(self):
        self.rotate_image()
        self.open_photo()

    def action_like(self):
        image_path = self.state.current()
        if image_path in self.state.favourites:
            self.state.dislike(image_path)
            self.state.restore(image_path)
        else:
            self.state.like(image_path)
        self.open_photo()

    def action_delete(self):
        image_path = self.state.current()
        if image_path in self.state.to_delete:
            self.state.restore(image_path)
        else:
            if not image_path in self.state.favourites:
                self.state.delete(image_path)
        self.open_photo()

    ######################
    # Function Overloads #
    ######################

    def resizeEvent(self, event: QResizeEvent):
        super().resizeEvent(event)
        self.open_photo()

    ####################
    # Helper Functions #
    ####################

    def open_photo(self):
        old_photo = self.current_photo
        image_path = self.state.current()
        self.current_photo = LargePhoto(
            image_path=image_path,
            is_favourite=image_path in self.state.favourites,
            to_delete=image_path in self.state.to_delete,
            parent=self,
        )
        layout = self.layout()
        if layout:
            layout.replaceWidget(old_photo, self.current_photo)
        old_photo.deleteLater()

    def rotate_image(self):
        try:
            image_path = self.state.current()
            image = Image.open(image_path)
            image = image.rotate(-90, expand=True)
            image.save(image_path)
        except Exception as e:
            print(f"Error rotating image: {e}")
