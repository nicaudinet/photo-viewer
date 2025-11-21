import sys
from pathlib import Path

from PySide6.QtWidgets import (
    QApplication,
    QMainWindow,
    QWidget,
    QVBoxLayout,
    QPushButton,
    QLabel,
    QFileDialog,
)
from PySide6.QtGui import QPixmap
from PySide6.QtCore import Qt


class PointedList:
    """
    A circular list with a highlighted item and next/prev functions
    """

    def __init__(self, list):
        assert len(list) > 0, "The list in a PointedList cannot be empty"
        self.list = list
        self.index = 0

    def current(self):
        return self.list[self.index]

    def next(self):
        self.index = (self.index + 1) % len(self.list)
        return self.current()

    def prev(self):
        self.index = (self.index - 1) % len(self.list)
        return self.current()


class PhotoViewer(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("Photo Viewer")
        self.setGeometry(100, 100, 800, 600)

        central_widget = QWidget()
        self.setCentralWidget(central_widget)
        layout = QVBoxLayout(central_widget)

        self.open_button = QPushButton("Open Photo")
        self.open_button.setDefault(True)
        self.open_button.clicked.connect(self.action_open_button)
        layout.addWidget(self.open_button)

        self.image_label = QLabel("No image loaded")
        self.image_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.image_label.setStyleSheet("border: 2px dashed #aaa;")
        self.image_label.setMinimumSize(400, 400)
        layout.addWidget(self.image_label)

        self.image_paths = None

    def action_open_button(self):
        self.choose_directory()
        if self.image_paths:
            self.open_button.hide()
            self.image_label.setStyleSheet("")
            self.open_photo(self.image_paths.current())

    def keyPressEvent(self, event):
        if event.key() == Qt.Key.Key_Escape:
            print("Escape was pressed")
        elif event.key() == Qt.Key.Key_Q:
            self.close()

    def choose_directory(self):
        directory = QFileDialog.getExistingDirectory(
            parent=self,
            caption="Open Image Directory",
        )
        if directory:
            exts = (".png", ".jpg", ".jpeg")
            all_files = Path(directory).iterdir()
            image_paths = [f for f in all_files if f.suffix in exts]
            image_paths = sorted(image_paths)
            self.image_paths = PointedList(image_paths)

        else:
            assert ValueError(f"Directory {directory}")

    def open_photo(self, file_path: Path):
        pixmap = QPixmap(file_path)
        scaled_pixmap = pixmap.scaled(
            self.image_label.size(),
            Qt.AspectRatioMode.KeepAspectRatio,
            Qt.TransformationMode.SmoothTransformation,
        )
        self.image_label.setPixmap(scaled_pixmap)


if __name__ == "__main__":
    app = QApplication(sys.argv)
    viewer = PhotoViewer()
    viewer.show()
    sys.exit(app.exec())
