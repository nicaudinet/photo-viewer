from typing import Optional
from pathlib import Path

from PySide6.QtWidgets import (
    QMainWindow,
    QWidget,
    QFileDialog,
)
from PySide6.QtGui import QKeyEvent
from PySide6.QtCore import Qt

from lib.command import Command, NoModifier, handle_key_event

from lib.state import ImageState, load_image_state
from lib.help_overlay import HelpOverlay
from lib.view.wall_view import WallView
from lib.view.single_view import SingleView
from lib.view.empty_view import EmptyView


class PhotoViewer(QMainWindow):

    def __init__(self, filepath: Optional[Path]):

        super().__init__()

        self.setWindowTitle("Photo Viewer")
        self.setGeometry(100, 100, 800, 600)

        ###########
        # Widgets #
        ###########

        central_widget = EmptyView(self)
        self.setCentralWidget(central_widget)

        self.help_overlay = HelpOverlay(central_widget)
        self.help_overlay.adjustSize()  # Has 0 size otherwise
        self.help_overlay.hide()
        self.help_overlay.raise_()

        if filepath:
            self.load_path(filepath)

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
            self.help_overlay.raise_()

    def action_open_directory(self):
        image_state = self.choose_directory()
        if image_state:
            self.swap_to_single_view(image_state)

    ############
    # Commands #
    ############

    def commands(self):
        return [
            Command(
                key=Qt.Key.Key_Question,
                modifiers=NoModifier,
                description="Show help (toggle)",
                action=self.action_toggle_help,
            ),
            Command(
                key=Qt.Key.Key_Q,
                modifiers=NoModifier,
                description="Quit application",
                action=self.action_quit,
            ),
            Command(
                key=Qt.Key.Key_E,
                modifiers=NoModifier,
                description="Fullscreen (toggle)",
                action=self.action_fullscreen,
            ),
            Command(
                key=Qt.Key.Key_O,
                modifiers=NoModifier,
                description="Open directory",
                action=self.action_open_directory,
            ),
        ]

    ######################
    # Function Overloads #
    ######################

    def keyPressEvent(self, event: QKeyEvent):
        central = self.centralWidget()
        all_commands = central.commands() + self.commands()
        if not handle_key_event(event, all_commands):
            super().keyPressEvent(event)

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
        self.help_overlay.setParent(new_view)
        self.help_overlay.hide()
        self.help_overlay.raise_()

    def swap_to_wall_view(self, state: ImageState):
        wall_view = WallView(state, self.swap_to_single_view)
        self.swap_view(wall_view)

    def swap_to_single_view(self, state: ImageState):
        single_view = SingleView(state, self.swap_to_wall_view)
        self.swap_view(single_view)

    def choose_directory(self) -> Optional[ImageState]:
        image_dir = QFileDialog.getExistingDirectory(
            parent=self,
            caption="Open Image Directory",
            dir="/Users/audinet/Pictures/Camera/2025 China/Favourites",
        )
        if not image_dir == "":
            return load_image_state(Path(image_dir))

    def load_path(self, path: Path) -> None:
        if path.is_file():
            filedir = path.parent
            image_state = load_image_state(filedir)
            assert not image_state == None
            image_state.image_paths.goto_value(path)
        else:
            image_state = load_image_state(path)
            if image_state == None:
                raise ValueError(f"Directory {path} is empty")
        self.swap_to_single_view(image_state)
