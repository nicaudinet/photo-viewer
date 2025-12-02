from pathlib import Path
from typing import Callable
from PIL import Image

from PySide6.QtWidgets import (
    QWidget,
    QVBoxLayout,
    QHBoxLayout,
    QDialog,
    QPushButton,
    QLabel,
    QFileDialog,
)
from PySide6.QtGui import QShortcut, QKeySequence
from PySide6.QtCore import Qt

from lib.state import ImageState
from lib.photo import LargePhoto


class DeleteConfirmDialog(QDialog):

    def __init__(self, count: int, parent=None):

        super().__init__(parent)

        self.setWindowTitle("Confirm Deletion")
        self.setModal(True)
        window_flag = Qt.WindowType.Dialog | Qt.WindowType.FramelessWindowHint
        self.setWindowFlags(window_flag)

        layout = QVBoxLayout()
        layout.setContentsMargins(20, 20, 20, 20)

        if count == 1:
            message = "Are you sure you want to delete 1 photo?"
        else:
            message = f"Are you sure you want to delete {count} photos?"
        message = QLabel(message)
        message.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.addWidget(message)

        button_layout = QHBoxLayout()

        yes_button = QPushButton("Yes")
        yes_button.clicked.connect(self.accept)
        button_layout.addWidget(yes_button)

        no_button = QPushButton("No")
        no_button.clicked.connect(self.reject)
        no_button.setFocus()
        button_layout.addWidget(no_button)

        layout.addLayout(button_layout)

        self.setLayout(layout)


class SingleView(QWidget):

    def __init__(
        self,
        init_state: ImageState,
        swap_to_wall_view: Callable[[ImageState], None],
        parent=None,
    ):

        super().__init__(parent)

        #########
        # State #
        #########

        self.state: ImageState = init_state

        ###########
        # Widgets #
        ###########

        layout = QVBoxLayout(self)

        self.current_photo: LargePhoto = self.open_photo()
        layout.addWidget(self.current_photo)

        #############
        # Shortcuts #
        #############

        QShortcut(Qt.Key.Key_Left, self, self.action_prev)
        QShortcut(Qt.Key.Key_Right, self, self.action_next)
        QShortcut(Qt.Key.Key_R, self, self.action_rotate)
        QShortcut(QKeySequence("F"), self, self.action_favourite)
        QShortcut(QKeySequence("Ctrl+F"), self, self.action_favourite_save)
        QShortcut(QKeySequence("D"), self, self.action_delete)
        QShortcut(QKeySequence("Ctrl+D"), self, self.action_delete_all)
        QShortcut(Qt.Key.Key_W, self, lambda: swap_to_wall_view(self.state))

    #####################
    # Action Functions  #
    #####################

    def action_prev(self):
        self.state.prev()
        self.replace_photo()

    def action_next(self):
        self.state.next()
        self.replace_photo()

    def action_rotate(self):
        self.rotate_image()
        self.replace_photo()

    def action_favourite(self):
        image_path = self.state.current()
        if image_path in self.state.favourites:
            self.state.unfavourite(image_path)
            self.state.undelete(image_path)
        else:
            self.state.favourite(image_path)
        self.replace_photo()

    def action_delete(self):
        image_path = self.state.current()
        if image_path in self.state.to_delete:
            self.state.undelete(image_path)
        else:
            if not image_path in self.state.favourites:
                self.state.delete(image_path)
        self.replace_photo()

    def action_delete_all(self):
        count = len(self.state.to_delete)
        dialog = DeleteConfirmDialog(count, self)
        result = dialog.exec()
        if result == QDialog.DialogCode.Accepted:
            self.state.delete_all()
            self.replace_photo()

    def action_favourite_save(self):
        favourites_dir = QFileDialog.getExistingDirectory(
            parent=self,
            caption="Create or select directory to save favourites",
            dir="/Users/audinet/Pictures/Camera/2025 China/Favourites",
        )
        self.state.save_favourites(Path(favourites_dir))

    ####################
    # Helper Functions #
    ####################

    def open_photo(self) -> LargePhoto:
        image_path = self.state.current()
        return LargePhoto(
            image_path=image_path,
            is_favourite=image_path in self.state.favourites,
            to_delete=image_path in self.state.to_delete,
            parent=self,
        )

    def replace_photo(self) -> None:
        old_photo = self.current_photo
        self.current_photo = self.open_photo()
        layout = self.layout()
        if layout:
            layout.replaceWidget(old_photo, self.current_photo)
        old_photo.deleteLater()

    def rotate_image(self):
        try:
            image_path = self.state.current()
            image = Image.open(image_path)
            image = image.rotate(90, expand=True)
            image.save(image_path)
        except Exception as e:
            print(f"Error rotating image: {e}")
