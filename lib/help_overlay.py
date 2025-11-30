from PySide6.QtWidgets import (
    QGridLayout,
    QWidget,
    QVBoxLayout,
    QLabel,
)
from PySide6.QtGui import QFont
from PySide6.QtCore import Qt


class HelpOverlay(QWidget):

    def __init__(self, parent=None):

        super().__init__(parent)

        self.setMinimumWidth(300)
        self.setAutoFillBackground(True)

        layout = QVBoxLayout(self)
        layout.setSpacing(15)
        layout.setContentsMargins(25, 25, 25, 25)

        title = QLabel("Keyboard Shortcuts")
        title_font = QFont()
        title_font.setPointSize(20)
        title_font.setBold(True)
        title.setFont(title_font)
        title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.addWidget(title)

        grid = QGridLayout()
        grid.setSpacing(10)
        grid.setColumnStretch(0, 1)
        grid.setColumnStretch(1, 2)

        shortcuts = [
            ("O", "Open directory"),
            ("←", "Previous image"),
            ("→", "Next image"),
            ("W", "Wall view (toggle)"),
            ("R", "Rotate image 90° anti-clockwise"),
            ("L", "Mark as favourite (toggle)"),
            ("D", "Mark to delete (toggle)"),
            ("F", "Fullscreen (toggle)"),
            ("?", "Show this help (toggle)"),
            ("Q", "Quit application"),
        ]

        for i, (key, description) in enumerate(shortcuts):

            key_label = QLabel(key)
            key_label_font = QFont()
            key_label_font.setPointSize(16)
            key_label_font.setBold(True)
            key_label.setFont(key_label_font)
            key_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
            grid.addWidget(key_label, i, 0)

            desc_label = QLabel(description)
            desc_label_font = QFont()
            desc_label_font.setPointSize(16)
            desc_label.setFont(desc_label_font)
            grid.addWidget(desc_label, i, 1)

        layout.addLayout(grid)
