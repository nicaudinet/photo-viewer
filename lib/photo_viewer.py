from typing import Optional
from pathlib import Path

from PySide6.QtWidgets import (
    QMainWindow,
    QWidget,
    QFileDialog,
)
from PySide6.QtGui import QKeyEvent
from PySide6.QtCore import Qt, QTimer

from lib.command import Command, NoModifier, handle_key_event

from lib.state import ImageState, load_image_state
from lib.help_overlay import HelpOverlay
from lib.view.wall_view import WallView
from lib.view.single_view import SingleView
from lib.view.empty_view import EmptyView
from lib.view.loading_view import LoadingView


class PhotoViewer(QMainWindow):

    # How long to keep the (quiet) loading view before falling back to the
    # empty view when nothing was opened. Gives a macOS open event that lands
    # just after launch a chance to arrive without an EmptyView flash first.
    EMPTY_FALLBACK_MS: int = 200

    def __init__(self, filepath: Optional[Path]):

        super().__init__()

        self.setWindowTitle("Photo Viewer")
        self.setGeometry(100, 100, 800, 600)

        self.help_overlay = None

        ###########
        # Widgets #
        ###########

        # The loading view is always the first view: opening from Finder must
        # not flash the empty view first. Its indicator is delayed, so if we
        # snap straight to the empty view (nothing opened) there is no noise.
        loading_view = LoadingView(self)
        self.setCentralWidget(loading_view)

        if filepath:
            self.load_path(filepath)
        else:
            # No path yet — it may still arrive as a macOS open event. Wait
            # briefly, then fall back to the empty view if nothing loaded.
            QTimer.singleShot(
                self.EMPTY_FALLBACK_MS,
                self,
                lambda: self._fallback_to_empty(loading_view),
            )

    def _fallback_to_empty(self, loading_view: QWidget) -> None:
        # Only act if we are still sitting on the original loading view; a
        # load (or a direct view swap) since then must not be clobbered.
        if self.centralWidget() is loading_view:
            self.swap_view(EmptyView(self))

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
        if self.help_overlay:
            self.help_overlay.deleteLater()
            self.help_overlay = None
        else:
            central = self.centralWidget()
            self.help_overlay = HelpOverlay(self.all_commands(), central)
            self.help_overlay.show()
            self.help_overlay.raise_()

    def action_close_help(self):
        if self.help_overlay:
            self.help_overlay.deleteLater()
            self.help_overlay = None

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
                modifiers=None,
                description="Show help (toggle)",
                action=self.action_toggle_help,
            ),
            Command(
                key=Qt.Key.Key_Escape,
                modifiers=None,
                description="Close help",
                action=self.action_close_help,
            ),
            Command(
                key=Qt.Key.Key_Q,
                modifiers=None,
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

    def all_commands(self):
        view_commands = self.centralWidget().commands()  # pyright: ignore
        return view_commands + self.commands()

    ######################
    # Function Overloads #
    ######################

    def keyPressEvent(self, event: QKeyEvent):
        if not handle_key_event(event, self.all_commands()):
            super().keyPressEvent(event)

    def resizeEvent(self, event):
        super().resizeEvent(event)
        if self.help_overlay:
            self.help_overlay.resize_and_center()

    ####################
    # Helper Functions #
    ####################

    def swap_view(self, new_view: QWidget):
        old_view = self.takeCentralWidget()
        self.setCentralWidget(new_view)
        old_view.deleteLater()
        self.help_overlay = None

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
        # Show the loading view immediately, then do the actual (potentially
        # slow) directory scan and decode on the next event-loop tick so the
        # window paints a frame first instead of freezing on launch. Reuse the
        # current loading view when there is one (e.g. straight after launch).
        if not isinstance(self.centralWidget(), LoadingView):
            self.swap_view(LoadingView(self))
        QTimer.singleShot(0, lambda: self._load_path(path))

    def _load_path(self, path: Path) -> None:
        if path.is_file():
            filedir = path.parent
            image_state = load_image_state(filedir)
            assert not image_state == None
            image_state.image_paths.goto_value(path)
        else:
            image_state = load_image_state(path)
            if image_state == None:
                # Nothing to show — fall back to the "nothing open" state
                self.swap_view(EmptyView(self))
                return
        self.swap_to_single_view(image_state)
