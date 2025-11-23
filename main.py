from typing import Optional, Callable, List
import sys
from pathlib import Path
from PIL import Image

from PySide6.QtWidgets import (
    QGridLayout,
    QApplication,
    QMainWindow,
    QWidget,
    QVBoxLayout,
    QPushButton,
    QLabel,
    QFileDialog,
    QScrollArea,
)
from PySide6.QtGui import QPixmap, QFont, QIcon, QMouseEvent
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

    def goto(self, index: int):
        assert 0 <= index < len(self.list), "Index is not in the list"
        self.index = index


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
            ("R", "Rotate image 90° clockwise"),
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


class Thumbnail(QLabel):

    THUMBNAIL_WIDTH = 300

    def __init__(self, file_path: Path, index: int, parent=None):

        super().__init__(parent)

        self.index: int = index
        self.click_callback: Optional[Callable] = None

        pixmap = QPixmap(file_path)
        pixmap = pixmap.scaledToWidth(
            self.THUMBNAIL_WIDTH,
            Qt.TransformationMode.SmoothTransformation,
        )
        self.setPixmap(pixmap)
        self.setFixedSize(pixmap.size())

        self.setCursor(Qt.CursorShape.PointingHandCursor)

    def setClickCallback(self, callback: Callable):
        self.click_callback = callback

    def mousePressEvent(self, event: QMouseEvent):
        if event.button() == Qt.MouseButton.LeftButton and self.click_callback:
            self.click_callback(self.index)


class MasonryWall(QWidget):

    def __init__(self, parent=None):

        super().__init__(parent)

        self.thumbnails: List[Thumbnail] = []
        self.image_paths: List[Path] = []

        self.column_count: int = 3
        self.spacing: int = 10
        self.thumbnail_width: int = 300
        self.click_callback: Optional[Callable] = None

    def setClickCallback(self, callback: Callable):
        self.click_callback = callback
        for thumbnail in self.thumbnails:
            thumbnail.setClickCallback(callback)

    def setImages(self, image_paths: List[Path]):

        self.image_paths = image_paths

        for thumbnail in self.thumbnails:
            thumbnail.deleteLater()
        self.thumbnails.clear()

        for i, image_path in enumerate(image_paths):
            thumbnail = Thumbnail(image_path, i, self)
            if self.click_callback:
                thumbnail.setClickCallback(self.click_callback)
            self.thumbnails.append(thumbnail)

        self.layout_masonry()

    def layout_masonry(self):

        if not self.thumbnails:
            return

        column_width = Thumbnail.THUMBNAIL_WIDTH
        column_heights = [self.spacing] * self.column_count

        for thumbnail in self.thumbnails:
            shortest_column = column_heights.index(min(column_heights))
            x = self.spacing + shortest_column * (column_width + self.spacing)
            y = column_heights[shortest_column]
            thumbnail.move(x, y)
            column_heights[shortest_column] += thumbnail.height() + self.spacing

        for thumbnail in self.thumbnails:
            thumbnail.show()

        self.setMinimumHeight(max(column_heights))


class MasonryScrollArea(QScrollArea):

    def __init__(self, parent=None):

        super().__init__(parent)

        self.masonry_wall = MasonryWall()
        self.setWidget(self.masonry_wall)
        self.setWidgetResizable(True)
        self.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAsNeeded)

    def setImages(self, image_paths: List[Path]):
        self.masonry_wall.setImages(image_paths)

    def setClickCallback(self, callback: Callable):
        self.masonry_wall.setClickCallback(callback)


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

        self.masonry_wall = MasonryScrollArea()
        self.masonry_wall.setClickCallback(self.on_thumbnail_clicked)
        self.masonry_wall.hide()
        layout.addWidget(self.masonry_wall)

        self.help_overlay = HelpOverlay(central_widget)
        self.help_overlay.adjustSize()  # Has 0 size otherwise
        self.help_overlay.hide()
        self.help_overlay.raise_()

        #########
        # Icons #
        #########

        self.icon_size: int = 50
        self.icon_margin: int = 20

        self.star_label = QLabel(central_widget)
        star_icon = QIcon("./icons/star.png")
        star_pixmap = star_icon.pixmap(self.icon_size, self.icon_size)
        self.star_label.setPixmap(star_pixmap)
        self.star_label.setFixedSize(self.icon_size, self.icon_size)
        self.star_label.hide()
        self.star_label.raise_()

        self.delete_label = QLabel(central_widget)
        delete_icon = QIcon("./icons/delete.png")
        delete_pixmap = delete_icon.pixmap(self.icon_size, self.icon_size)
        self.delete_label.setPixmap(delete_pixmap)
        self.delete_label.setFixedSize(self.icon_size, self.icon_size)
        self.delete_label.hide()
        self.delete_label.raise_()

        #############
        # App State #
        #############

        self.image_paths = None
        self.favourites = set()
        self.to_delete = set()
        self.in_wall_view = False

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
            if self.isFullScreen():
                self.showNormal()
            else:
                self.showFullScreen()

        if event.key() == Qt.Key.Key_Question:
            if self.help_overlay.isVisible():
                self.help_overlay.hide()
            else:
                self.help_overlay.show()
                # self.help_overlay.raise_()

        if event.key() == Qt.Key.Key_O:
            self.action_open_button()

        if self.image_paths:

            if event.key() == Qt.Key.Key_Left:
                self.image_paths.prev()
                self.open_photo(self.image_paths.current())

            if event.key() == Qt.Key.Key_Right:
                self.image_paths.next()
                self.open_photo(self.image_paths.current())

            if event.key() == Qt.Key.Key_R:
                self.rotate_image(self.image_paths.current())
                self.open_photo(self.image_paths.current())

            if event.key() == Qt.Key.Key_L:
                current = self.image_paths.current()
                if current in self.favourites:
                    self.favourites.remove(current)
                    self.star_label.hide()
                else:
                    self.favourites.add(current)
                    self.star_label.show()
                self.open_photo(current)

            if event.key() == Qt.Key.Key_D:
                current = self.image_paths.current()
                if current in self.to_delete:
                    self.to_delete.remove(current)
                    self.delete_label.hide()
                else:
                    if not current in self.favourites:
                        self.to_delete.add(current)
                        self.delete_label.show()
                self.open_photo(current)

            if event.key() == Qt.Key.Key_W:
                if self.in_wall_view:
                    self.switch_to_single_view()
                else:
                    self.switch_to_wall_view()

    def resizeEvent(self, event):
        """
        Update resizeEvent to automatically move icons to the top-right corner
        """
        super().resizeEvent(event)

        if self.image_paths and not self.in_wall_view:
            self.open_photo(self.image_paths.current())

        x = self.centralWidget().width() // 2 - self.help_overlay.width() // 2
        y = self.centralWidget().height() // 2 - self.help_overlay.height() // 2
        self.help_overlay.move(x, y)

        x = self.centralWidget().width() - self.icon_margin - self.icon_size
        y = self.icon_margin
        self.star_label.move(x, y)
        self.delete_label.move(x, y)

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
        pixmap = pixmap.scaled(
            self.image_label.size(),
            Qt.AspectRatioMode.KeepAspectRatio,
            Qt.TransformationMode.SmoothTransformation,
        )
        if file_path in self.favourites:
            self.star_label.show()
        else:
            self.star_label.hide()
        self.image_label.setPixmap(pixmap)

    def rotate_image(self, file_path: Path):
        try:
            image = Image.open(file_path)
            image = image.rotate(-90, expand=True)
            image.save(file_path)
        except Exception as e:
            print(f"Error rotating image: {e}")

    def on_thumbnail_clicked(self, index: int):
        if self.image_paths:
            self.image_paths.goto(index)
            self.switch_to_single_view()

    def switch_to_single_view(self):
        if self.image_paths:
            self.masonry_wall.hide()
            self.image_label.show()
            self.in_wall_view = False
            self.open_photo(self.image_paths.current())
            self.setFocus()

    def switch_to_wall_view(self):
        if self.image_paths:
            self.image_label.hide()
            self.star_label.hide()
            self.delete_label.hide()
            self.help_overlay.hide()

            self.masonry_wall.setImages(self.image_paths.list)
            self.masonry_wall.show()
            self.in_wall_view = True


if __name__ == "__main__":
    app = QApplication(sys.argv)
    viewer = PhotoViewer()
    viewer.show()
    sys.exit(app.exec())
