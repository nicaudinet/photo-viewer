from pathlib import Path
from PIL import Image

from PySide6.QtWidgets import (
    QMainWindow,
    QWidget,
    QVBoxLayout,
    QPushButton,
    QLabel,
    QFileDialog,
)
from PySide6.QtGui import QPixmap, QIcon
from PySide6.QtCore import Qt

from lib.pointed_list import PointedList
from lib.help_overlay import HelpOverlay
from lib.view.wall_view import WallView


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

        self.wall_view = WallView()
        self.wall_view.setClickCallback(self.on_thumbnail_clicked)
        self.wall_view.hide()
        layout.addWidget(self.wall_view)

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
            self.open_photo()

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
                self.open_photo()

            if event.key() == Qt.Key.Key_Right:
                self.image_paths.next()
                self.open_photo()

            if event.key() == Qt.Key.Key_R:
                self.rotate_image(self.image_paths.current())
                self.open_photo()

            if event.key() == Qt.Key.Key_L:
                current = self.image_paths.current()
                if current in self.favourites:
                    self.favourites.remove(current)
                    self.star_label.hide()
                else:
                    self.favourites.add(current)
                    self.star_label.show()
                self.open_photo()

            if event.key() == Qt.Key.Key_D:
                current = self.image_paths.current()
                if current in self.to_delete:
                    self.to_delete.remove(current)
                    self.delete_label.hide()
                else:
                    if not current in self.favourites:
                        self.to_delete.add(current)
                        self.delete_label.show()
                self.open_photo()

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
            self.open_photo()

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
            dir="/Users/audinet/Pictures/Camera/China/Favourites",
        )
        if directory:
            exts = (".png", ".jpg", ".jpeg")
            all_files = Path(directory).iterdir()
            image_paths = [f for f in all_files if f.suffix.lower() in exts]
            image_paths = sorted(image_paths)
            if len(image_paths) == 0:
                raise ValueError("Directory does not contain any image files")
            else:
                self.image_paths = PointedList(image_paths)

        else:
            assert ValueError(f"Directory {directory}")

    def open_photo(self):
        if self.image_paths:
            current = self.image_paths.current()
            pixmap = QPixmap(current)
            pixmap = pixmap.scaled(
                self.image_label.size(),
                Qt.AspectRatioMode.KeepAspectRatio,
                Qt.TransformationMode.SmoothTransformation,
            )
            if current in self.favourites:
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
            self.wall_view.hide()
            self.image_label.show()
            self.in_wall_view = False
            self.open_photo()
            self.setFocus()

    def switch_to_wall_view(self):
        if self.image_paths:
            self.image_label.hide()
            self.star_label.hide()
            self.delete_label.hide()
            self.help_overlay.hide()

            self.wall_view.setImages(self.image_paths.list)
            self.wall_view.show()
            self.in_wall_view = True
