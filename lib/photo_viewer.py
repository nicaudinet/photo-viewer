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
            self.swap_to_single_view(PointedList(image_paths))

    ######################
    # Function Overloads #
    ######################

    def resizeEvent(self, event):
        super().resizeEvent(event)
        x = self.centralWidget().width() // 2 - self.help_overlay.width() // 2
        y = self.centralWidget().height() // 2 - self.help_overlay.height() // 2
        self.help_overlay.move(x, y)

    ####################
    # Helper Functions #
    ####################

    def swap_view(self, new_view: QWidget):
        old_view = self.takeCentralWidget()
        self.setCentralWidget(new_view)
        old_view.deleteLater()
        self.help_overlay.hide()
        self.help_overlay.raise_()

    def swap_to_wall_view(self, image_paths: PointedList[Path]):
        wall_view = WallView(image_paths, self.swap_to_single_view)
        self.swap_view(wall_view)

    def swap_to_single_view(self, image_paths: PointedList[Path]):
        single_view = SingleView(image_paths, self.swap_to_wall_view)
        self.swap_view(single_view)

    def choose_directory(self) -> Optional[List[Path]]:
        directory = QFileDialog.getExistingDirectory(
            parent=self,
            caption="Open Image Directory",
            dir="/Users/audinet/Pictures/Camera/2025 China/Favourites",
        )
        if directory:
            exts = (".png", ".jpg", ".jpeg")
            all_files = Path(directory).iterdir()
            image_paths = [f for f in all_files if f.suffix.lower() in exts]
            image_paths = sorted(image_paths)
            if len(image_paths) != 0:
                return image_paths
        return None
