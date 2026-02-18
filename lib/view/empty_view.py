from PySide6.QtWidgets import QWidget, QVBoxLayout, QLabel
from PySide6.QtGui import QPen, QPainter, QColor
from PySide6.QtCore import Qt


class EmptyLabel(QLabel):

    def paintEvent(self, event):

        super().paintEvent(event)

        painter = QPainter(self)
        pen = QPen(QColor(170, 170, 170))
        pen.setWidth(2)
        pen.setStyle(Qt.PenStyle.DashLine)
        painter.setPen(pen)

        offset: int = pen.width() // 2
        rect = self.rect().adjusted(offset, offset, -offset, -offset)

        painter.drawRect(rect)


class EmptyView(QWidget):

    MIN_IMAGE_WIDTH: int = 400
    MIN_IMAGE_HEIGHT: int = 400

    def __init__(self, parent=None):

        super().__init__(parent)

        self.setAutoFillBackground(True)

        layout = QVBoxLayout(self)

        self.empty_label = EmptyLabel("No image loaded\nPress ? for help!")
        self.empty_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.empty_label.setMinimumSize(400, 400)
        layout.addWidget(self.empty_label)

    def commands(self):
        return []
