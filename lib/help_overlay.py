from typing import List

from PySide6.QtWidgets import (
    QGridLayout,
    QWidget,
    QVBoxLayout,
    QLabel,
)
from PySide6.QtGui import QFont
from PySide6.QtCore import Qt

from lib.command import Command, key_display_name


class HelpOverlay(QWidget):

    def __init__(self, commands: List[Command], parent):

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

        for i, cmd in enumerate(commands):

            key_label = QLabel(key_display_name(cmd))
            key_label_font = QFont()
            key_label_font.setPointSize(16)
            key_label_font.setBold(True)
            key_label.setFont(key_label_font)
            key_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
            grid.addWidget(key_label, i, 0)

            desc_label = QLabel(cmd.description)
            desc_label_font = QFont()
            desc_label_font.setPointSize(16)
            desc_label.setFont(desc_label_font)
            grid.addWidget(desc_label, i, 1)

        layout.addLayout(grid)

        self.resize_and_center()

    def resize_and_center(self):
        self.adjustSize()
        parent = self.parentWidget()

        parent_width = parent.width()  # pyright: ignore
        parent_height = parent.height()  # pyright: ignore

        x = parent_width // 2 - self.width() // 2
        y = parent_height // 2 - self.height() // 2
        self.move(x, y)
