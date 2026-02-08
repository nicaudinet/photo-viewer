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
from PySide6.QtCore import Qt

from lib.command import Command, NoModifier

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

        # This needs to come from the parent class so we have to pass it in as
        # an argument
        self.swap_to_wall_view = swap_to_wall_view

        ###########
        # Widgets #
        ###########

        layout = QVBoxLayout(self)

        self.current_photo: LargePhoto = self.open_image()
        layout.addWidget(self.current_photo)

    ############
    # Commands #
    ############

    def commands(self):

        return [
            Command(
                key=Qt.Key.Key_Left,
                modifiers=NoModifier,
                description="Previous image",
                action=self.action_prev,
            ),
            Command(
                key=Qt.Key.Key_Right,
                modifiers=NoModifier,
                description="Next image",
                action=self.action_next,
            ),
            Command(
                key=Qt.Key.Key_R,
                modifiers=NoModifier,
                description="Rotate image 90°",
                action=self.action_rotate,
            ),
            Command(
                key=Qt.Key.Key_F,
                modifiers=NoModifier,
                description="Mark as favourite (toggle)",
                action=self.action_favourite,
            ),
            Command(
                key=Qt.Key.Key_F,
                modifiers=Qt.KeyboardModifier.ControlModifier,
                description="Save favourites",
                action=self.action_favourite_save,
            ),
            Command(
                key=Qt.Key.Key_D,
                modifiers=NoModifier,
                description="Mark to delete (toggle)",
                action=self.action_delete,
            ),
            Command(
                key=Qt.Key.Key_D,
                modifiers=Qt.KeyboardModifier.ControlModifier,
                description="Delete all marked",
                action=self.action_delete_all,
            ),
            Command(
                key=Qt.Key.Key_W,
                modifiers=NoModifier,
                description="Wall view",
                action=lambda: self.swap_to_wall_view(self.state),
            ),
        ]

    #####################
    # Action Functions  #
    #####################

    def action_prev(self):
        self.state.prev()
        self.replace_image()

    def action_next(self):
        self.state.next()
        self.replace_image()

    def action_rotate(self):
        self.rotate_image()
        self.replace_image()

    def action_favourite(self):
        image_path = self.state.current()
        if image_path in self.state.favourites:
            self.state.unfavourite(image_path)
            self.state.undelete(image_path)
        else:
            self.state.favourite(image_path)
        self.replace_image()

    def action_delete(self):
        image_path = self.state.current()
        if image_path in self.state.to_delete:
            self.state.undelete(image_path)
        else:
            if not image_path in self.state.favourites:
                self.state.delete(image_path)
        self.replace_image()

    def action_delete_all(self):
        count = len(self.state.to_delete)
        dialog = DeleteConfirmDialog(count, self)
        result = dialog.exec()
        if result == QDialog.DialogCode.Accepted:
            self.state.delete_all()
            self.replace_image()

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

    def open_image(self) -> LargePhoto:
        image_path = self.state.current()
        return LargePhoto(
            image_path=image_path,
            is_favourite=image_path in self.state.favourites,
            to_delete=image_path in self.state.to_delete,
            parent=self,
        )

    def replace_image(self) -> None:
        old_photo = self.current_photo
        self.current_photo = self.open_image()
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
