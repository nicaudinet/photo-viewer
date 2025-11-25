from typing import Set
from PIL import Image
from pathlib import Path

from PySide6.QtWidgets import QWidget, QVBoxLayout, QLabel
from PySide6.QtGui import QPixmap, QIcon, QShortcut, QResizeEvent
from PySide6.QtCore import Qt

from lib.pointed_list import PointedList


class SingleView(QWidget):

    ICON_SIZE: int = 50
    ICON_MARGIN: int = 20
    MIN_IMAGE_WIDTH: int = 400
    MIN_IMAGE_HEIGHT: int = 400

    def __init__(self, image_paths: PointedList[Path], parent=None):

        super().__init__(parent)

        #########
        # State #
        #########

        self.image_paths: PointedList[Path] = image_paths
        self.favourites: Set[Path] = set()
        self.to_delete: Set[Path] = set()

        ###########
        # Widgets #
        ###########

        layout = QVBoxLayout(self)

        self.image_label = QLabel("No image loaded")
        self.image_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.image_label.setMinimumWidth(self.MIN_IMAGE_WIDTH)
        self.image_label.setMinimumHeight(self.MIN_IMAGE_HEIGHT)
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

        #############
        # Shortcuts #
        #############

        QShortcut(Qt.Key.Key_Left, self, self.action_prev)
        QShortcut(Qt.Key.Key_Right, self, self.action_next)
        QShortcut(Qt.Key.Key_R, self, self.action_rotate)
        QShortcut(Qt.Key.Key_L, self, self.action_like)
        QShortcut(Qt.Key.Key_D, self, self.action_delete)

    #####################
    # Action Functions  #
    #####################

    def action_prev(self):
        self.image_paths.prev()
        self.open_photo()

    def action_next(self):
        self.image_paths.next()
        self.open_photo()

    def action_rotate(self):
        self.rotate_image()
        self.open_photo()

    def action_like(self):
        current = self.image_paths.current()
        if current in self.favourites:
            self.favourites.remove(current)
            self.star_label.hide()
        else:
            self.favourites.add(current)
            self.star_label.show()
        self.open_photo()

    def action_delete(self):
        current = self.image_paths.current()
        if current in self.to_delete:
            self.to_delete.remove(current)
            self.delete_label.hide()
        else:
            if not current in self.favourites:
                self.to_delete.add(current)
                self.delete_label.show()
        self.open_photo()

    ######################
    # Function Overloads #
    ######################

    def resizeEvent(self, event: QResizeEvent):
        super().resizeEvent(event)
        self.open_photo()
        x = self.width() - self.ICON_MARGIN - self.ICON_SIZE
        y = self.ICON_MARGIN
        self.star_label.move(x, y)
        self.delete_label.move(x, y)

    ####################
    # Helper Functions #
    ####################

    def open_photo(self):
        if self.image_paths:
            current = self.image_paths.current()
            pixmap = QPixmap(current)
            pixmap = pixmap.scaled(
                self.image_label.size(),
                Qt.AspectRatioMode.KeepAspectRatio,
                Qt.TransformationMode.SmoothTransformation,
            )
            self.image_label.setPixmap(pixmap)
            if current in self.favourites:
                self.star_label.show()
            elif current in self.to_delete:
                self.delete_label.show()
            else:
                self.star_label.hide()
                self.delete_label.hide()

    def rotate_image(self):
        try:
            current = self.image_paths.current()
            image = Image.open(current)
            image = image.rotate(-90, expand=True)
            image.save(current)
        except Exception as e:
            print(f"Error rotating image: {e}")
