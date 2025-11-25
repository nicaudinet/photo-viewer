from typing import Optional, List
from pathlib import Path

from PySide6.QtWidgets import (
    QMainWindow,
    QWidget,
    QFileDialog,
)
from PySide6.QtGui import QShortcut
from PySide6.QtCore import Qt

from lib.pointed_list import PointedList
from lib.help_overlay import HelpOverlay
from lib.view.wall_view import WallView
from lib.view.single_view import SingleView
from lib.view.empty_view import EmptyView


class PhotoViewer(QMainWindow):

    def __init__(self):

        super().__init__()

        self.setWindowTitle("Photo Viewer")
        self.setGeometry(100, 100, 800, 600)

        #############
        # Shortcuts #
        #############

        QShortcut(Qt.Key.Key_Question, self, self.action_toggle_help)
        QShortcut(Qt.Key.Key_Q, self, self.action_quit)
        QShortcut(Qt.Key.Key_F, self, self.action_fullscreen)
        QShortcut(Qt.Key.Key_O, self, self.action_open_directory)
        QShortcut(Qt.Key.Key_W, self, self.action_toggle_wall)

        ###########
        # Widgets #
        ###########

        self.help_overlay = HelpOverlay(self)
        self.help_overlay.adjustSize()  # Has 0 size otherwise
        self.help_overlay.hide()
        self.help_overlay.raise_()

        self.setCentralWidget(EmptyView(self))

    ###########
    # Actions #
    ###########

    def action_quit(self):
        self.close()

    def action_fullscreen(self):
        if self.isFullScreen():
            self.showNormal()
        else:
            self.showFullScreen()

    def action_toggle_help(self):
        if self.help_overlay.isVisible():
            self.help_overlay.hide()
        else:
            self.help_overlay.show()

    def action_open_directory(self):
        image_paths = self.choose_directory()
        if image_paths:
            self.swap_view(SingleView(PointedList(image_paths)))

    def action_toggle_wall(self):
        view = self.centralWidget()
        if hasattr(view, "image_paths"):
            image_paths = view.image_paths  # pyright: ignore
            if isinstance(view, SingleView):
                new_view = WallView(image_paths, self.click_callback)
            elif isinstance(view, WallView):
                new_view = SingleView(image_paths)
            else:
                return
            self.swap_view(new_view)

    def swap_view(self, new_view: QWidget):
        old_view = self.takeCentralWidget()
        self.setCentralWidget(new_view)
        old_view.deleteLater()
        self.help_overlay.hide()
        self.help_overlay.raise_()

    def choose_directory(self) -> Optional[List[Path]]:
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
            if len(image_paths) != 0:
                return image_paths
        return None

    ######################
    # Function Overloads #
    ######################

    def resizeEvent(self, event):
        super().resizeEvent(event)
        x = self.centralWidget().width() // 2 - self.help_overlay.width() // 2
        y = self.centralWidget().height() // 2 - self.help_overlay.height() // 2
        self.help_overlay.move(x, y)

    def click_callback(self, image_paths: PointedList[Path]):
        self.swap_view(SingleView(image_paths))

    # def keyPressEvent(self, event):
    #
    #     if self.image_paths:
    #
    #         if event.key() == Qt.Key.Key_Left:
    #             self.image_paths.prev()
    #             self.open_photo()
    #
    #         if event.key() == Qt.Key.Key_Right:
    #             self.image_paths.next()
    #             self.open_photo()
    #
    #         if event.key() == Qt.Key.Key_R:
    #             self.rotate_image(self.image_paths.current())
    #             self.open_photo()
    #
    #         if event.key() == Qt.Key.Key_L:
    #             current = self.image_paths.current()
    #             if current in self.favourites:
    #                 self.favourites.remove(current)
    #                 self.star_label.hide()
    #             else:
    #                 self.favourites.add(current)
    #                 self.star_label.show()
    #             self.open_photo()
    #
    #         if event.key() == Qt.Key.Key_D:
    #             current = self.image_paths.current()
    #             if current in self.to_delete:
    #                 self.to_delete.remove(current)
    #                 self.delete_label.hide()
    #             else:
    #                 if not current in self.favourites:
    #                     self.to_delete.add(current)
    #                     self.delete_label.show()
    #             self.open_photo()
    #
    #         if event.key() == Qt.Key.Key_W:
    #             if self.in_wall_view:
    #                 self.switch_to_single_view()
    #             else:
    #                 self.switch_to_wall_view()
    #
    # def open_photo(self):
    #     if self.image_paths:
    #         current = self.image_paths.current()
    #         pixmap = QPixmap(current)
    #         pixmap = pixmap.scaled(
    #             self.image_label.size(),
    #             Qt.AspectRatioMode.KeepAspectRatio,
    #             Qt.TransformationMode.SmoothTransformation,
    #         )
    #         if current in self.favourites:
    #             self.star_label.show()
    #         else:
    #             self.star_label.hide()
    #         self.image_label.setPixmap(pixmap)
    #
    # def rotate_image(self, file_path: Path):
    #     try:
    #         image = Image.open(file_path)
    #         image = image.rotate(-90, expand=True)
    #         image.save(file_path)
    #     except Exception as e:
    #         print(f"Error rotating image: {e}")
    #
    # def switch_to_single_view(self):
    #     if self.image_paths:
    #         self.wall_view.hide()
    #         self.image_label.show()
    #         self.in_wall_view = False
    #         self.open_photo()
    #         self.setFocus()
    #
    # def switch_to_wall_view(self):
    #     if self.image_paths:
    #         self.image_label.hide()
    #         self.star_label.hide()
    #         self.delete_label.hide()
    #         self.help_overlay.hide()
    #
    #         self.wall_view.setImages(self.image_paths.list)
    #         self.wall_view.show()
    #         self.in_wall_view = True
