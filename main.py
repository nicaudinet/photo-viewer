import sys
from pathlib import Path
from PIL import Image

from PySide6.QtWidgets import (
    QGridLayout,
    QDialog,
    QApplication,
    QMainWindow,
    QWidget,
    QVBoxLayout,
    QPushButton,
    QLabel,
    QFileDialog,
)
from PySide6.QtGui import QPixmap, QFont
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


class HelpDialog(QDialog):

    def __init__(self, parent=None):

        super().__init__(parent)

        self.setWindowTitle("Keyboard Shortcuts")
        self.setModal(True)
        self.setMinimumWidth(400)
        self.setWindowFlags(
            Qt.WindowType.FramelessWindowHint | Qt.WindowType.Dialog
        )

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
            ("R", "Rotate image 90° clockwise"),
            ("F", "Toggle fullscreen"),
            ("?", "Show this help"),
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
            # desc_label.setStyleSheet("padding: 5px;")
            grid.addWidget(desc_label, i, 1)

        layout.addLayout(grid)


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

        if event.key() == Qt.Key.Key_Q:
            self.close()

        if event.key() == Qt.Key.Key_F:
            self.toggle_fullscreen()

        if event.key() == Qt.Key.Key_Question:
            self.show_help()

        if event.key() == Qt.Key.Key_O:
            self.action_open_button()

        if self.image_paths:
            if event.key() == Qt.Key.Key_Left:
                self.image_paths.prev()
            if event.key() == Qt.Key.Key_Right:
                self.image_paths.next()
            if event.key() == Qt.Key.Key_R:
                self.rotate_image(self.image_paths.current())
            self.open_photo(self.image_paths.current())

    def toggle_fullscreen(self):
        if self.isFullScreen():
            self.showNormal()
        else:
            self.showFullScreen()

    def show_help(self):
        dialog = HelpDialog(self)
        dialog.exec()

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
            if len(image_paths) == 0:
                raise ValueError("Directory does not contain any image files")
            else:
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

    def rotate_image(self, file_path: Path):
        try:
            image = Image.open(file_path)
            image = image.rotate(-90, expand=True)
            image.save(file_path)
        except Exception as e:
            print(f"Error rotating image: {e}")


if __name__ == "__main__":
    app = QApplication(sys.argv)
    viewer = PhotoViewer()
    viewer.show()
    sys.exit(app.exec())
