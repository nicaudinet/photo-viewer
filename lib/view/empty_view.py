from PySide6.QtWidgets import QWidget, QVBoxLayout, QLabel
from PySide6.QtCore import Qt


class EmptyView(QWidget):

    MIN_IMAGE_WIDTH: int = 400
    MIN_IMAGE_HEIGHT: int = 400

    def __init__(self, parent=None):

        super().__init__(parent)

        layout = QVBoxLayout(self)

        self.empty_image = QLabel("No image loaded\nPress ? for help!")
        self.empty_image.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.empty_image.setStyleSheet("border: 2px dashed #aaa;")
        self.empty_image.setMinimumSize(400, 400)
        layout.addWidget(self.empty_image)
